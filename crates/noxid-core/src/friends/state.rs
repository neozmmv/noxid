//! The friendship state machine: what we believe about each peer, and how
//! that changes on our own actions, on their messages, and on their replies
//! to ours. No I/O and no clock: `now` (unix milliseconds) is passed in, so
//! every transition is a plain function of its inputs and is tested as one.
//!
//! The rules that keep two peers' books in step:
//!
//! - The sender of a message is the connection's authenticated id, never a
//!   field in it. A peer can only ever speak for itself.
//! - Every message names a request id, and only acts on a relation holding
//!   that exact id. A new request always has a fresh random id, so nothing
//!   stale or replayed can touch a later relationship.
//! - The requester becomes a friend when it stores the recipient's valid
//!   Accept; the recipient when the requester's Ack comes back. Until then the
//!   recipient is `Accepting`, which counts as not a friend. Each side stores
//!   its new state before it replies, so a lost reply only ever looks like a
//!   message to retry.
//! - Two requests that cross are mutual consent; both sides settle on the
//!   lower of the two ids, so even a true race ends on one friendship.

use std::collections::{BTreeMap, HashMap};

use iroh::EndpointId;
use serde::{Deserialize, Serialize};

use super::wire::{CloseReason, FriendMsg, FriendReply, RequestId};

/// Unix time in milliseconds.
pub type Time = i64;

const HOUR: Time = 60 * 60 * 1000;
const DAY: Time = 24 * HOUR;

/// After we decline someone, their new requests are refused this long.
pub const DECLINE_COOLDOWN: Time = DAY;
/// How many new request ids one peer may send us in `REQUEST_WINDOW`.
pub const MAX_NEW_REQUESTS: usize = 3;
pub const REQUEST_WINDOW: Time = DAY;
/// Requests waiting for our answer, from everyone together.
pub const MAX_PENDING_INCOMING: usize = 100;
pub const MAX_FRIENDS: usize = 500;
/// Closed records nobody needs telling about are forgotten after this long,
/// and beyond this many the oldest go first.
pub const TOMBSTONE_TTL: Time = 180 * DAY;
pub const MAX_TOMBSTONES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Us,
    Them,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Relation {
    /// We asked. `delivered_at` is set once they confirmed they stored it.
    Outgoing {
        id: RequestId,
        created_at: Time,
        #[serde(default)]
        delivered_at: Option<Time>,
    },
    /// They asked.
    Incoming { id: RequestId, received_at: Time },
    /// We said yes and wait for them to confirm. Not a friend yet.
    Accepting { id: RequestId, accepted_at: Time },
    Friends { id: RequestId, since: Time },
    /// Over. `notify` stays true until the peer has been told.
    Closed {
        id: RequestId,
        reason: CloseReason,
        by: Side,
        at: Time,
        #[serde(default)]
        notify: bool,
    },
}

impl Relation {
    pub fn id(&self) -> RequestId {
        match self {
            Relation::Outgoing { id, .. }
            | Relation::Incoming { id, .. }
            | Relation::Accepting { id, .. }
            | Relation::Friends { id, .. }
            | Relation::Closed { id, .. } => *id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub relation: Relation,
    /// What they call themselves: a label, never an identifier.
    #[serde(default)]
    pub name: String,
    /// What we call them, if we chose something.
    #[serde(default)]
    pub alias: Option<String>,
}

/// Why a local action was refused. The message is what the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalError {
    OwnId,
    AlreadyFriends,
    AlreadyPending,
    NoRequest,
    NotFriends,
    TooManyFriends,
}

impl std::fmt::Display for LocalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            LocalError::OwnId => "that's your own ID",
            LocalError::AlreadyFriends => "you're already friends",
            LocalError::AlreadyPending => "a request is already pending",
            LocalError::NoRequest => "there's no such request",
            LocalError::NotFriends => "you're not friends",
            LocalError::TooManyFriends => "your friends list is full",
        })
    }
}

impl std::error::Error for LocalError {}

/// Everything we know about everyone, plus the in-memory counters that
/// bound what a stranger can make us store.
#[derive(Debug, Clone)]
pub struct Book {
    own: EndpointId,
    peers: BTreeMap<EndpointId, PeerRecord>,
    /// When each peer last sent us a request id we had not seen, for
    /// `MAX_NEW_REQUESTS`. Not persisted: a restart forgives.
    recent_requests: HashMap<EndpointId, Vec<Time>>,
}

impl Book {
    pub fn new(own: EndpointId) -> Self {
        Self::from_records(own, BTreeMap::new())
    }

    pub fn from_records(own: EndpointId, peers: BTreeMap<EndpointId, PeerRecord>) -> Self {
        Self {
            own,
            peers,
            recent_requests: HashMap::new(),
        }
    }

    pub fn records(&self) -> &BTreeMap<EndpointId, PeerRecord> {
        &self.peers
    }

    pub fn own_id(&self) -> EndpointId {
        self.own
    }

    pub fn relation(&self, peer: &EndpointId) -> Option<&Relation> {
        self.peers.get(peer).map(|r| &r.relation)
    }

