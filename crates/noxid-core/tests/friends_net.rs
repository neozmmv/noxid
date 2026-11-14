//! The friends protocol between real iroh endpoints on loopback: no relay, no
//! DNS, addresses handed around in memory. Each peer has its own key and its
//! own friends.json, and can be stopped and started again on a new port with
//! the same identity, as an app restart would.

use std::path::PathBuf;
use std::time::Duration;

use iroh::address_lookup::MemoryLookup;
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, TransportAddr};
use noxid_core::friends::state::Relation;
use noxid_core::friends::wire::{FRIENDS_ALPN, FriendMsg, FriendReply, RequestId};
use noxid_core::friends::{Friends, FriendsConfig, Presence, RingOutcome, Timing, parse_friend_id};
use tokio::sync::watch;

struct Peer {
    key: SecretKey,
    dir: PathBuf,
    name: watch::Sender<String>,
    endpoint: Endpoint,
    router: Router,
    friends: Friends,
}

impl Peer {
    fn id(&self) -> EndpointId {
        self.key.public()
    }

    async fn start(key: SecretKey, dir: PathBuf, name: &str, lookup: &MemoryLookup) -> Self {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .relay_mode(RelayMode::Disabled)
            .address_lookup(lookup.clone())
            .bind_addr("127.0.0.1:0")
            .unwrap()
            .bind()
            .await
            .unwrap();
        lookup.add_endpoint_info(EndpointAddr {
            id: endpoint.id(),
            addrs: endpoint.bound_sockets().into_iter().map(TransportAddr::Ip).collect(),
        });
        let (name_tx, name_rx) = watch::channel(name.to_string());
        let friends = Friends::spawn(FriendsConfig {
            endpoint: endpoint.clone(),
            store_path: dir.join("friends.json"),
            name: name_rx,
            timing: Timing::fast(),
        });
        let router = Router::builder(endpoint.clone())
            .accept(FRIENDS_ALPN, friends.handler())
            .spawn();
        Self {
            key,
            dir,
            name: name_tx,
            endpoint,
            router,
            friends,
        }
    }

    /// Goes offline: nothing answers on its address any more.
    async fn stop(self) -> (SecretKey, PathBuf) {
        let _ = self.router.shutdown().await;
        self.endpoint.close().await;
        drop(self.name);
        (self.key, self.dir)
    }

    async fn restart(self, lookup: &MemoryLookup) -> Self {
        let name = self.name.borrow().clone();
        let (key, dir) = self.stop().await;
        Self::start(key, dir, &name, lookup).await
    }
}

