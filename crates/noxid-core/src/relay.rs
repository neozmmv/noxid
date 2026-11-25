//! A relay of the user's own, used beside n0's, or instead of them.
//!
//! By default the endpoint keeps n0's relays and gains this one, so it still
//! picks whichever is best as its home relay, and falls back to this one where
//! n0's are blocked. Our home relay goes out with our address (codes, the
//! roster, friends' messages, pkarr and mDNS), and iroh dials a relay it
//! learned from a peer's address whether or not that relay is in its own map:
//! a friend reaches us through ours without configuring it.
//!
//! With "only this relay" on, n0's relays leave the map, so the home relay is
//! always this one and every address we hand out names it. That is for
//! talking to people whose network blocks n0's relays: homed on n0, we would
//! hand them a relay they cannot reach.
//!
//! How iroh 1.4 takes a map change at runtime: every insert or removal starts
//! a full net report, which only weighs relays now in the map, so the
//! endpoint re-homes as soon as that report finishes, within seconds. The
//! exception is a report that reaches no relay at all: it keeps the home
//! relay it had, even one that has just left the map. That is why the rest of
//! Noxid asks [`CustomRelay::is_usable_home`] before handing a home relay out.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use iroh::{Endpoint, RelayConfig, RelayMap, RelayUrl};
use tracing::{debug, info};

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

/// The relays the map should hold: the defaults and the custom one, or the
/// custom one alone when `only` is on. `only` without a custom relay counts
/// for nothing; the map is never emptied. Always a fresh map: a `RelayMap`
/// shares its contents with its clones.
pub fn desired_map(defaults: &RelayMap, custom: Option<&RelayUrl>, only: bool) -> RelayMap {
    let map = RelayMap::empty();
    if !(only && custom.is_some()) {
        map.extend(defaults);
    }
    if let Some(url) = custom {
        // A default relay typed in as the custom one keeps its own config,
        // which knows its QUIC port.
        let config = defaults
            .get(url)
            .unwrap_or_else(|| Arc::new(RelayConfig::from(url.clone())));
        map.insert(url.clone(), config);
    }
    map
}

/// What the user chose.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Choice {
    url: Option<RelayUrl>,
    only: bool,
}

/// The custom relay in effect, if any, and whether it is the only one.
pub struct CustomRelay {
    /// n0's relays, as the preset puts them in the map.
    defaults: RelayMap,
    /// What the endpoint's map holds now. Held across the map changes, so
    /// two changes cannot interleave.
    applied: tokio::sync::Mutex<RelayMap>,
    /// The choice in effect, readable without waiting, for snapshots.
    choice: Mutex<Choice>,
}

impl CustomRelay {
    /// Describes the map the endpoint is built with,
    /// [`initial_map`](Self::initial_map). `only` without a relay is off.
    pub fn new(defaults: RelayMap, url: Option<RelayUrl>, only: bool) -> Self {
        let choice = Choice {
            only: only && url.is_some(),
            url,
        };
        let applied = desired_map(&defaults, choice.url.as_ref(), choice.only);
        Self {
            defaults,
            applied: tokio::sync::Mutex::new(applied),
            choice: Mutex::new(choice),
        }
    }

    /// The map to build the endpoint with, so its first home relay is chosen
    /// from it: with "only this relay" on, no n0 relay is ever our home, even
    /// briefly, and none ends up in a code.
    pub fn initial_map(&self) -> RelayMap {
        let choice = self.choice.lock().unwrap().clone();
        desired_map(&self.defaults, choice.url.as_ref(), choice.only)
    }

    pub fn url(&self) -> Option<RelayUrl> {
        self.choice.lock().unwrap().url.clone()
    }

    /// Whether the custom relay is the only one.
    pub fn only(&self) -> bool {
        self.choice.lock().unwrap().only
    }

    /// Whether `home` may be handed out as our home relay. With "only this
    /// relay" on, only the custom one is: anything else is an n0 relay the
    /// endpoint kept because the custom one could not be reached.
    pub fn is_usable_home(&self, home: &RelayUrl) -> bool {
        let choice = self.choice.lock().unwrap();
        !choice.only || choice.url.as_ref() == Some(home)
    }

    /// The relays the endpoint's map holds now.
    pub async fn applied(&self) -> BTreeSet<RelayUrl> {
        self.applied.lock().await.urls()
    }

    /// Makes `url` the custom relay, or removes it with `None`, leaving
    /// "only this relay" as it was. Removing the relay turns that off, which
    /// puts n0's relays back.
    pub async fn set(&self, endpoint: &Endpoint, url: Option<RelayUrl>) {
        let only = self.only() && url.is_some();
        self.choose(endpoint, Choice { url, only }).await;
    }