    pub fn record(&self, peer: &EndpointId) -> Option<&PeerRecord> {
        self.peers.get(peer)
    }

    /// The one test every friendship-gated feature uses.
    pub fn is_friend(&self, peer: &EndpointId) -> bool {
        matches!(self.relation(peer), Some(Relation::Friends { .. }))
    }

    pub fn friend_count(&self) -> usize {
        self.peers
            .values()
            .filter(|r| matches!(r.relation, Relation::Friends { .. }))
            .count()
    }

    fn incoming_count(&self) -> usize {
        self.peers
            .values()
            .filter(|r| matches!(r.relation, Relation::Incoming { .. }))
            .count()
    }

    pub fn set_alias(&mut self, peer: &EndpointId, alias: Option<String>) -> Result<(), LocalError> {
        let record = self.peers.get_mut(peer).ok_or(LocalError::NotFriends)?;
        record.alias = alias
            .map(|a| crate::room::display_name(&a, &peer.to_string()))
            .filter(|a| *a != crate::room::short_id(&peer.to_string()));
        Ok(())
    }

    /// Sets `peer`'s relation, keeping its name and alias.
    fn set(&mut self, peer: EndpointId, relation: Relation) {
        match self.peers.get_mut(&peer) {
            Some(record) => record.relation = relation,
            None => {
                self.peers.insert(
                    peer,
                    PeerRecord {
                        relation,
                        name: String::new(),
                        alias: None,
                    },
                );
            }
        }
    }

    /// Takes the name a peer sent, cleaned up. Returns whether it changed.
    fn set_name(&mut self, peer: &EndpointId, name: &str) -> bool {
        let Some(record) = self.peers.get_mut(peer) else {
            return false;
        };
        if name.trim().is_empty() {
            return false;
        }
        let name = crate::room::display_name(name, &peer.to_string());
        if record.name == name {
            return false;
        }
        record.name = name;
        true
    }

    /// Closes `id`. Closed by us means the peer is owed the news.
    fn close(&mut self, peer: EndpointId, id: RequestId, reason: CloseReason, by: Side, now: Time) {
        self.set(
            peer,
            Relation::Closed {
                id,
                reason,
                by,
                at: now,
                notify: by == Side::Us,
            },
        );
    }

    // ------------------------------------------------------------ our actions

    /// Asks `peer` to be friends. To someone who already asked us, that is a yes.
    pub fn request(&mut self, peer: EndpointId, now: Time) -> Result<(), LocalError> {
        if peer == self.own {
            return Err(LocalError::OwnId);
        }
        match self.relation(&peer) {
            Some(Relation::Incoming { .. }) => self.accept(&peer, now),
            Some(Relation::Outgoing { .. } | Relation::Accepting { .. }) => Err(LocalError::AlreadyPending),
            Some(Relation::Friends { .. }) => Err(LocalError::AlreadyFriends),
            None | Some(Relation::Closed { .. }) => {
                if self.friend_count() >= MAX_FRIENDS {
                    return Err(LocalError::TooManyFriends);
                }
                self.set(
                    peer,
                    Relation::Outgoing {
                        id: RequestId::random(),
                        created_at: now,
                        delivered_at: None,
                    },
                );
                Ok(())
            }
        }
    }

    pub fn accept(&mut self, peer: &EndpointId, now: Time) -> Result<(), LocalError> {
        let Some(Relation::Incoming { id, .. }) = self.relation(peer) else {
            return Err(LocalError::NoRequest);
        };
        let id = *id;
        if self.friend_count() >= MAX_FRIENDS {
            return Err(LocalError::TooManyFriends);
        }
        self.set(*peer, Relation::Accepting { id, accepted_at: now });
        Ok(())
    }

    pub fn decline(&mut self, peer: &EndpointId, now: Time) -> Result<(), LocalError> {
        let Some(Relation::Incoming { id, .. }) = self.relation(peer) else {
            return Err(LocalError::NoRequest);
        };
        let id = *id;
        self.close(*peer, id, CloseReason::Declined, Side::Us, now);
        Ok(())
    }

    pub fn cancel(&mut self, peer: &EndpointId, now: Time) -> Result<(), LocalError> {
        let Some(Relation::Outgoing { id, .. }) = self.relation(peer) else {
            return Err(LocalError::NoRequest);
        };
        let id = *id;
        self.close(*peer, id, CloseReason::Canceled, Side::Us, now);
        Ok(())
    }

    /// Ends a friendship, or takes back a yes that was not confirmed yet.
    pub fn remove(&mut self, peer: &EndpointId, now: Time) -> Result<(), LocalError> {
        let id = match self.relation(peer) {
            Some(Relation::Friends { id, .. } | Relation::Accepting { id, .. }) => *id,
            _ => return Err(LocalError::NotFriends),
        };
        self.close(*peer, id, CloseReason::Removed, Side::Us, now);
        Ok(())
    }

    // ------------------------------------------------------- what to send

