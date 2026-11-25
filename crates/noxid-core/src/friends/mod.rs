//! Friends: a mutual relationship between two endpoint ids, agreed directly
//! between the two peers on an ALPN of its own, with no server keeping a list.
//!
//! - `wire`: the messages.
//! - `state`: the state machine both peers run.
//! - `store`: the list on disk.
//! - `net`: the ALPN handler, delivery with retries, presence, and rings.
//! - `view`: what a frontend shows.
//!
//! Friendship grants nothing in a room: it does not get anyone past the
//! member gate, and a ring only carries an invite the host chose to send.
//!
//! Nothing is stored anywhere but on the two peers, so a message only arrives
//! while both are online at the same time. Delivery is retried with backoff,
//! and any contact from a peer flushes what we owe it, so two peers that are
//! rarely online together still converge quickly once they are.

mod net;
pub mod state;
pub mod store;
pub mod view;
pub mod wire;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use iroh::{Endpoint, EndpointId, RelayUrl};
use serde::Serialize;
use tokio::sync::{Notify, Semaphore, broadcast, oneshot, watch};

use crate::address_book::{self, AddressBook};
use crate::room::Code;
pub use net::FriendsHandler;
use state::{Book, LocalError, Relation, Time};
use store::Store;
pub use view::{FriendsView, Presence};
pub use wire::Status;

/// How long each step of the protocol may take. Shortened in tests.
#[derive(Debug, Clone)]
pub struct Timing {
    /// One request and its reply, connecting included.
    pub call: Duration,
    /// How long a ring rings before it counts as missed.
    pub ring: Duration,
    /// How often the background loop looks for work.
    pub tick: Duration,
    /// Retry backoff for undelivered messages: doubling from `retry_min`, up
    /// to `retry_max`, with some jitter.
    pub retry_min: Duration,
    pub retry_max: Duration,
    /// How often a reachable friend is probed.
    pub probe_every: Duration,
    /// How long after the last contact a friend still shows as online.
    pub online_for: Duration,
    /// After this long undelivered, a request is only retried by hand.
    pub give_up_after: Duration,
    /// Messages a single peer may send us per minute.
    pub inbound_per_minute: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            call: Duration::from_secs(10),
            ring: Duration::from_secs(20),
            tick: Duration::from_secs(1),
            retry_min: Duration::from_secs(15),
            retry_max: Duration::from_secs(15 * 60),
            probe_every: Duration::from_secs(2 * 60),
            online_for: Duration::from_secs(3 * 60),
            give_up_after: Duration::from_secs(14 * 24 * 60 * 60),
            inbound_per_minute: 10,
        }
    }
}

impl Timing {
    /// For tests on loopback.
    pub fn fast() -> Self {
        Self {
            call: Duration::from_secs(2),
            ring: Duration::from_secs(1),
            tick: Duration::from_millis(50),
            retry_min: Duration::from_millis(100),
            retry_max: Duration::from_millis(400),
            probe_every: Duration::from_millis(300),
            online_for: Duration::from_secs(2),
            give_up_after: Duration::from_secs(24 * 60 * 60),
            inbound_per_minute: 10_000,
        }
    }
}

pub struct FriendsConfig {
    pub endpoint: Endpoint,
    /// Where the list lives; `friends.json` in the state directory.
    pub store_path: PathBuf,
    /// Our display name, sent with requests and pings.
    pub name: watch::Receiver<String>,
    /// Our profile picture's hash, sent with pings and pongs.
    pub avatar: watch::Receiver<Option<String>>,
    /// The status we chose to show, sent with pings and pongs.
    pub status: watch::Receiver<Status>,
    /// Our home relay, sent with requests, accepts, pings and their replies
    /// so friends can reach us without a lookup.
    pub relay: watch::Receiver<Option<RelayUrl>>,
    /// Where friends' relays go so dials by id find them: the address book
    /// registered on `endpoint`.
    pub address_book: AddressBook,
    pub timing: Timing,
}

/// How a ring ended, from the caller's side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "message", rename_all = "snake_case")]
pub enum RingOutcome {
    Answered,
    Declined,
    NoAnswer,
    /// We hung up first.
    HungUp,
    /// They are already being rung, or too many rings at once.
    Busy,
    /// They do not count us as a friend.
    NotFriends,
    /// We could not reach them.
    Unreachable(String),
}

