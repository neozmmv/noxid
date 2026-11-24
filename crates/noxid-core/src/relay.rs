//! A relay of the user's own, used beside n0's rather than instead of them.
//!
//! The endpoint keeps n0's relays and gains this one, so it still picks
//! whichever is best as its home relay, and falls back to this one where n0's
//! are blocked. Our home relay goes out with our address (pkarr and mDNS
//! publish it on every change), and iroh dials a relay it learned from a
//! peer's address whether or not that relay is in its own map: a friend
//! reaches us through ours without configuring it.

use std::sync::{Arc, Mutex};

use iroh::{Endpoint, RelayConfig, RelayUrl};
use tracing::info;

/// Reads a relay URL the user typed. Only `https://` is taken: a relay is
/// reached over TLS, and anything else is a typo or a downgrade.
pub fn parse(text: &str) -> Result<RelayUrl, String> {
    let url: RelayUrl = text
        .trim()
        .parse()
        .map_err(|_| "That isn't a valid URL.".to_string())?;
    if url.scheme() != "https" {
        return Err("Use an https:// URL.".to_string());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("That URL has no host.".to_string());
    }
    Ok(url)
}

/// The relay we added to the endpoint's map, and what it replaced there.
struct Added {
    url: RelayUrl,
    /// The map's own entry for the same URL, put back when ours goes: the user
    /// may have typed one of n0's relays.
    replaced: Option<Arc<RelayConfig>>,
}

/// The custom relay in effect, if any.
#[derive(Default)]
pub(crate) struct CustomRelay {
    /// Held across the map changes, so two changes cannot interleave.
    added: tokio::sync::Mutex<Option<Added>>,
    /// The same URL, readable without waiting, for snapshots.
    url: Mutex<Option<RelayUrl>>,
}

impl CustomRelay {
    pub fn url(&self) -> Option<RelayUrl> {
        self.url.lock().unwrap().clone()
    }

    /// Makes `url` the custom relay, or removes it with `None`. The map change
    /// makes the endpoint run a new net report, which chooses the home relay
    /// again, so nothing needs a restart.
    pub async fn set(&self, endpoint: &Endpoint, url: Option<RelayUrl>) {
        let mut added = self.added.lock().await;
        if added.as_ref().map(|a| &a.url) == url.as_ref() {
            return;
        }
        if let Some(old) = added.take() {
            match old.replaced {
                Some(config) => {
                    endpoint.insert_relay(old.url.clone(), config).await;
                }
                None => {
                    endpoint.remove_relay(&old.url).await;
                }
            }
            info!(url = %old.url, "custom relay removed");
        }
        if let Some(url) = url {
            let config = Arc::new(RelayConfig::from(url.clone()));
            let replaced = endpoint.insert_relay(url.clone(), config).await;
            info!(%url, "custom relay added");
            *added = Some(Added { url, replaced });
        }
        *self.url.lock().unwrap() = added.as_ref().map(|a| a.url.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn takes_an_https_url() {
        let url = parse("  https://relay.example.com/ ").unwrap();
        assert_eq!(url.host_str(), Some("relay.example.com"));
    }

    #[test]
    fn refuses_anything_but_https() {
        assert!(parse("http://relay.example.com").is_err());
        assert!(parse("wss://relay.example.com").is_err());
    }

    #[test]
    fn refuses_what_is_not_a_url() {
        assert!(parse("").is_err());
        assert!(parse("relay.example.com").is_err());
        assert!(parse("https://").is_err());
    }
}
