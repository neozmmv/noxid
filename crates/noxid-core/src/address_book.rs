//! The peers we already know how to reach, kept where iroh looks first.
//!
//! Every dial in Noxid is by endpoint id alone (admin calls, the mesh, avatars,
//! files), which leaves iroh to look up the peer's home relay through pkarr,
//! DNS or mDNS. Where those are filtered, dialing fails even though the relay
//! itself is reachable. Yet we usually know the relay already: a code carries
//! the host's, and the roster carries every member's. Putting them here, an
//! address lookup on the endpoint itself, lets every `connect(id)` resolve
//! without the network, iroh-live's mesh included, with no call site changed.
//! The other lookups stay on the endpoint as the fallback.
//!
//! Only home relays are kept, never direct addresses: those are found by
//! hole-punching once the relay connects us.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iroh::address_lookup::{AddressLookup, Error, Item, MemoryLookup};
use iroh::{EndpointAddr, EndpointId, RelayUrl};
use n0_future::boxed::BoxStream;
use tracing::debug;

/// Where an address came from, for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The room code or invite we joined with.
    Code,
    /// A roster the host sent us, or our own as host.
    Roster,
    /// A join, knock or address update the member sent us as host.
    Member,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Source::Code => "code",
            Source::Roster => "roster",
            Source::Member => "member",
        })
    }
}

/// An address lookup answering from what we were told. Cheap to clone; every
/// clone is the same book.
#[derive(Debug, Clone)]
pub(crate) struct AddressBook {
    own: EndpointId,
    memory: MemoryLookup,
    /// Each entry's relay and where it came from: the log line for a dial, and
    /// how an unchanged entry is told apart from a new one.
    known: Arc<Mutex<HashMap<EndpointId, (RelayUrl, Source)>>>,
}

/// The provenance iroh records for addresses from the book.
const PROVENANCE: &str = "noxid_address_book";

impl AddressBook {
    pub fn new(own: EndpointId) -> Self {
        Self {
            own,
            memory: MemoryLookup::with_provenance(PROVENANCE),
            known: Arc::default(),
        }
    }

    /// Records `id`'s home relay, replacing whatever we had for it. Unchanged
    /// entries are skipped quietly: the roster arrives again every few seconds.
    pub fn insert(&self, id: EndpointId, relay: RelayUrl, source: Source) {
        if id == self.own {
            return;
        }
        let mut known = self.known.lock().unwrap();
        if known.get(&id).is_some_and(|(old, _)| *old == relay) {
            return;
        }
        match known.insert(id, (relay.clone(), source)) {
            Some((old, _)) => {
                debug!(peer = %id.fmt_short(), %relay, %old, %source, "address book: home relay changed")
            }
            None => {
                debug!(peer = %id.fmt_short(), %relay, %source, "address book: learned home relay")
            }
        }
        self.memory
            .set_endpoint_info(EndpointAddr::new(id).with_relay_url(relay));
    }

    /// The relay we hold for `id`, if any.
    #[cfg(test)]
    pub fn relay(&self, id: &EndpointId) -> Option<RelayUrl> {
        self.known
            .lock()
            .unwrap()
            .get(id)
            .map(|(relay, _)| relay.clone())
    }
}

impl AddressLookup for AddressBook {
    fn resolve(&self, id: EndpointId) -> Option<BoxStream<Result<Item, Error>>> {
        match self.known.lock().unwrap().get(&id) {
            Some((relay, source)) => {
                debug!(peer = %id.fmt_short(), %relay, %source, "dialing with the address book's relay");
            }
            None => {
                debug!(peer = %id.fmt_short(), "not in the address book; left to pkarr, DNS and mDNS lookup");
            }
        }
        self.memory.resolve(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    fn url(s: &str) -> RelayUrl {
        s.parse().unwrap()
    }

    #[test]
    fn answers_with_the_latest_relay() {
        let book = AddressBook::new(key(1));
        book.insert(key(2), url("https://a.example.com"), Source::Code);
        book.insert(key(2), url("https://b.example.com"), Source::Roster);

        assert_eq!(book.relay(&key(2)), Some(url("https://b.example.com")));
        let stored = book.memory.get_endpoint_info(key(2)).unwrap();
        let relays: Vec<_> = stored.relay_urls().cloned().collect();
        assert_eq!(
            relays,
            vec![url("https://b.example.com")],
            "replaced, not added to"
        );
    }

    #[test]
    fn never_lists_ourselves() {
        let book = AddressBook::new(key(1));
        book.insert(key(1), url("https://a.example.com"), Source::Roster);
        assert!(book.relay(&key(1)).is_none());
        assert!(book.memory.get_endpoint_info(key(1)).is_none());
    }
}