/// Something worth a one-off notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FriendEvent {
    RequestReceived { id: String, name: String },
    FriendAdded { id: String, name: String },
    RequestDeclined { id: String, name: String },
    Ringing { id: String, name: String },
    MissedCall { id: String, name: String },
}

/// Who a pasted Noxid ID names, and where to reach them if it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendId {
    pub id: EndpointId,
    /// Their home relay when they shared the code; only ever https.
    pub relay: Option<RelayUrl>,
}

impl From<EndpointId> for FriendId {
    fn from(id: EndpointId) -> Self {
        Self { id, relay: None }
    }
}

/// Reads a Noxid ID someone pasted: an endpoint id (hex or base32), or a
/// friend code. A friend code is the same code as a room code: the id and,
/// in the current format, the home relay. Our own id, and room invites, are
/// refused with a reason.
pub fn parse_friend_id(text: &str, own: EndpointId) -> Result<FriendId> {
    let text = text.trim();
    if text.is_empty() {
        bail!("paste a Noxid ID");
    }
    let parsed = match text.parse::<EndpointId>() {
        Ok(id) => FriendId { id, relay: None },
        Err(_) => match Code::decode(text) {
            Ok(Code::Knock { coordinator, relay }) => FriendId {
                id: coordinator,
                relay: relay.filter(|r| r.scheme() == "https"),
            },
            Ok(Code::Invite { .. }) => bail!("that's a room invite, not a Noxid ID"),
            Err(e) if e.to_string().contains("update Noxid") => bail!("{e}"),
            Err(_) => bail!("that's not a Noxid ID"),
        },
    };
    if parsed.id == own {
        bail!("that's your own ID");
    }
    Ok(parsed)
}

/// The code to share so others can add us: our id and current home relay.
pub fn friend_code(own: &EndpointId, relay: Option<&RelayUrl>) -> String {
    crate::room::room_code(own, relay)
}

pub(crate) fn now_ms() -> Time {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as Time)
        .unwrap_or(0)
}

/// How things stand with one peer on the network, in memory only.
#[derive(Debug, Default)]
pub(crate) struct Link {
    /// Last time we reached them or they reached us, and last failure.
    pub last_ok: Option<Time>,
    pub last_fail: Option<Time>,
    pub unaware: bool,
    /// Delivery of what we owe them.
    pub send_attempts: u32,
    pub next_send: Option<tokio::time::Instant>,
    pub sending: bool,
    pub last_error: Option<String>,
    /// Retry even a request we had given up on.
    pub forced: bool,
    /// The status they last told us they chose.
    pub status: Status,
    /// Presence probes.
    pub probe_attempts: u32,
    pub next_probe: Option<tokio::time::Instant>,
    pub probing: bool,
}

pub(crate) struct IncomingRing {
    pub call_id: wire::RequestId,
    pub code: String,
    pub room_name: Option<String>,
    pub expires_at: Time,
    pub answer: oneshot::Sender<bool>,
}

pub(crate) struct Calling {
    pub to: EndpointId,
    pub since: Time,
    pub hang_up: Option<oneshot::Sender<()>>,
}

/// Missed calls kept for the list.
const MAX_MISSED: usize = 20;

pub(crate) struct Shared {
    pub endpoint: Endpoint,
    pub timing: Timing,
    pub name: watch::Receiver<String>,
    pub avatar: watch::Receiver<Option<String>>,
    pub status: watch::Receiver<Status>,
    pub relay: watch::Receiver<Option<RelayUrl>>,
    pub address_book: AddressBook,
    pub book: Mutex<Book>,
    pub store: Store,
    pub problem: Option<String>,
    /// Serializes writes to disk; each write takes the latest book.
    pub save_lock: tokio::sync::Mutex<()>,
    pub links: Mutex<HashMap<EndpointId, Link>>,
    pub rings: Mutex<HashMap<EndpointId, IncomingRing>>,
    pub calling: Mutex<Option<Calling>>,
    pub missed: Mutex<Vec<(EndpointId, Time)>>,
    /// Inbound messages per peer: tokens left and when they were counted.
    pub rate: Mutex<HashMap<EndpointId, (f64, tokio::time::Instant)>>,
    pub inbound: Semaphore,
    pub dials: Semaphore,
    pub changed: watch::Sender<u64>,
    pub events: broadcast::Sender<FriendEvent>,
    pub wake: Notify,
}