    /// The message `peer` still has to receive, if any: an undelivered
    /// request, an unconfirmed acceptance, or news of something we closed.
    pub fn pending(&self, peer: &EndpointId, own_name: &str) -> Option<FriendMsg> {
        match self.relation(peer)? {
            Relation::Outgoing {
                id, delivered_at: None, ..
            } => Some(FriendMsg::Request {
                id: *id,
                name: own_name.to_string(),
            }),
            Relation::Accepting { id, .. } => Some(FriendMsg::Accept { id: *id }),
            Relation::Closed {
                id,
                reason,
                by: Side::Us,
                notify: true,
                ..
            } => Some(match reason {
                CloseReason::Declined => FriendMsg::Decline { id: *id },
                CloseReason::Canceled => FriendMsg::Cancel { id: *id },
                CloseReason::Removed => FriendMsg::Remove { id: *id },
            }),
            _ => None,
        }
    }

    /// Every peer with something pending.
    pub fn pending_peers(&self) -> Vec<EndpointId> {
        self.peers
            .keys()
            .filter(|peer| self.pending(peer, "").is_some())
            .copied()
            .collect()
    }

    /// What to probe a friend with.
    pub fn ping(&self, peer: &EndpointId, own_name: &str) -> Option<FriendMsg> {
        match self.relation(peer)? {
            Relation::Friends { id, .. } => Some(FriendMsg::Ping {
                id: *id,
                name: own_name.to_string(),
            }),
            _ => None,
        }
    }

    pub fn friends(&self) -> Vec<EndpointId> {
        self.peers
            .iter()
            .filter(|(_, r)| matches!(r.relation, Relation::Friends { .. }))
            .map(|(id, _)| *id)
            .collect()
    }

    // ---------------------------------------------------- their messages

    /// Applies a message `peer` sent us and says what to answer. `Ring` is
    /// not handled here (see `net`); it gets `Unknown`.
    pub fn on_message(&mut self, peer: EndpointId, msg: &FriendMsg, now: Time) -> FriendReply {
        if peer == self.own {
            return FriendReply::Error {
                message: "that is our own id".into(),
            };
        }
        let current = self.relation(&peer).cloned();
        match msg {
            FriendMsg::Request { id, name } => self.on_request(peer, *id, name, current, now),
            FriendMsg::Accept { id } => match current {
                Some(Relation::Outgoing { id: ours, .. }) if ours == *id => {
                    if self.friend_count() >= MAX_FRIENDS {
                        return FriendReply::Busy;
                    }
                    self.set(peer, Relation::Friends { id: ours, since: now });
                    FriendReply::Ack
                }
                Some(Relation::Friends { id: ours, .. }) if ours == *id => FriendReply::Ack,
                Some(Relation::Closed { id: ours, reason, .. }) if ours == *id => FriendReply::Gone { reason },
                _ => FriendReply::Unknown,
            },
            FriendMsg::Decline { id } => {
                if let Some(Relation::Outgoing { id: ours, .. }) = current
                    && ours == *id
                {
                    self.close(peer, ours, CloseReason::Declined, Side::Them, now);
                }
                FriendReply::Ack
            }
            FriendMsg::Cancel { id } => {
                if let Some(Relation::Incoming { id: ours, .. } | Relation::Accepting { id: ours, .. }) = current
                    && ours == *id
                {
                    self.close(peer, ours, CloseReason::Canceled, Side::Them, now);
                }
                FriendReply::Ack
            }
            FriendMsg::Remove { id } => {
                if let Some(Relation::Friends { id: ours, .. } | Relation::Accepting { id: ours, .. }) = current
                    && ours == *id
                {
                    self.close(peer, ours, CloseReason::Removed, Side::Them, now);
                }
                FriendReply::Ack
            }
            FriendMsg::Ping { id, name } => match current {
                Some(Relation::Friends { id: ours, .. }) if ours == *id => {
                    self.set_name(&peer, name);
                    FriendReply::Pong { name: String::new() }
                }
                // They only ping once they count us as a friend, so this is
                // the confirmation our Accept was waiting for.
                Some(Relation::Accepting { id: ours, .. }) if ours == *id => {
                    self.set(peer, Relation::Friends { id: ours, since: now });
                    self.set_name(&peer, name);
                    FriendReply::Pong { name: String::new() }
                }
                Some(Relation::Closed { id: ours, reason, .. }) if ours == *id => FriendReply::Gone { reason },
                _ => FriendReply::Unknown,
            },
            FriendMsg::Ring { .. } => FriendReply::Unknown,
        }
    }

