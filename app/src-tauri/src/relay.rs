//! The custom relay's health check, behind the Test button in Settings.
//!
//! Three checks, each given [`CHECK_TIMEOUT`]:
//! 1. the relay answers an HTTPS GET with 200;
//! 2. it is an iroh relay: a throwaway endpoint with only this relay in its
//!    map comes online and takes it as its home relay;
//! 3. QUIC address discovery works through it: that endpoint's net report saw
//!    a public address over QUIC.
//!
//! The throwaway endpoint has a random key and no address lookup, so it never
//! publishes anything or touches the running node.

use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMap, RelayMode, RelayUrl, Watcher};
use serde::Serialize;
use tokio::time::timeout;
use tracing::{debug, info};

const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// What the checks found, from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayHealth {
    /// All three passed.
    Good,
    /// An iroh relay, but no public address seen over QUIC: connections may
    /// stay on the relay rather than go direct.
    NoAddressDiscovery,
    /// Not reachable, or not an iroh relay.
    Unreachable,
}

pub async fn check(url: RelayUrl) -> RelayHealth {
    if !reachable(&url).await {
        info!(%url, "relay check: not reachable over HTTPS");
        return RelayHealth::Unreachable;
    }
    let bind = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(RelayMap::from(url.clone())))
        .bind();
    let endpoint = match timeout(CHECK_TIMEOUT, bind).await {
        Ok(Ok(endpoint)) => endpoint,
        Ok(Err(e)) => {
            info!(%url, "relay check: could not bind a test endpoint: {e:#}");
            return RelayHealth::Unreachable;
        }
        Err(_) => return RelayHealth::Unreachable,
    };
    let health = probe(&endpoint, &url).await;
    endpoint.close().await;
    info!(%url, ?health, "relay check done");
    health
}

/// Check 1: an HTTPS GET on the relay URL returns 200.
async fn reachable(url: &RelayUrl) -> bool {
    let Ok(client) = reqwest::Client::builder().timeout(CHECK_TIMEOUT).build() else {
        return false;
    };
    match client.get(url.as_str()).send().await {
        Ok(response) => response.status() == reqwest::StatusCode::OK,
        Err(e) => {
            debug!(%url, "relay check GET failed: {e:#}");
            false
        }
    }
}

/// Checks 2 and 3, on the throwaway endpoint.
async fn probe(endpoint: &Endpoint, url: &RelayUrl) -> RelayHealth {
    if timeout(CHECK_TIMEOUT, endpoint.online()).await.is_err() {
        return RelayHealth::Unreachable;
    }
    let is_home = endpoint
        .home_relay_status()
        .get()
        .iter()
        .any(|status| status.url() == url && status.is_connected());
    if !is_home {
        return RelayHealth::Unreachable;
    }

    // A report is only published once its QUIC probes have finished, so the
    // first one with this relay alone in the map already holds the answer;
    // later ones are waited for in case the first predates the relay.
    let mut reports = endpoint.net_report();
    let discovered = timeout(CHECK_TIMEOUT, async {
        let mut report = reports.get();
        loop {
            if let Some(report) = &report
                && report.has_udp()
                && (report.global_v4.is_some() || report.global_v6.is_some())
            {
                return true;
            }
            report = match reports.updated().await {
                Ok(report) => report,
                Err(_) => return false,
            };
        }
    })
    .await
    .unwrap_or(false);

    if discovered {
        RelayHealth::Good
    } else {
        RelayHealth::NoAddressDiscovery
    }
}