/// The friends list and everything that keeps it in step with peers. Cheap
/// to clone.
#[derive(Clone)]
pub struct Friends {
    shared: Arc<Shared>,
}

impl Friends {
    /// Opens the list and starts delivery and presence in the background.
    /// Needs a tokio runtime.
    pub fn spawn(config: FriendsConfig) -> Self {
        let own = config.endpoint.id();
        let opened = Store::open(config.store_path, own);
        let mut book = opened.book;
        book.prune(now_ms());
        // Before anything is dialed, so friends are reached where they were
        // last, without a lookup.
        for (peer, relay) in book.relays() {
            config.address_book.insert(peer, relay, address_book::Source::Saved);
        }
        let shared = Arc::new(Shared {
            endpoint: config.endpoint,
            timing: config.timing,
            name: config.name,
            avatar: config.avatar,
            status: config.status,
            relay: config.relay,
            address_book: config.address_book,
            book: Mutex::new(book),
            store: opened.store,
            problem: opened.problem,
            save_lock: tokio::sync::Mutex::new(()),
            links: Mutex::new(HashMap::new()),
            rings: Mutex::new(HashMap::new()),
            calling: Mutex::new(None),
            missed: Mutex::new(Vec::new()),
            rate: Mutex::new(HashMap::new()),
            inbound: Semaphore::new(net::MAX_INBOUND),
            dials: Semaphore::new(net::MAX_DIALS),
            changed: watch::channel(0).0,
            events: broadcast::channel(32).0,
            wake: Notify::new(),
        });
        tokio::spawn(net::run(Arc::downgrade(&shared)));
        Self { shared }
    }

    /// The handler to register on the router for [`wire::FRIENDS_ALPN`].
    pub fn handler(&self) -> FriendsHandler {
        FriendsHandler {
            shared: self.shared.clone(),
        }
    }

