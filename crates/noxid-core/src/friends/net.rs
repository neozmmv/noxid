//! The friends ALPN on the wire: the handler for what peers send us, the
//! background loop that delivers what we owe them and probes friends for
//! presence, and rings.
//!
//! Bounds: a few inbound connections at once, a per-peer message rate, a few
//! outbound dials at once, a timeout on every exchange, and backoff on every
//! retry, so neither a flood nor an unreachable peer costs much.

use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, bail};
use iroh::EndpointId;
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tokio::time::Instant;
use tracing::debug;

use super::wire::{FRIENDS_ALPN, FriendMsg, FriendReply, MAX_MESSAGE, RequestId};
use super::{Calling, FriendEvent, IncomingRing, RingOutcome, Shared, now_ms};
use crate::room::Code;
use crate::rpc;

/// Inbound friends connections handled at once; beyond that they are turned
/// away and the sender retries later.
pub(super) const MAX_INBOUND: usize = 16;
/// Outbound dials at once, for delivery and presence together.
pub(super) const MAX_DIALS: usize = 4;
/// How long a peer gets to send its message once connected.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Rings at once, from different friends.
const MAX_RINGS: usize = 3;
/// Peers whose message rate is tracked before the oldest are forgotten.
const MAX_RATE_ENTRIES: usize = 1024;
/// Close code for a connection turned away because too many are open.
const CLOSE_BUSY: u32 = 2;

#[derive(Clone)]
pub struct FriendsHandler {
    pub(super) shared: Arc<Shared>,
}

impl std::fmt::Debug for FriendsHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FriendsHandler").finish_non_exhaustive()
    }
}

impl ProtocolHandler for FriendsHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let shared = &self.shared;
        let peer = connection.remote_id();
        let Ok(_permit) = shared.inbound.try_acquire() else {
            connection.close(CLOSE_BUSY.into(), b"busy");
            return Ok(());
        };
        let (send, reply) = match rpc::read_request::<FriendMsg>(&connection, MAX_MESSAGE, REQUEST_TIMEOUT).await? {
            rpc::Incoming::Request(send, msg) => {
                if !shared.within_rate(peer) {
                    (send, FriendReply::Busy)
                } else if let FriendMsg::Ring {
                    call_id,
                    code,
                    room_name,
                } = msg
                {
                    match on_ring(shared, &connection, peer, call_id, code, room_name).await {
                        Some(reply) => (send, reply),
                        // They hung up; nobody is left to answer.
                        None => return Ok(()),
                    }
                } else {
                    (send, on_message(shared, peer, &msg).await)
                }
            }
            rpc::Incoming::Malformed(send, message) => (send, FriendReply::Error { message }),
            rpc::Incoming::TimedOut => return Ok(()),
        };
        rpc::reply(&connection, send, &reply).await
    }
}