fn temp_dir(test: &str, who: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("noxid-friends-net-{test}-{who}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn pair(test: &str) -> (MemoryLookup, Peer, Peer) {
    let lookup = MemoryLookup::new();
    let a = Peer::start(SecretKey::from_bytes(&[1; 32]), temp_dir(test, "a"), "alice", &lookup).await;
    let b = Peer::start(SecretKey::from_bytes(&[2; 32]), temp_dir(test, "b"), "bob", &lookup).await;
    (lookup, a, b)
}

/// Waits for `check` to hold, failing the test after ten seconds.
async fn until(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn befriend(a: &Peer, b: &Peer) {
    a.friends.send_request(b.id()).await.unwrap();
    until("B sees A's request", || b.friends.view().incoming.iter().any(|r| r.id == a.id().to_string())).await;
    b.friends.answer(a.id(), true).await.unwrap();
    until("both are friends", || a.friends.is_friend(&b.id()) && b.friends.is_friend(&a.id())).await;
}

fn same_friendship(a: &Peer, b: &Peer) -> bool {
    match (a.friends.relation(&b.id()), b.friends.relation(&a.id())) {
        (Some(Relation::Friends { id: x, .. }), Some(Relation::Friends { id: y, .. })) => x == y,
        _ => false,
    }
}

/// Sends one message on the friends ALPN as `endpoint`, the way a peer that
/// is not running Noxid's own code might.
async fn raw_call(endpoint: &Endpoint, to: EndpointId, payload: &[u8]) -> Option<FriendReply> {
    let conn = endpoint.connect(to, FRIENDS_ALPN).await.unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(payload).await.unwrap();
    send.finish().unwrap();
    let data = recv.read_to_end(64 * 1024).await.ok()?;
    conn.close(0u32.into(), b"done");
    serde_json::from_slice(&data).ok()
}

fn invite_from(coordinator: &SecretKey) -> String {
    let mut room = noxid_core::room::Room::default();
    let id = coordinator.public();
    room.create(None, &id.to_string(), "host");
    room.generate_invite(id, chrono::Duration::minutes(1), chrono::Utc::now())
        .unwrap()
        .0
}

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn a_request_and_an_acceptance_make_two_friends() {
    let (_lookup, a, b) = pair("accept").await;
    a.friends.send_request(b.id()).await.unwrap();
    assert!(!a.friends.is_friend(&b.id()), "a request is not a friendship");
    until("B sees A's request with A's name", || {
        b.friends
            .view()
            .incoming
            .iter()
            .any(|r| r.id == a.id().to_string() && r.name == "alice")
    })
    .await;
    until("A sees it delivered", || {
        a.friends.view().outgoing.iter().any(|r| r.delivered_at.is_some())
    })
    .await;
    assert!(!b.friends.is_friend(&a.id()));
    b.friends.answer(a.id(), true).await.unwrap();
    until("both are friends", || a.friends.is_friend(&b.id()) && b.friends.is_friend(&a.id())).await;
    assert!(same_friendship(&a, &b));
    assert!(a.friends.view().outgoing.is_empty() && b.friends.view().incoming.is_empty());
}

#[tokio::test]
async fn a_declined_request_leaves_no_friendship() {
    let (_lookup, a, b) = pair("decline").await;
    a.friends.send_request(b.id()).await.unwrap();
    until("B sees it", || !b.friends.view().incoming.is_empty()).await;
    b.friends.answer(a.id(), false).await.unwrap();
    until("A's request is gone", || a.friends.view().outgoing.is_empty()).await;
    assert!(!a.friends.is_friend(&b.id()) && !b.friends.is_friend(&a.id()));
}

#[tokio::test]
async fn a_canceled_request_disappears_for_the_recipient() {
    let (_lookup, a, b) = pair("cancel").await;
    a.friends.send_request(b.id()).await.unwrap();
    until("B sees it", || !b.friends.view().incoming.is_empty()).await;
    a.friends.cancel(b.id()).await.unwrap();
    until("B's incoming request is gone", || b.friends.view().incoming.is_empty()).await;
    assert!(!a.friends.is_friend(&b.id()) && !b.friends.is_friend(&a.id()));
}

#[tokio::test]
async fn simultaneous_requests_end_in_one_friendship() {
    let (_lookup, a, b) = pair("simultaneous").await;
    let (ra, rb) = tokio::join!(a.friends.send_request(b.id()), b.friends.send_request(a.id()));
    ra.unwrap();
    rb.unwrap();
    until("both are friends with the same id", || same_friendship(&a, &b)).await;
}

#[tokio::test]
async fn friendships_and_identity_survive_a_restart() {
    let (lookup, a, b) = pair("restart").await;
    befriend(&a, &b).await;
    let a_id = a.id();
    let a = a.restart(&lookup).await;
    assert_eq!(a.id(), a_id);
    assert!(a.friends.is_friend(&b.id()), "the friendship was on disk");
    assert!(same_friendship(&a, &b));
}

#[tokio::test]
async fn a_request_to_an_offline_peer_arrives_when_it_is_back() {
    let (lookup, a, b) = pair("offline").await;
    let (b_key, b_dir) = b.stop().await;
    a.friends.send_request(b_key.public()).await.unwrap();
    until("A has tried and failed", || {
        a.friends
            .view()
            .outgoing
            .iter()
            .any(|r| r.delivered_at.is_none() && r.last_error.is_some())
    })
    .await;
    let b = Peer::start(b_key, b_dir, "bob", &lookup).await;
    until("B gets the request after coming back", || !b.friends.view().incoming.is_empty()).await;
    until("A sees it delivered", || {
        a.friends.view().outgoing.iter().any(|r| r.delivered_at.is_some())
    })
    .await;
}

#[tokio::test]
async fn an_acceptance_made_while_the_requester_is_offline_completes_later() {
    let (lookup, a, b) = pair("interrupted").await;
    a.friends.send_request(b.id()).await.unwrap();
    until("B sees it", || !b.friends.view().incoming.is_empty()).await;
    let (a_key, a_dir) = a.stop().await;
    b.friends.answer(a_key.public(), true).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!b.friends.is_friend(&a_key.public()), "not a friend until A confirms");
    assert!(b.friends.view().friends.iter().any(|f| f.finishing));
    let a = Peer::start(a_key, a_dir, "alice", &lookup).await;
    until("both are friends", || same_friendship(&a, &b)).await;
}

#[tokio::test]
async fn removing_an_offline_friend_reaches_them_when_they_return() {
    let (lookup, a, b) = pair("remove-offline").await;
    befriend(&a, &b).await;
    let (b_key, b_dir) = b.stop().await;
    a.friends.remove(b_key.public()).await.unwrap();
    assert!(!a.friends.is_friend(&b_key.public()), "removal is immediate");
    let b = Peer::start(b_key, b_dir, "bob", &lookup).await;
    until("B no longer counts A as a friend", || !b.friends.is_friend(&a.id())).await;
}

#[tokio::test]
async fn a_stale_acceptance_cannot_bring_back_a_removed_friend() {
    let (_lookup, a, b) = pair("stale").await;
    a.friends.send_request(b.id()).await.unwrap();
    until("B sees it", || !b.friends.view().incoming.is_empty()).await;
    let request_id = a.friends.relation(&b.id()).unwrap().id();
    b.friends.answer(a.id(), true).await.unwrap();
    until("both are friends", || same_friendship(&a, &b)).await;
    a.friends.remove(b.id()).await.unwrap();
    until("B saw the removal", || !b.friends.is_friend(&a.id())).await;

    // B's old Accept turns up again.
    let payload = serde_json::to_vec(&FriendMsg::Accept { id: request_id }).unwrap();
    let reply = raw_call(&b.endpoint, a.id(), &payload).await;
    assert!(matches!(reply, Some(FriendReply::Gone { .. })), "{reply:?}");
    assert!(!a.friends.is_friend(&b.id()));
}

#[tokio::test]
async fn nobody_else_can_accept_on_the_recipients_behalf() {
    let (lookup, a, b) = pair("forge").await;
    let c = Peer::start(SecretKey::from_bytes(&[3; 32]), temp_dir("forge", "c"), "carol", &lookup).await;
    a.friends.send_request(b.id()).await.unwrap();
    until("delivered", || !b.friends.view().incoming.is_empty()).await;
    let id = a.friends.relation(&b.id()).unwrap().id();

    let payload = serde_json::to_vec(&FriendMsg::Accept { id }).unwrap();
    let reply = raw_call(&c.endpoint, a.id(), &payload).await;
    assert_eq!(reply, Some(FriendReply::Unknown));
    assert!(!a.friends.is_friend(&b.id()) && !a.friends.is_friend(&c.id()));
    assert!(matches!(a.friends.relation(&b.id()), Some(Relation::Outgoing { .. })));
}

#[tokio::test]
async fn malformed_messages_get_an_error_and_change_nothing() {
    let (_lookup, a, b) = pair("malformed").await;
    let reply = raw_call(&b.endpoint, a.id(), b"{\"op\":\"nonsense\"").await;
    assert!(matches!(reply, Some(FriendReply::Error { .. })), "{reply:?}");
    let huge = vec![b'x'; 64 * 1024];
    let reply = raw_call(&b.endpoint, a.id(), &huge).await;
    assert!(!matches!(reply, Some(FriendReply::Received | FriendReply::Ack)));
    assert!(a.friends.view().incoming.is_empty());
}

#[tokio::test]
async fn friends_show_as_online_once_reached() {
    let (_lookup, a, b) = pair("presence").await;
    befriend(&a, &b).await;
    until("A sees B online", || {
        a.friends.view().friends.iter().any(|f| f.presence == Presence::Online)
    })
    .await;
}

#[tokio::test]
async fn ids_are_parsed_from_hex_or_a_room_code_and_checked() {
    let own = SecretKey::from_bytes(&[1; 32]).public();
    let other = SecretKey::from_bytes(&[2; 32]).public();
    assert_eq!(parse_friend_id(&other.to_string(), own).unwrap(), other);
    assert_eq!(parse_friend_id(&noxid_core::room::room_code(&other), own).unwrap(), other);
    assert!(parse_friend_id(&own.to_string(), own).is_err(), "not ourselves");
    assert!(parse_friend_id("hello", own).is_err());
    let invite = invite_from(&SecretKey::from_bytes(&[2; 32]));
    let err = parse_friend_id(&invite, own).unwrap_err().to_string();
    assert!(err.contains("invite"), "{err}");
}

// ------------------------------------------------------------------ rings

#[tokio::test]
async fn a_ring_that_is_answered_hands_over_the_invite() {
    let (_lookup, a, b) = pair("ring-answer").await;
    befriend(&a, &b).await;
    let code = invite_from(&a.key);
    let ring = {
        let friends = a.friends.clone();
        let (to, code) = (b.id(), code.clone());
        tokio::spawn(async move { friends.ring(to, code, Some("Hangout".into())).await })
    };
    until("B is ringing", || !b.friends.view().rings.is_empty()).await;
    assert!(a.friends.view().calling.is_some());
    assert_eq!(b.friends.view().rings[0].room_name.as_deref(), Some("Hangout"));
    let joined = b.friends.answer_ring(a.id(), true).unwrap();
    assert_eq!(joined.as_deref(), Some(code.as_str()));
    assert_eq!(ring.await.unwrap().unwrap(), RingOutcome::Answered);
    until("A is no longer calling", || a.friends.view().calling.is_none()).await;
}

#[tokio::test]
async fn a_declined_ring_says_so() {
    let (_lookup, a, b) = pair("ring-decline").await;
    befriend(&a, &b).await;
    let ring = {
        let friends = a.friends.clone();
        let (to, code) = (b.id(), invite_from(&a.key));
        tokio::spawn(async move { friends.ring(to, code, None).await })
    };
    until("B is ringing", || !b.friends.view().rings.is_empty()).await;
    assert_eq!(b.friends.answer_ring(a.id(), false).unwrap(), None);
    assert_eq!(ring.await.unwrap().unwrap(), RingOutcome::Declined);
}

#[tokio::test]
async fn an_unanswered_ring_times_out_as_a_missed_call() {
    let (_lookup, a, b) = pair("ring-timeout").await;
    befriend(&a, &b).await;
    let outcome = a.friends.ring(b.id(), invite_from(&a.key), None).await.unwrap();
    assert_eq!(outcome, RingOutcome::NoAnswer);
    until("B lists a missed call", || !b.friends.view().missed.is_empty()).await;
    assert!(b.friends.view().rings.is_empty());
}

#[tokio::test]
async fn hanging_up_stops_the_ring() {
    let (_lookup, a, b) = pair("ring-hangup").await;
    befriend(&a, &b).await;
    let ring = {
        let friends = a.friends.clone();
        let (to, code) = (b.id(), invite_from(&a.key));
        tokio::spawn(async move { friends.ring(to, code, None).await })
    };
    until("B is ringing", || !b.friends.view().rings.is_empty()).await;
    a.friends.hang_up();
    assert_eq!(ring.await.unwrap().unwrap(), RingOutcome::HungUp);
    until("B stops ringing", || b.friends.view().rings.is_empty()).await;
    until("and lists a missed call", || !b.friends.view().missed.is_empty()).await;
}

#[tokio::test]
async fn only_friends_can_ring() {
    let (_lookup, a, b) = pair("ring-stranger").await;
    assert!(a.friends.ring(b.id(), invite_from(&a.key), None).await.is_err(), "not friends locally");
    let msg = FriendMsg::Ring {
        call_id: RequestId::random(),
        code: invite_from(&a.key),
        room_name: None,
    };
    let reply = raw_call(&a.endpoint, b.id(), &serde_json::to_vec(&msg).unwrap()).await;
    assert_eq!(reply, Some(FriendReply::Unknown));
    assert!(b.friends.view().rings.is_empty());
}

#[tokio::test]
async fn a_ring_for_someone_elses_room_is_refused() {
    let (_lookup, a, b) = pair("ring-relay").await;
    befriend(&a, &b).await;
    let elsewhere = invite_from(&SecretKey::from_bytes(&[9; 32]));
    let msg = FriendMsg::Ring {
        call_id: RequestId::random(),
        code: elsewhere,
        room_name: None,
    };
    let reply = raw_call(&a.endpoint, b.id(), &serde_json::to_vec(&msg).unwrap()).await;
    assert!(matches!(reply, Some(FriendReply::Error { .. })), "{reply:?}");
    assert!(b.friends.view().rings.is_empty());
}