    fn on_request(
        &mut self,
        peer: EndpointId,
        id: RequestId,
        name: &str,
        current: Option<Relation>,
        now: Time,
    ) -> FriendReply {
        match current {
            Some(Relation::Incoming { id: ours, .. }) if ours == id => {
                self.set_name(&peer, name);
                FriendReply::Received
            }
            Some(Relation::Closed { id: ours, reason, .. }) if ours == id => FriendReply::Gone { reason },
            // We want this too: our own request, our yes, or a friendship
            // they have lost track of. Settle on the lower id, so a true race
            // of two requests still ends on one friendship.
            Some(
                Relation::Outgoing { id: ours, .. }
                | Relation::Accepting { id: ours, .. }
                | Relation::Friends { id: ours, .. },
            ) => {
                let since = match current {
                    Some(Relation::Friends { since, .. }) => Some(since),
                    _ => None,
                };
                if since.is_none() && self.friend_count() >= MAX_FRIENDS {
                    return FriendReply::Busy;
                }
                let settled = ours.min(id);
                self.set(
                    peer,
                    Relation::Friends {
                        id: settled,
                        since: since.unwrap_or(now),
                    },
                );
                self.set_name(&peer, name);
                FriendReply::Accepted { id: settled }
            }
            None | Some(Relation::Incoming { .. } | Relation::Closed { .. }) => {
                if let Some(Relation::Closed {
                    reason: CloseReason::Declined,
                    by: Side::Us,
                    at,
                    ..
                }) = current
                    && now - at < DECLINE_COOLDOWN
                {
                    return FriendReply::Gone {
                        reason: CloseReason::Declined,
                    };
                }
                let replacing = matches!(current, Some(Relation::Incoming { .. }));
                if !replacing && self.incoming_count() >= MAX_PENDING_INCOMING {
                    return FriendReply::Busy;
                }
                let recent = self.recent_requests.entry(peer).or_default();
                recent.retain(|t| now - *t < REQUEST_WINDOW);
                if recent.len() >= MAX_NEW_REQUESTS {
                    return FriendReply::Busy;
                }
                recent.push(now);
                self.set(peer, Relation::Incoming { id, received_at: now });
                self.set_name(&peer, name);
                FriendReply::Received
            }
        }
    }

    // ---------------------------------------------- replies to our messages

    /// Applies `peer`'s reply to a message we sent, if our state still is
    /// what it was when we sent it. Returns whether anything changed.
    pub fn on_reply(&mut self, peer: &EndpointId, sent: &FriendMsg, reply: &FriendReply, now: Time) -> bool {
        let Some(current) = self.relation(peer).cloned() else {
            return false;
        };
        let peer = *peer;
        match (sent, current, reply) {
            (
                FriendMsg::Request { id, .. },
                Relation::Outgoing {
                    id: ours,
                    created_at,
                    delivered_at: None,
                },
                FriendReply::Received,
            ) if ours == *id => {
                self.set(
                    peer,
                    Relation::Outgoing {
                        id: ours,
                        created_at,
                        delivered_at: Some(now),
                    },
                );
                true
            }
            // Ours, or the lower id of two crossing requests.
            (
                FriendMsg::Request { id, .. },
                Relation::Outgoing { id: ours, .. },
                FriendReply::Accepted { id: settled },
            ) if ours == *id && *settled <= ours => {
                self.set(peer, Relation::Friends { id: *settled, since: now });
                true
            }
            (FriendMsg::Request { id, .. }, Relation::Outgoing { id: ours, .. }, FriendReply::Gone { reason })
                if ours == *id =>
            {
                self.close(peer, ours, *reason, Side::Them, now);
                true
            }
            (FriendMsg::Accept { id }, Relation::Accepting { id: ours, .. }, FriendReply::Ack) if ours == *id => {
                self.set(peer, Relation::Friends { id: ours, since: now });
                true
            }
            (FriendMsg::Accept { id }, Relation::Accepting { id: ours, .. }, FriendReply::Gone { reason })
                if ours == *id =>
            {
                self.close(peer, ours, *reason, Side::Them, now);
                true
            }
            (FriendMsg::Accept { id }, Relation::Accepting { id: ours, .. }, FriendReply::Unknown) if ours == *id => {
                self.close(peer, ours, CloseReason::Canceled, Side::Them, now);
                true
            }
            (
                FriendMsg::Decline { id } | FriendMsg::Cancel { id } | FriendMsg::Remove { id },
                Relation::Closed {
                    id: ours,
                    reason,
                    by: Side::Us,
                    at,
                    notify: true,
                },
                FriendReply::Ack | FriendReply::Unknown | FriendReply::Gone { .. },
            ) if ours == *id => {
                self.set(
                    peer,
                    Relation::Closed {
                        id: ours,
                        reason,
                        by: Side::Us,
                        at,
                        notify: false,
                    },
                );
                true
            }
            (
                FriendMsg::Ping { id, .. },
                Relation::Friends { id: ours, .. },
                FriendReply::Gone {
                    reason: CloseReason::Removed,
                },
            ) if ours == *id => {
                self.close(peer, ours, CloseReason::Removed, Side::Them, now);
                true
            }
            (FriendMsg::Ping { id, .. }, Relation::Friends { id: ours, .. }, FriendReply::Pong { name })
                if ours == *id =>
            {
                self.set_name(&peer, name)
            }
            _ => false,
        }
    }