impl Shared {
    /// A token bucket per peer: `inbound_per_minute`, refilled continuously.
    fn within_rate(&self, peer: EndpointId) -> bool {
        let per_minute = f64::from(self.timing.inbound_per_minute);
        let now = Instant::now();
        let mut rate = self.rate.lock().unwrap();
        if rate.len() >= MAX_RATE_ENTRIES && !rate.contains_key(&peer) {
            rate.retain(|_, (_, at)| now.duration_since(*at) < Duration::from_secs(60));
        }
        let (tokens, at) = rate.entry(peer).or_insert((per_minute, now));
        let refill = now.duration_since(*at).as_secs_f64() / 60.0 * per_minute;
        *tokens = (*tokens + refill).min(per_minute);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// We reached `peer`, or it reached us: it is online, and anything we owe
    /// it can go now.
    fn contact_ok(&self, peer: EndpointId) {
        let owes = self.book.lock().unwrap().pending(&peer, "").is_some();
        {
            let mut links = self.links.lock().unwrap();
            let link = links.entry(peer).or_default();
            link.last_ok = Some(now_ms());
            if owes && !link.sending {
                link.next_send = None;
                link.send_attempts = 0;
            }
        }
        if owes {
            self.wake.notify_one();
        }
    }

    fn contact_failed(&self, peer: EndpointId) {
        self.links.lock().unwrap().entry(peer).or_default().last_fail = Some(now_ms());
    }

    /// Doubling from `min` per attempt up to `retry_max`, give or take a fifth.
    fn backoff(&self, min: Duration, attempts: u32) -> Duration {
        let base = min
            .saturating_mul(1u32 << attempts.min(16))
            .min(self.timing.retry_max);
        let mut byte = [0u8; 1];
        let _ = getrandom::fill(&mut byte);
        // 0.8 to 1.2
        let factor = 0.8 + f64::from(byte[0]) / 255.0 * 0.4;
        base.mul_f64(factor)
    }

    /// Applies a reply to something we sent, storing and announcing changes.
    async fn apply_reply(&self, peer: EndpointId, sent: &FriendMsg, reply: &FriendReply) {
        let (before, after, changed) = {
            let mut book = self.book.lock().unwrap();
            let before = book.relation(&peer).cloned();
            let changed = book.on_reply(&peer, sent, reply, now_ms());
            (before, book.relation(&peer).cloned(), changed)
        };
        if changed {
            if let Err(e) = self.persist().await {
                tracing::warn!("could not save the friends list: {e:#}");
            }
            self.notify_transition(peer, before.as_ref(), after.as_ref(), true);
            self.bump();
        }
    }
}

/// Handles everything but rings: applies it, stores the change, then replies.
async fn on_message(shared: &Shared, peer: EndpointId, msg: &FriendMsg) -> FriendReply {
    if shared.problem.is_some() {
        return FriendReply::Busy;
    }
    let (reply, before, after) = {
        let mut book = shared.book.lock().unwrap();
        let before = book.relation(&peer).cloned();
        let reply = book.on_message(peer, msg, now_ms());
        (reply, before, book.relation(&peer).cloned())
    };
    if before != after {
        // Stored before we answer, so a yes is never lost to a crash.
        if let Err(e) = shared.persist().await {
            tracing::warn!("could not save the friends list: {e:#}");
            return FriendReply::Busy;
        }
        shared.notify_transition(peer, before.as_ref(), after.as_ref(), true);
        shared.bump();
    }
    // Only peers we keep a record of get a link: a stranger's message must
    // not grow what we hold in memory.
    if after.is_some() {
        shared.contact_ok(peer);
    }
    if matches!(msg, FriendMsg::Ping { .. }) && matches!(reply, FriendReply::Pong { .. }) {
        shared.links.lock().unwrap().entry(peer).or_default().unaware = false;
        shared.bump();
        return FriendReply::Pong {
            name: shared.name.borrow().clone(),
        };
    }
    reply
}

/// A friend ringing us: hold the connection until we answer, it times out,
/// or they hang up. `None` when they hung up.
async fn on_ring(
    shared: &Shared,
    connection: &Connection,
    peer: EndpointId,
    call_id: RequestId,
    code: String,
    room_name: Option<String>,
) -> Option<FriendReply> {
    if !shared.book.lock().unwrap().is_friend(&peer) {
        return Some(FriendReply::Unknown);
    }
    // Only an invite to the caller's own room: a friend cannot ring us into
    // somebody else's.
    match Code::decode(&code) {
        Ok(Code::Invite { coordinator, .. }) if coordinator == peer => {}
        _ => {
            return Some(FriendReply::Error {
                message: "a ring must carry an invite to the caller's own room".into(),
            });
        }
    }
    let room_name = room_name.map(|n| n.chars().filter(|c| !c.is_control()).take(48).collect());
    let (answer, answered) = tokio::sync::oneshot::channel();
    {
        let mut rings = shared.rings.lock().unwrap();
        if rings.contains_key(&peer) || rings.len() >= MAX_RINGS {
            return Some(FriendReply::Busy);
        }
        rings.insert(
            peer,
            IncomingRing {
                call_id,
                code,
                room_name,
                expires_at: now_ms() + shared.timing.ring.as_millis() as i64,
                answer,
            },
        );
    }
    shared.contact_ok(peer);
    let _ = shared.events.send(FriendEvent::Ringing {
        id: peer.to_string(),
        name: shared.display_name(&peer),
    });
    shared.bump();

    let outcome = tokio::select! {
        answer = answered => Some(match answer {
            Ok(true) => FriendReply::Answered,
            Ok(false) => FriendReply::Declined,
            Err(_) => FriendReply::NoAnswer,
        }),
        _ = tokio::time::sleep(shared.timing.ring) => Some(FriendReply::NoAnswer),
        _ = connection.closed() => None,
    };
    let unanswered = {
        let mut rings = shared.rings.lock().unwrap();
        // Still ours to clear only if nobody answered it meanwhile.
        let still_ringing = rings.get(&peer).is_some_and(|r| r.call_id == call_id);
        if still_ringing {
            rings.remove(&peer);
        }
        still_ringing
    };
    if unanswered {
        shared.missed_call(peer);
    }
    shared.bump();
    outcome
}

/// Rings `peer` and waits for the outcome, or for us to hang up.
pub(super) async fn ring(
    shared: &Arc<Shared>,
    peer: EndpointId,
    code: String,
    room_name: Option<String>,
) -> Result<RingOutcome> {
    if !shared.book.lock().unwrap().is_friend(&peer) {
        bail!("you can only call friends");
    }
    let (hang_up, hung_up) = tokio::sync::oneshot::channel();
    {
        let mut calling = shared.calling.lock().unwrap();
        if calling.is_some() {
            bail!("you're already calling someone");
        }
        *calling = Some(Calling {
            to: peer,
            since: now_ms(),
            hang_up: Some(hang_up),
        });
    }
    shared.bump();

    let msg = FriendMsg::Ring {
        call_id: RequestId::random(),
        code,
        room_name,
    };
    // Long enough for their own timeout to answer before ours fires.
    let wait = shared.timing.ring + shared.timing.call * 2;
    let call = rpc::call::<_, FriendReply>(&shared.endpoint, peer, FRIENDS_ALPN, &msg, MAX_MESSAGE, wait);
    let outcome = tokio::select! {
        reply = call => match reply {
            Ok(reply) => {
                shared.contact_ok(peer);
                match reply {
                    FriendReply::Answered => RingOutcome::Answered,
                    FriendReply::Declined => RingOutcome::Declined,
                    FriendReply::NoAnswer => RingOutcome::NoAnswer,
                    FriendReply::Busy => RingOutcome::Busy,
                    FriendReply::Unknown => RingOutcome::NotFriends,
                    other => RingOutcome::Unreachable(format!("unexpected reply: {other:?}")),
                }
            }
            Err(e) => {
                shared.contact_failed(peer);
                RingOutcome::Unreachable(format!("{e:#}"))
            }
        },
        // Dropping the call drops the connection, which stops their ring.
        _ = hung_up => RingOutcome::HungUp,
    };
    *shared.calling.lock().unwrap() = None;
    shared.bump();
    Ok(outcome)
}

/// Delivers what we owe `peer`, once.
async fn deliver(shared: Arc<Shared>, peer: EndpointId) {
    let name = shared.name.borrow().clone();
    let msg = shared.book.lock().unwrap().pending(&peer, &name);
    if let Some(msg) = msg {
        let _permit = shared.dials.acquire().await;
        let result = rpc::call::<_, FriendReply>(
            &shared.endpoint,
            peer,
            FRIENDS_ALPN,
            &msg,
            MAX_MESSAGE,
            shared.timing.call,
        )
        .await;
        let error = match result {
            Ok(reply) => {
                shared.contact_ok(peer);
                shared.apply_reply(peer, &msg, &reply).await;
                matches!(reply, FriendReply::Busy | FriendReply::Error { .. }).then(|| format!("{reply:?}"))
            }
            Err(e) => {
                debug!(%peer, "friends delivery failed: {e:#}");
                shared.contact_failed(peer);
                Some(short_error(&e))
            }
        };
        let still_owed = shared.book.lock().unwrap().pending(&peer, "").is_some();
        let mut links = shared.links.lock().unwrap();
        let link = links.entry(peer).or_default();
        if still_owed && error.is_some() {
            link.send_attempts = link.send_attempts.saturating_add(1);
            link.next_send = Some(Instant::now() + shared.backoff(shared.timing.retry_min, link.send_attempts - 1));
        } else {
            link.send_attempts = 0;
            link.next_send = None;
        }
        link.last_error = error;
    }
    let mut links = shared.links.lock().unwrap();
    let link = links.entry(peer).or_default();
    link.sending = false;
    link.forced = false;
    drop(links);
    shared.bump();
}

/// Probes a friend: presence, a fresh name, and a check that they still
/// count us as a friend.
async fn probe(shared: Arc<Shared>, peer: EndpointId) {
    let name = shared.name.borrow().clone();
    let msg = shared.book.lock().unwrap().ping(&peer, &name);
    let mut ok = false;
    if let Some(msg) = msg {
        let _permit = shared.dials.acquire().await;
        let result = rpc::call::<_, FriendReply>(
            &shared.endpoint,
            peer,
            FRIENDS_ALPN,
            &msg,
            MAX_MESSAGE,
            shared.timing.call,
        )
        .await;
        match result {
            Ok(reply) => {
                ok = true;
                shared.contact_ok(peer);
                shared.links.lock().unwrap().entry(peer).or_default().unaware =
                    matches!(reply, FriendReply::Unknown);
                shared.apply_reply(peer, &msg, &reply).await;
            }
            Err(e) => {
                debug!(%peer, "friend probe failed: {e:#}");
                shared.contact_failed(peer);
            }
        }
    }
    let mut links = shared.links.lock().unwrap();
    let link = links.entry(peer).or_default();
    link.probing = false;
    if ok {
        link.probe_attempts = 0;
        link.next_probe = Some(Instant::now() + shared.timing.probe_every);
    } else {
        // Offline: back off from half the probe interval up to the cap.
        let delay = shared.backoff(shared.timing.probe_every / 2, link.probe_attempts);
        link.probe_attempts = link.probe_attempts.saturating_add(1);
        link.next_probe = Some(Instant::now() + delay);
    }
    drop(links);
    shared.bump();
}

fn short_error(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    if text.contains("timed out") {
        "they could not be reached".into()
    } else {
        text.chars().take(160).collect()
    }
}

/// The background loop: every tick, or when woken, starts delivery to peers
/// we owe something and probes friends that are due. Ends when the friends
/// list is dropped.
pub(super) async fn run(shared: Weak<Shared>) {
    let tick = match shared.upgrade() {
        Some(s) => s.timing.tick,
        None => return,
    };
    loop {
        {
            let Some(shared) = shared.upgrade() else { return };
            if shared.problem.is_none() {
                start_due_work(&shared);
            }
            // The strong reference lasts one tick at most, so the loop ends
            // within a tick of the list being dropped.
            tokio::select! {
                _ = shared.wake.notified() => {}
                _ = tokio::time::sleep(tick) => {}
            }
        }
    }
}

fn start_due_work(shared: &Arc<Shared>) {
    let now = Instant::now();
    let now_ms = now_ms();
    let give_up = shared.timing.give_up_after.as_millis() as i64;
    let (owed, friends) = {
        let book = shared.book.lock().unwrap();
        let owed: Vec<(EndpointId, bool)> = book
            .pending_peers()
            .into_iter()
            .map(|peer| {
                let stale = matches!(
                    book.relation(&peer),
                    Some(super::state::Relation::Outgoing { created_at, delivered_at: None, .. })
                        if now_ms - created_at > give_up
                );
                (peer, stale)
            })
            .collect();
        (owed, book.friends())
    };
    let mut links = shared.links.lock().unwrap();
    for (peer, stale) in owed {
        let link = links.entry(peer).or_default();
        let due = link.next_send.is_none_or(|at| at <= now);
        if link.sending || !due || (stale && !link.forced) {
            continue;
        }
        link.sending = true;
        tokio::spawn(deliver(shared.clone(), peer));
    }
    for peer in friends {
        let link = links.entry(peer).or_default();
        let due = link.next_probe.is_none_or(|at| at <= now);
        if link.probing || !due {
            continue;
        }
        link.probing = true;
        tokio::spawn(probe(shared.clone(), peer));
    }
}