    /// Turns "only this relay" on or off. Refused without a custom relay.
    pub async fn set_only(&self, endpoint: &Endpoint, only: bool) -> Result<(), String> {
        let url = self.url();
        if only && url.is_none() {
            return Err("Set a custom relay first.".to_string());
        }
        self.choose(endpoint, Choice { url, only }).await;
        Ok(())
    }

    /// Brings the endpoint's map in line with `choice`. Additions go first,
    /// so the map is never empty on the way.
    async fn choose(&self, endpoint: &Endpoint, choice: Choice) {
        let mut applied = self.applied.lock().await;
        let before = self.choice.lock().unwrap().clone();
        let want = desired_map(&self.defaults, choice.url.as_ref(), choice.only);
        let have: BTreeSet<RelayUrl> = applied.urls();
        let wanted: BTreeSet<RelayUrl> = want.urls();
        for url in &wanted {
            let config = want.get(url).expect("listed");
            // A changed config for the same URL (the custom relay turning
            // out to be one of n0's) is put in too.
            if applied.get(url).is_none_or(|c| *c != *config) {
                endpoint.insert_relay(url.clone(), config).await;
            }
        }
        for url in have.difference(&wanted) {
            endpoint.remove_relay(url).await;
        }
        *applied = want;
        *self.choice.lock().unwrap() = choice.clone();
        drop(applied);

        if before.url != choice.url {
            match &choice.url {
                Some(url) => info!(%url, "custom relay set"),
                None => info!("custom relay removed"),
            }
        }
        if before.only != choice.only {
            debug!(
                only = choice.only,
                relays = ?wanted.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "relay mode switched; the endpoint re-homes after its next net report"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn n0() -> RelayMap {
        iroh::defaults::prod::default_relay_map()
    }

    fn custom() -> RelayUrl {
        "https://relay.example.com".parse().unwrap()
    }

    fn urls(map: &RelayMap) -> BTreeSet<RelayUrl> {
        map.urls()
    }

    #[test]
    fn off_keeps_n0_beside_the_custom_relay() {
        let map = desired_map(&n0(), Some(&custom()), false);
        let mut expected = urls(&n0());
        expected.insert(custom());
        assert_eq!(urls(&map), expected);
    }

    #[test]
    fn on_leaves_only_the_custom_relay() {
        let map = desired_map(&n0(), Some(&custom()), true);
        assert_eq!(urls(&map), BTreeSet::from([custom()]));
    }

    #[test]
    fn on_without_a_custom_relay_keeps_n0() {
        assert_eq!(urls(&desired_map(&n0(), None, true)), urls(&n0()));
        assert_eq!(urls(&desired_map(&n0(), None, false)), urls(&n0()));
    }

    #[test]
    fn an_n0_relay_as_the_custom_one_keeps_its_config() {
        let defaults = n0();
        let first = defaults.urls::<Vec<RelayUrl>>().remove(0);
        let map = desired_map(&defaults, Some(&first), true);
        assert!(Arc::ptr_eq(&map.get(&first).unwrap(), &defaults.get(&first).unwrap()));
    }

    #[test]
    fn maps_are_fresh_and_never_share_contents() {
        let defaults = n0();
        let map = desired_map(&defaults, None, false);
        map.insert(custom(), Arc::new(RelayConfig::from(custom())));
        assert!(!defaults.contains(&custom()), "the defaults are untouched");
    }

    #[test]
    fn the_endpoint_starts_with_the_map_chosen() {
        let on = CustomRelay::new(n0(), Some(custom()), true);
        assert_eq!(urls(&on.initial_map()), BTreeSet::from([custom()]));
        assert!(on.only());

        let off = CustomRelay::new(n0(), Some(custom()), false);
        assert_eq!(off.initial_map().len(), n0().len() + 1);

        let no_relay = CustomRelay::new(n0(), None, true);
        assert!(!no_relay.only(), "on needs a relay");
        assert_eq!(urls(&no_relay.initial_map()), urls(&n0()));
    }

    #[test]
    fn only_the_custom_relay_is_a_usable_home_when_on() {
        let n0_relay = n0().urls::<Vec<RelayUrl>>().remove(0);
        let on = CustomRelay::new(n0(), Some(custom()), true);
        assert!(on.is_usable_home(&custom()));
        assert!(!on.is_usable_home(&n0_relay), "a stale n0 home is not handed out");

        let off = CustomRelay::new(n0(), Some(custom()), false);
        assert!(off.is_usable_home(&n0_relay));
    }
}