    /// Ticks whenever [`view`](Self::view) would change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.shared.changed.subscribe()
    }

    pub fn events(&self) -> broadcast::Receiver<FriendEvent> {
        self.shared.events.subscribe()
    }

    pub fn is_friend(&self, peer: &EndpointId) -> bool {
        self.shared.book.lock().unwrap().is_friend(peer)
    }

    pub fn relation(&self, peer: &EndpointId) -> Option<Relation> {
        self.shared.book.lock().unwrap().relation(peer).cloned()
    }

    /// Everything we keep about `peer`.
    pub fn record(&self, peer: &EndpointId) -> Option<state::PeerRecord> {
        self.shared.book.lock().unwrap().record(peer).cloned()
    }

    /// The name to show for `peer`: our alias, their name, or their short id.
    pub fn display_name(&self, peer: &EndpointId) -> String {
        self.shared.display_name(peer)
    }

    pub fn view(&self) -> FriendsView {
        self.shared.view()
    }

    /// Friends whose pictures we know of, with each picture's hash.
    pub fn avatars(&self) -> Vec<(EndpointId, String)> {
        let book = self.shared.book.lock().unwrap();
        book.records()
            .iter()
            .filter(|(_, r)| matches!(r.relation, Relation::Friends { .. }))
            .filter_map(|(peer, r)| Some((*peer, r.avatar.clone()?)))
            .collect()
    }

    /// Pings every friend now rather than at its next turn, so a change to
    /// what we send them (our status) reaches them at once.
    pub fn announce(&self) {
        for link in self.shared.links.lock().unwrap().values_mut() {
            link.next_probe = None;
        }
        self.shared.wake.notify_one();
    }

    /// Tells the frontend the view changed: a friend's picture arrived.
    pub fn refresh(&self) {
        self.shared.bump();
    }

    /// Asks `peer.id` to be friends, keeping the relay its code named so
    /// the request goes straight there.
    pub async fn send_request(&self, peer: impl Into<FriendId>) -> Result<()> {
        let FriendId { id, relay } = peer.into();
        if let Some(relay) = &relay {
            self.shared
                .address_book
                .insert(id, relay.clone(), address_book::Source::Code);
        }
        let relay = relay.map(|r| r.to_string());
        let named_relay = relay.is_some();
        let result = self
            .shared
            .local(id, |book, now| {
                let result = book.request(id, now);
                // Kept even when the request is refused because they are a
                // friend already: pasting a friend's fresh code is how a
                // relay that changed while neither of us could tell the
                // other is learned again.
                book.set_relay(&id, relay.as_deref());
                result
            })
            .await;
        if result.is_err() && named_relay && self.shared.problem.is_none() {
            // `local` stores only what succeeded; the relay is kept anyway.
            if let Err(e) = self.shared.persist().await {
                tracing::warn!("could not save the friends list: {e:#}");
            }
        }
        result
    }

    /// Accepts or declines `peer`'s request.
    pub async fn answer(&self, peer: EndpointId, accept: bool) -> Result<()> {
        if accept {
            self.shared.local(peer, |book, now| book.accept(&peer, now)).await
        } else {
            self.shared.local(peer, |book, now| book.decline(&peer, now)).await
        }
    }

    pub async fn cancel(&self, peer: EndpointId) -> Result<()> {
        self.shared.local(peer, |book, now| book.cancel(&peer, now)).await
    }

    pub async fn remove(&self, peer: EndpointId) -> Result<()> {
        self.shared.local(peer, |book, now| book.remove(&peer, now)).await
    }

    pub async fn set_alias(&self, peer: EndpointId, alias: Option<String>) -> Result<()> {
        self.shared
            .local(peer, |book, _| book.set_alias(&peer, alias))
            .await
    }

    /// Tries to deliver what `peer` is owed now, even a request given up on.
    pub fn retry(&self, peer: EndpointId) {
        {
            let mut links = self.shared.links.lock().unwrap();
            let link = links.entry(peer).or_default();
            link.forced = true;
            link.send_attempts = 0;
            link.next_send = None;
        }
        self.shared.wake.notify_one();
    }

    /// Rings a friend into our room with `code`, a single-use invite to it,
    /// and waits for the outcome.
    pub async fn ring(&self, peer: EndpointId, code: String, room_name: Option<String>) -> Result<RingOutcome> {
        net::ring(&self.shared, peer, code, room_name).await
    }

    /// Stops ringing whoever we are ringing.
    pub fn hang_up(&self) {
        if let Some(calling) = self.shared.calling.lock().unwrap().as_mut()
            && let Some(hang_up) = calling.hang_up.take()
        {
            let _ = hang_up.send(());
        }
    }

    /// Answers `peer`'s ring. On yes, returns the invite to join with.
    pub fn answer_ring(&self, peer: EndpointId, accept: bool) -> Result<Option<String>> {
        let ring = self.shared.rings.lock().unwrap().remove(&peer);
        let Some(ring) = ring else {
            bail!("they are not ringing any more");
        };
        self.shared.bump();
        if ring.answer.send(accept).is_err() {
            bail!("they hung up");
        }
        Ok(accept.then_some(ring.code))
    }

    pub fn dismiss_missed(&self) {
        self.shared.missed.lock().unwrap().clear();
        self.shared.bump();
    }
}

impl Shared {
    pub fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn display_name(&self, peer: &EndpointId) -> String {
        let book = self.book.lock().unwrap();
        let record = book.record(peer);
        record
            .and_then(|r| r.alias.clone())
            .or_else(|| record.map(|r| r.name.clone()).filter(|n| !n.is_empty()))
            .unwrap_or_else(|| crate::room::short_id(&peer.to_string()))
    }

    /// Writes the latest book to disk.
    pub async fn persist(&self) -> Result<()> {
        let _guard = self.save_lock.lock().await;
        let book = self.book.lock().unwrap().clone();
        self.store.save(&book)
    }

    /// Runs one of our own actions on the book, stores it, and sends what it
    /// owes the peer straight away.
    async fn local(
        &self,
        peer: EndpointId,
        action: impl FnOnce(&mut Book, Time) -> std::result::Result<(), LocalError>,
    ) -> Result<()> {
        if let Some(problem) = &self.problem {
            bail!("{problem}");
        }
        let (before, after) = {
            let mut book = self.book.lock().unwrap();
            let before = book.relation(&peer).cloned();
            action(&mut book, now_ms())?;
            (before, book.relation(&peer).cloned())
        };
        self.persist().await?;
        self.notify_transition(peer, before.as_ref(), after.as_ref(), false);
        {
            let mut links = self.links.lock().unwrap();
            let link = links.entry(peer).or_default();
            link.send_attempts = 0;
            link.next_send = None;
            link.last_error = None;
        }
        self.bump();
        self.wake.notify_one();
        Ok(())
    }