    /// Forgets closed records that are old and need no more telling.
    pub fn prune(&mut self, now: Time) {
        let forgettable_at = |r: &PeerRecord| match r.relation {
            Relation::Closed { notify: false, at, .. } => Some(at),
            _ => None,
        };
        self.peers
            .retain(|_, r| !forgettable_at(r).is_some_and(|at| now - at > TOMBSTONE_TTL));
        let mut closed: Vec<(Time, EndpointId)> = self
            .peers
            .iter()
            .filter_map(|(id, r)| forgettable_at(r).map(|at| (at, *id)))
            .collect();
        if closed.len() > MAX_TOMBSTONES {
            closed.sort();
            for (_, id) in &closed[..closed.len() - MAX_TOMBSTONES] {
                self.peers.remove(id);
            }
        }
        self.recent_requests.retain(|_, times| {
            times.retain(|t| now - *t < REQUEST_WINDOW);
            !times.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn key(n: u8) -> EndpointId {
        SecretKey::from_bytes(&[n; 32]).public()
    }

    const T0: Time = 1_800_000_000_000;

    /// Two books and a wire between them that delivers, drops or repeats.
    struct Pair {
        a: Book,
        b: Book,
        ia: EndpointId,
        ib: EndpointId,
    }

    impl Pair {
        fn new() -> Self {
            let (ia, ib) = (key(1), key(2));
            Self {
                a: Book::new(ia),
                b: Book::new(ib),
                ia,
                ib,
            }
        }

        /// A sends its pending message (if any) to B, B answers, A applies the
        /// answer. Returns the reply.
        fn a_to_b(&mut self, now: Time) -> Option<FriendReply> {
            let msg = self.a.pending(&self.ib, "alice")?;
            let reply = self.b.on_message(self.ia, &msg, now);
            self.a.on_reply(&self.ib, &msg, &reply, now);
            Some(reply)
        }

        fn b_to_a(&mut self, now: Time) -> Option<FriendReply> {
            let msg = self.b.pending(&self.ia, "bob")?;
            let reply = self.a.on_message(self.ib, &msg, now);
            self.b.on_reply(&self.ia, &msg, &reply, now);
            Some(reply)
        }

        /// B's pending message reaches A, but A's reply is lost.
        fn b_to_a_reply_lost(&mut self, now: Time) {
            let msg = self.b.pending(&self.ia, "bob").unwrap();
            self.a.on_message(self.ib, &msg, now);
        }

        fn friends_with_same_id(&self) -> bool {
            match (self.a.relation(&self.ib), self.b.relation(&self.ia)) {
                (Some(Relation::Friends { id: x, .. }), Some(Relation::Friends { id: y, .. })) => x == y,
                _ => false,
            }
        }
    }

    fn incoming(book: &Book, peer: &EndpointId) -> bool {
        matches!(book.relation(peer), Some(Relation::Incoming { .. }))
    }

    // ---------------------------------------------------------- happy path

    #[test]
    fn a_request_is_only_a_request_until_accepted() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        assert!(!p.a.is_friend(&p.ib));
        assert_eq!(p.a_to_b(T0), Some(FriendReply::Received));
        assert!(incoming(&p.b, &p.ia), "B sees the incoming request");
        assert!(!p.b.is_friend(&p.ia));
        assert!(matches!(
            p.a.relation(&p.ib),
            Some(Relation::Outgoing { delivered_at: Some(_), .. })
        ));
        assert_eq!(p.a.pending(&p.ib, "alice"), None, "delivered requests are not resent");
    }

    #[test]
    fn accepting_makes_both_friends_with_one_id() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0 + 1).unwrap();
        assert!(!p.b.is_friend(&p.ia), "accepting is not a friendship until confirmed");
        assert_eq!(p.b_to_a(T0 + 2), Some(FriendReply::Ack));
        assert!(p.a.is_friend(&p.ib));
        assert!(p.b.is_friend(&p.ia));
        assert!(p.friends_with_same_id());
        assert_eq!(p.b.pending(&p.ia, "bob"), None);
    }