    /// Raises the events a change in our relation with `peer` deserves.
    /// `remote` says whether the peer caused it.
    pub fn notify_transition(&self, peer: EndpointId, before: Option<&Relation>, after: Option<&Relation>, remote: bool) {
        if before == after || !remote {
            return;
        }
        let id = peer.to_string();
        let name = self.display_name(&peer);
        let event = match after {
            Some(Relation::Incoming { .. }) if !matches!(before, Some(Relation::Incoming { .. })) => {
                FriendEvent::RequestReceived { id, name }
            }
            Some(Relation::Friends { .. }) if !matches!(before, Some(Relation::Friends { .. })) => {
                FriendEvent::FriendAdded { id, name }
            }
            Some(Relation::Closed {
                reason: wire::CloseReason::Declined,
                by: state::Side::Them,
                ..
            }) => FriendEvent::RequestDeclined { id, name },
            _ => return,
        };
        let _ = self.events.send(event);
    }

    pub fn missed_call(&self, peer: EndpointId) {
        {
            let mut missed = self.missed.lock().unwrap();
            missed.retain(|(id, _)| *id != peer);
            missed.insert(0, (peer, now_ms()));
            missed.truncate(MAX_MISSED);
        }
        let _ = self.events.send(FriendEvent::MissedCall {
            id: peer.to_string(),
            name: self.display_name(&peer),
        });
        self.bump();
    }

    fn presence(&self, link: Option<&Link>, now: Time) -> (Presence, Option<Time>) {
        let Some(link) = link else {
            return (Presence::Unknown, None);
        };
        let fresh = self.timing.online_for.as_millis() as Time;
        let presence = match (link.last_ok, link.last_fail) {
            (Some(ok), fail) if fail.is_none_or(|f| f <= ok) && now - ok <= fresh => Presence::Online,
            (_, Some(_)) => Presence::Offline,
            _ => Presence::Unknown,
        };
        (presence, link.last_ok)
    }