    #[test]
    fn the_requesters_name_is_kept_as_a_label() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        assert_eq!(p.b.record(&p.ia).unwrap().name, "alice");
    }

    #[test]
    fn a_name_is_cleaned_before_it_is_kept() {
        let mut b = Book::new(key(2));
        let msg = FriendMsg::Request {
            id: RequestId([1; 16]),
            name: format!("  {}\u{7}", "x".repeat(80)),
        };
        b.on_message(key(1), &msg, T0);
        assert_eq!(b.record(&key(1)).unwrap().name, "x".repeat(32));
    }

    // ------------------------------------------------------ decline, cancel

    #[test]
    fn a_declined_request_never_becomes_a_friendship() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.decline(&p.ia, T0).unwrap();
        assert_eq!(p.b_to_a(T0), Some(FriendReply::Ack));
        assert!(matches!(
            p.a.relation(&p.ib),
            Some(Relation::Closed { reason: CloseReason::Declined, by: Side::Them, .. })
        ));
        assert!(!p.a.is_friend(&p.ib) && !p.b.is_friend(&p.ia));
        assert_eq!(p.b.pending(&p.ia, "bob"), None, "told once, then quiet");
    }

    #[test]
    fn a_canceled_request_disappears_from_the_recipient() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.a.cancel(&p.ib, T0).unwrap();
        assert_eq!(p.a_to_b(T0), Some(FriendReply::Ack));
        assert!(!incoming(&p.b, &p.ia));
        assert_eq!(p.a.pending(&p.ib, "alice"), None);
    }

    #[test]
    fn accepting_a_request_canceled_meanwhile_ends_it_on_both_sides() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        // A cancels, but the Cancel has not reached B when B accepts.
        p.a.cancel(&p.ib, T0).unwrap();
        p.b.accept(&p.ia, T0).unwrap();
        assert_eq!(p.b_to_a(T0), Some(FriendReply::Gone { reason: CloseReason::Canceled }));
        assert!(!p.a.is_friend(&p.ib) && !p.b.is_friend(&p.ia));
        assert_eq!(p.b.pending(&p.ia, "bob"), None);
    }

    #[test]
    fn a_cancel_reaching_a_recipient_mid_acceptance_stops_it() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0).unwrap();
        p.a.cancel(&p.ib, T0).unwrap();
        p.a_to_b(T0);
        assert!(matches!(p.b.relation(&p.ia), Some(Relation::Closed { .. })));
        assert_eq!(p.b.pending(&p.ia, "bob"), None);
        assert!(!p.a.is_friend(&p.ib));
    }

    // ---------------------------------------------------------- crossing

    #[test]
    fn crossing_requests_settle_on_the_lower_id_in_either_order() {
        for a_first in [true, false] {
            let mut p = Pair::new();
            p.a.request(p.ib, T0).unwrap();
            p.b.request(p.ia, T0).unwrap();
            let reply = if a_first { p.a_to_b(T0) } else { p.b_to_a(T0) };
            assert!(matches!(reply, Some(FriendReply::Accepted { .. })), "{reply:?}");
            assert!(p.friends_with_same_id(), "a_first={a_first}");
            // Nothing left to send either way.
            assert_eq!(p.a.pending(&p.ib, "alice"), None);
            assert_eq!(p.b.pending(&p.ia, "bob"), None);
        }
    }

    #[test]
    fn crossing_requests_in_a_true_race_still_agree() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.b.request(p.ia, T0).unwrap();
        let from_a = p.a.pending(&p.ib, "alice").unwrap();
        let from_b = p.b.pending(&p.ia, "bob").unwrap();
        // Both handlers run before either reply is seen.
        let reply_to_a = p.b.on_message(p.ia, &from_a, T0);
        let reply_to_b = p.a.on_message(p.ib, &from_b, T0);
        p.a.on_reply(&p.ib, &from_a, &reply_to_a, T0);
        p.b.on_reply(&p.ia, &from_b, &reply_to_b, T0);
        assert!(p.friends_with_same_id());
    }

    #[test]
    fn requesting_someone_who_asked_us_is_accepting() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.request(p.ia, T0).unwrap();
        assert!(matches!(p.b.relation(&p.ia), Some(Relation::Accepting { .. })));
        p.b_to_a(T0);
        assert!(p.friends_with_same_id());
    }

    // -------------------------------------------------------- duplicates

    #[test]
    fn duplicate_messages_change_nothing() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        let request = p.a.pending(&p.ib, "alice").unwrap();
        assert_eq!(p.b.on_message(p.ia, &request, T0), FriendReply::Received);
        assert_eq!(p.b.on_message(p.ia, &request, T0), FriendReply::Received);
        p.a.on_reply(&p.ib, &request, &FriendReply::Received, T0);
        p.a.on_reply(&p.ib, &request, &FriendReply::Received, T0);

        p.b.accept(&p.ia, T0).unwrap();
        let accept = p.b.pending(&p.ia, "bob").unwrap();
        assert_eq!(p.a.on_message(p.ib, &accept, T0), FriendReply::Ack);
        assert_eq!(p.a.on_message(p.ib, &accept, T0), FriendReply::Ack);
        p.b.on_reply(&p.ia, &accept, &FriendReply::Ack, T0);
        p.b.on_reply(&p.ia, &accept, &FriendReply::Ack, T0);
        assert!(p.friends_with_same_id());

        p.a.remove(&p.ib, T0).unwrap();
        let remove = p.a.pending(&p.ib, "alice").unwrap();
        assert_eq!(p.b.on_message(p.ia, &remove, T0), FriendReply::Ack);
        assert_eq!(p.b.on_message(p.ia, &remove, T0), FriendReply::Ack);
        assert!(!p.b.is_friend(&p.ia));
    }

    #[test]
    fn local_actions_refuse_what_makes_no_sense() {
        let own = key(1);
        let mut a = Book::new(own);
        assert_eq!(a.request(own, T0), Err(LocalError::OwnId));
        assert_eq!(a.accept(&key(2), T0), Err(LocalError::NoRequest));
        assert_eq!(a.remove(&key(2), T0), Err(LocalError::NotFriends));
        a.request(key(2), T0).unwrap();
        assert_eq!(a.request(key(2), T0), Err(LocalError::AlreadyPending));
    }

    // -------------------------------------------- interrupted acceptance

    #[test]
    fn a_lost_ack_is_recovered_by_retrying_the_accept() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0).unwrap();
        p.b_to_a_reply_lost(T0);
        assert!(p.a.is_friend(&p.ib));
        assert!(!p.b.is_friend(&p.ia), "B has not heard back");
        assert_eq!(p.b_to_a(T0 + 1000), Some(FriendReply::Ack));
        assert!(p.friends_with_same_id());
    }

    #[test]
    fn a_lost_ack_is_recovered_by_the_requesters_ping() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0).unwrap();
        p.b_to_a_reply_lost(T0);
        let ping = p.a.ping(&p.ib, "alice").unwrap();
        assert!(matches!(p.b.on_message(p.ia, &ping, T0), FriendReply::Pong { .. }));
        assert!(p.friends_with_same_id());
    }

    #[test]
    fn a_lost_received_means_the_request_is_resent_harmlessly() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        let msg = p.a.pending(&p.ib, "alice").unwrap();
        p.b.on_message(p.ia, &msg, T0); // reply lost
        assert_eq!(p.a_to_b(T0 + 1000), Some(FriendReply::Received));
        assert!(incoming(&p.b, &p.ia));
    }

    // ------------------------------------------------------------ removal

    fn friends() -> Pair {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0).unwrap();
        p.b_to_a(T0);
        assert!(p.friends_with_same_id());
        p
    }

    #[test]
    fn removing_an_offline_friend_takes_effect_at_once_and_reaches_them_later() {
        let mut p = friends();
        p.a.remove(&p.ib, T0 + 1).unwrap();
        assert!(!p.a.is_friend(&p.ib), "removal is immediate locally");
        assert!(p.b.is_friend(&p.ia), "B is offline and has not heard");
        assert!(p.a.pending(&p.ib, "alice").is_some(), "still owed a notice");
        p.a_to_b(T0 + DAY);
        assert!(!p.b.is_friend(&p.ia));
        assert_eq!(p.a.pending(&p.ib, "alice"), None);
    }

    #[test]
    fn a_removed_friend_learns_of_it_from_its_own_ping() {
        let mut p = friends();
        p.a.remove(&p.ib, T0 + 1).unwrap();
        let ping = p.b.ping(&p.ia, "bob").unwrap();
        let reply = p.a.on_message(p.ib, &ping, T0 + 2);
        assert_eq!(reply, FriendReply::Gone { reason: CloseReason::Removed });
        p.b.on_reply(&p.ia, &ping, &reply, T0 + 2);
        assert!(!p.b.is_friend(&p.ia));
    }

    #[test]
    fn stale_messages_cannot_recreate_a_removed_friendship() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        let request = p.a.pending(&p.ib, "alice").unwrap();
        p.a_to_b(T0);
        p.b.accept(&p.ia, T0).unwrap();
        let accept = p.b.pending(&p.ia, "bob").unwrap();
        p.b_to_a(T0);
        p.a.remove(&p.ib, T0 + 1).unwrap();
        p.a_to_b(T0 + 1);

        // Old messages turn up again, in both directions.
        assert!(matches!(p.a.on_message(p.ib, &accept, T0 + 2), FriendReply::Gone { .. }));
        assert!(matches!(p.b.on_message(p.ia, &request, T0 + 2), FriendReply::Gone { .. }));
        let ping = FriendMsg::Ping { id: accept_id(&accept), name: String::new() };
        assert!(matches!(p.a.on_message(p.ib, &ping, T0 + 2), FriendReply::Gone { .. }));
        assert!(!p.a.is_friend(&p.ib) && !p.b.is_friend(&p.ia));
    }

    fn accept_id(msg: &FriendMsg) -> RequestId {
        match msg {
            FriendMsg::Accept { id } => *id,
            other => panic!("not an accept: {other:?}"),
        }
    }

    #[test]
    fn friends_again_takes_a_new_request_and_a_new_yes() {
        let mut p = friends();
        let old = p.a.relation(&p.ib).unwrap().id();
        p.a.remove(&p.ib, T0 + 1).unwrap();
        p.a_to_b(T0 + 1);
        p.b.request(p.ia, T0 + 2).unwrap();
        p.b_to_a(T0 + 2);
        assert!(!p.a.is_friend(&p.ib), "a request alone is not a friendship");
        p.a.accept(&p.ib, T0 + 3).unwrap();
        p.a_to_b(T0 + 3);
        assert!(p.friends_with_same_id());
        assert_ne!(p.a.relation(&p.ib).unwrap().id(), old);
    }

    // ----------------------------------------------------------- forgery

    #[test]
    fn only_the_recipient_can_accept() {
        let (ia, ib, ic) = (key(1), key(2), key(3));
        let mut a = Book::new(ia);
        a.request(ib, T0).unwrap();
        let id = a.relation(&ib).unwrap().id();
        // C learned B's request id somehow and claims it.
        let reply = a.on_message(ic, &FriendMsg::Accept { id }, T0);
        assert_eq!(reply, FriendReply::Unknown);
        assert!(!a.is_friend(&ib) && !a.is_friend(&ic));
        assert!(matches!(a.relation(&ib), Some(Relation::Outgoing { .. })));
    }

    #[test]
    fn an_accept_with_the_wrong_id_is_refused() {
        let (ia, ib) = (key(1), key(2));
        let mut a = Book::new(ia);
        a.request(ib, T0).unwrap();
        let reply = a.on_message(ib, &FriendMsg::Accept { id: RequestId([9; 16]) }, T0);
        assert_eq!(reply, FriendReply::Unknown);
        assert!(!a.is_friend(&ib));
    }

    #[test]
    fn an_unsolicited_accept_is_refused() {
        let mut a = Book::new(key(1));
        let reply = a.on_message(key(2), &FriendMsg::Accept { id: RequestId([9; 16]) }, T0);
        assert_eq!(reply, FriendReply::Unknown);
        assert!(a.relation(&key(2)).is_none());
    }

    #[test]
    fn a_reply_to_a_message_we_no_longer_stand_by_is_ignored() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        let request = p.a.pending(&p.ib, "alice").unwrap();
        p.a.cancel(&p.ib, T0).unwrap();
        // A late "Accepted" for the canceled request must not revive it.
        let id = p.a.relation(&p.ib).unwrap().id();
        p.a.on_reply(&p.ib, &request, &FriendReply::Accepted { id }, T0);
        assert!(!p.a.is_friend(&p.ib));
    }

    // ------------------------------------------------------------- limits

    #[test]
    fn a_declined_peer_is_turned_away_for_a_while() {
        let mut p = Pair::new();
        p.a.request(p.ib, T0).unwrap();
        p.a_to_b(T0);
        p.b.decline(&p.ia, T0).unwrap();
        p.b_to_a(T0);
        p.a.request(p.ib, T0 + HOUR).unwrap();
        assert_eq!(p.a_to_b(T0 + HOUR), Some(FriendReply::Gone { reason: CloseReason::Declined }));
        p.a.request(p.ib, T0 + 2 * DAY).unwrap();
        assert_eq!(p.a_to_b(T0 + 2 * DAY), Some(FriendReply::Received));
    }

    #[test]
    fn one_peer_cannot_flood_us_with_new_requests() {
        let mut b = Book::new(key(2));
        for n in 0..MAX_NEW_REQUESTS as u8 {
            let msg = FriendMsg::Request { id: RequestId([n; 16]), name: String::new() };
            assert_eq!(b.on_message(key(1), &msg, T0), FriendReply::Received);
        }
        let msg = FriendMsg::Request { id: RequestId([99; 16]), name: String::new() };
        assert_eq!(b.on_message(key(1), &msg, T0), FriendReply::Busy);
        assert_eq!(b.on_message(key(1), &msg, T0 + REQUEST_WINDOW + 1), FriendReply::Received);
    }

    #[test]
    fn pending_requests_from_everyone_are_capped() {
        let mut b = Book::new(key(0));
        for n in 1..=MAX_PENDING_INCOMING {
            let peer = SecretKey::from_bytes(&[n as u8; 32]).public();
            let msg = FriendMsg::Request { id: RequestId::random(), name: String::new() };
            assert_eq!(b.on_message(peer, &msg, T0), FriendReply::Received, "n={n}");
        }
        let extra = SecretKey::from_bytes(&[251; 32]).public();
        let msg = FriendMsg::Request { id: RequestId::random(), name: String::new() };
        assert_eq!(b.on_message(extra, &msg, T0), FriendReply::Busy);
    }

    #[test]
    fn a_friend_who_lost_us_asking_again_is_let_straight_back() {
        let mut p = friends();
        // B lost its list and asks again with a fresh id.
        p.b = Book::new(p.ib);
        p.b.request(p.ia, T0 + 5).unwrap();
        assert!(matches!(p.b_to_a(T0 + 5), Some(FriendReply::Accepted { .. })));
        assert!(p.friends_with_same_id());
    }

    #[test]
    fn a_ping_we_cannot_match_is_answered_without_harm() {
        let mut p = friends();
        let ping = FriendMsg::Ping { id: RequestId([7; 16]), name: String::new() };
        assert_eq!(p.a.on_message(p.ib, &ping, T0), FriendReply::Unknown);
        assert!(p.a.is_friend(&p.ib));
        // And Unknown to our ping does not end it on our side either.
        let ours = p.a.ping(&p.ib, "alice").unwrap();
        p.a.on_reply(&p.ib, &ours, &FriendReply::Unknown, T0);
        assert!(p.a.is_friend(&p.ib));
    }

    #[test]
    fn old_tombstones_are_pruned_but_owed_notices_are_kept() {
        let mut p = friends();
        p.a.remove(&p.ib, T0).unwrap();
        p.a.prune(T0 + TOMBSTONE_TTL * 2);
        assert!(p.a.relation(&p.ib).is_some(), "B still has to be told");
        p.a_to_b(T0 + 1);
        p.a.prune(T0 + TOMBSTONE_TTL * 2);
        assert!(p.a.relation(&p.ib).is_none());
    }

    #[test]
    fn records_round_trip_through_json() {
        let p = friends();
        let json = serde_json::to_string(p.a.records().values().next().unwrap()).unwrap();
        let back: PeerRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, p.a.records().values().next().unwrap());
    }
}