    pub fn view(&self) -> FriendsView {
        use view::*;
        let now = now_ms();
        let give_up = self.timing.give_up_after.as_millis() as Time;
        let book = self.book.lock().unwrap();
        let links = self.links.lock().unwrap();
        let name_of = |peer: &EndpointId| {
            let record = book.record(peer);
            record
                .and_then(|r| r.alias.clone())
                .or_else(|| record.map(|r| r.name.clone()).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| crate::room::short_id(&peer.to_string()))
        };

        let mut friends = Vec::new();
        let mut incoming = Vec::new();
        let mut outgoing = Vec::new();
        for (peer, record) in book.records() {
            let link = links.get(peer);
            match &record.relation {
                Relation::Friends { .. } | Relation::Accepting { .. } => {
                    let finishing = matches!(record.relation, Relation::Accepting { .. });
                    let status = link.map(|l| l.status).unwrap_or_default();
                    let (presence, last_seen) = if finishing {
                        (Presence::Unknown, link.and_then(|l| l.last_ok))
                    } else if status == Status::Invisible {
                        // Shown as they asked: offline, and not just seen.
                        match self.presence(link, now) {
                            (Presence::Online, _) => (Presence::Offline, None),
                            other => other,
                        }
                    } else {
                        self.presence(link, now)
                    };
                    let status = match status {
                        Status::Invisible => Status::Online,
                        other => other,
                    };
                    friends.push(FriendView {
                        id: peer.to_string(),
                        name: name_of(peer),
                        avatar: record.avatar.clone(),
                        status,
                        alias: record.alias.clone(),
                        presence,
                        last_seen,
                        finishing,
                        unaware: link.is_some_and(|l| l.unaware),
                    });
                }
                Relation::Incoming { received_at, .. } => incoming.push(IncomingView {
                    id: peer.to_string(),
                    name: name_of(peer),
                    received_at: *received_at,
                }),
                Relation::Outgoing {
                    created_at,
                    delivered_at,
                    ..
                } => outgoing.push(OutgoingView {
                    id: peer.to_string(),
                    name: name_of(peer),
                    created_at: *created_at,
                    delivered_at: *delivered_at,
                    stalled: delivered_at.is_none()
                        && now - created_at > give_up
                        && !link.is_some_and(|l| l.forced),
                    last_error: link.and_then(|l| l.last_error.clone()),
                }),
                Relation::Closed { .. } => {}
            }
        }
        friends.sort_by_key(|f| f.name.to_lowercase());
        incoming.sort_by_key(|r| std::cmp::Reverse(r.received_at));
        outgoing.sort_by_key(|r| std::cmp::Reverse(r.created_at));

        let rings = self
            .rings
            .lock()
            .unwrap()
            .iter()
            .map(|(peer, ring)| RingView {
                id: peer.to_string(),
                name: name_of(peer),
                room_name: ring.room_name.clone(),
                expires_at: ring.expires_at,
            })
            .collect();
        let calling = self.calling.lock().unwrap().as_ref().map(|c| CallingView {
            id: c.to.to_string(),
            name: name_of(&c.to),
            since: c.since,
        });
        let missed = self
            .missed
            .lock()
            .unwrap()
            .iter()
            .map(|(peer, at)| MissedView {
                id: peer.to_string(),
                name: name_of(peer),
                at: *at,
            })
            .collect();

        FriendsView {
            friends,
            incoming,
            outgoing,
            rings,
            calling,
            missed,
            problem: self.problem.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::{Code, N0_RELAYS, Room, room_code};

    fn key(n: u8) -> EndpointId {
        iroh::SecretKey::from_bytes(&[n; 32]).public()
    }

    fn relay(url: &str) -> RelayUrl {
        url.parse().unwrap()
    }

    #[test]
    fn a_raw_id_has_no_relay() {
        for text in [key(2).to_string(), format!("  {}  ", key(2))] {
            assert_eq!(parse_friend_id(&text, key(1)).unwrap(), FriendId::from(key(2)));
        }
    }

    #[test]
    fn an_old_room_code_still_names_the_id() {
        let old = bs58::encode(key(2).as_bytes()).into_string();
        assert_eq!(parse_friend_id(&old, key(1)).unwrap(), FriendId::from(key(2)));
    }

    #[test]
    fn a_new_code_names_the_relay_too() {
        for url in [N0_RELAYS[2], "https://relay.neozmmv.dev"] {
            let code = friend_code(&key(2), Some(&relay(url)));
            let parsed = parse_friend_id(&code, key(1)).unwrap();
            assert_eq!(parsed.id, key(2));
            assert_eq!(parsed.relay, Some(relay(url)));
        }
    }

    #[test]
    fn a_relay_that_is_not_https_is_dropped() {
        let code = room_code(&key(2), Some(&relay("http://10.0.0.1")));
        assert_eq!(parse_friend_id(&code, key(1)).unwrap().relay, None);
    }

    #[test]
    fn room_invites_and_our_own_id_are_refused() {
        let mut room = Room::default();
        room.create(None, &key(2).to_string(), "host");
        let (invite, _) = room
            .generate_invite(key(2), Some(&relay(N0_RELAYS[0])), chrono::Duration::minutes(1), chrono::Utc::now())
            .unwrap();
        let err = parse_friend_id(&invite, key(1)).unwrap_err().to_string();
        assert!(err.contains("invite"), "{err}");

        for own in [key(1).to_string(), friend_code(&key(1), Some(&relay(N0_RELAYS[0])))] {
            let err = parse_friend_id(&own, key(1)).unwrap_err().to_string();
            assert!(err.contains("own ID"), "{err}");
        }
        assert!(parse_friend_id("hello", key(1)).is_err());
        assert!(parse_friend_id("", key(1)).is_err());
    }

    #[test]
    fn a_friend_code_is_a_room_code() {
        let url = relay(N0_RELAYS[1]);
        assert_eq!(friend_code(&key(2), Some(&url)), room_code(&key(2), Some(&url)));
        assert!(matches!(
            Code::decode(&friend_code(&key(2), None)).unwrap(),
            Code::Knock { relay: None, .. }
        ));
    }
}
