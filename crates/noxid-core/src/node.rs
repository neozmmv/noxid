//! The one handle a frontend holds: commands go in as method calls, and state
//! comes out as a [`Snapshot`] that is rebuilt whenever something changes.
//!
//! The CLI and the Tauri app both drive this, so neither carries any call logic
//! of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointId, SecretKey, endpoint::presets, protocol::Router};
use iroh_live::{
    Live,
    media::{frame_channel::FrameReceiver, publish::LocalBroadcast, video::Frame},
    util::{LanPresence, transport_config, with_mdns},
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot, watch};
use tracing::{info, warn};

use crate::admin::{self, ADMIN_ALPN, AdminHandler};
use crate::gate::MemberGate;
use crate::media::{self, Mic, Quality, Screen};
use crate::room::{self, Code, Member, Room};

/// How long a node waits for a relay at startup before carrying on. Until it
/// has one, its address is not published and nobody can dial it.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many knocks may wait for an answer at once. Anyone holding the room code
/// can knock, so this bounds what they can make the coordinator hold open.
const MAX_PENDING_KNOCKS: usize = 8;

/// How the receiver picks a participant's screen share rendition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "quality", rename_all = "lowercase")]
pub enum QualityMode {
    /// Follow the link: iroh-live's adaptation switches as conditions change.
    #[default]
    Auto,
    /// Hold one rendition whatever the link does.
    Pinned(Quality),
}

/// Everything a frontend shows, in one serializable value.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    pub room: Option<RoomView>,
    pub mic_on: bool,
    pub sharing: bool,
    pub max_quality: Quality,
    /// People waiting at the door. Only ever non-empty on the coordinator.
    pub knocks: Vec<KnockView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoomView {
    pub name: Option<String>,
    pub is_coordinator: bool,
    /// The code anyone can knock with. Shown on the coordinator only, since
    /// only it can answer.
    pub room_code: Option<String>,
    pub members: Vec<MemberView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub is_self: bool,
    pub is_coordinator: bool,
    #[serde(flatten)]
    pub peer: PeerView,
}

/// What the mesh knows about one remote participant.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PeerView {
    pub connected: bool,
    pub has_audio: bool,
    pub has_video: bool,
    /// Recent audio peak in `0.0..=1.0`, for a speaking indicator.
    pub level: f32,
    pub rendition: Option<String>,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub rtt_ms: Option<u64>,
    pub quality: QualityMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct KnockView {
    pub id: String,
    pub name: String,
}

/// Something that happened, as opposed to state that is: a frontend shows it
/// once rather than rendering it from the snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "snake_case")]
pub enum Notice {
    Kicked,
    RoomClosed,
    Error(String),
}

/// What [`Node::spawn`] needs.
#[derive(Debug)]
pub struct NodeConfig {
    pub secret_key: SecretKey,
    pub name: String,
    pub mic: Mic,
    pub start_muted: bool,
}

/// Someone waiting at the door, and the way to answer them.
pub(crate) struct PendingKnock {
    pub name: String,
    pub reply: oneshot::Sender<bool>,
}

/// The local half of the call: what we publish.
#[derive(Default)]
struct Local {
    broadcast: Option<LocalBroadcast>,
    mic_on: bool,
    screen: Option<Screen>,
}

pub(crate) struct Inner {
    pub endpoint: Endpoint,
    pub live: Live,
    pub mic: Mic,
    name: Mutex<String>,
    pub room: Mutex<Room>,
    local: Mutex<Local>,
    max_quality: Mutex<Quality>,
    pub knocks: Mutex<HashMap<String, PendingKnock>>,
    /// Filled in by the mesh, read when building snapshots.
    pub peers: Mutex<HashMap<EndpointId, PeerView>>,
    /// Decoded screen share frames per participant, for a renderer.
    pub tiles: Mutex<HashMap<EndpointId, Arc<FrameReceiver<Frame>>>>,
    /// Receiver-side rendition choice per participant.
    pub quality_modes: Mutex<HashMap<EndpointId, QualityMode>>,
    /// The current roster's ids, which the mesh follows.
    pub roster_tx: watch::Sender<Vec<EndpointId>>,
    /// Bumped on every change, so a frontend knows to read a new snapshot.
    changed: watch::Sender<u64>,
    notices: broadcast::Sender<Notice>,
}

/// A running Noxid node. Cheap to clone.
#[derive(Clone)]
pub struct Node {
    inner: Arc<Inner>,
    _router: Arc<Router>,
}

impl Node {
    pub async fn spawn(config: NodeConfig) -> Result<Self> {
        let builder = Endpoint::builder(presets::N0)
            .transport_config(transport_config())
            .secret_key(config.secret_key);
        let endpoint = with_mdns(builder, LanPresence::Announce).await.bind().await?;
        let id = endpoint.id();
        info!(%id, "endpoint bound");

        let live = Live::builder(endpoint.clone()).spawn();
        let (roster_tx, roster_rx) = watch::channel(Vec::new());
        let inner = Arc::new(Inner {
            endpoint: endpoint.clone(),
            live: live.clone(),
            mic: config.mic,
            name: Mutex::new(room::display_name(&config.name, &id.to_string())),
            room: Mutex::new(Room::default()),
            local: Mutex::new(Local {
                mic_on: !config.start_muted,
                ..Default::default()
            }),
            max_quality: Mutex::new(Quality::High),
            knocks: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            tiles: Mutex::new(HashMap::new()),
            quality_modes: Mutex::new(HashMap::new()),
            roster_tx,
            changed: watch::channel(0).0,
            notices: broadcast::channel(16).0,
        });

        // Media is only ever served to members: every MoQ ALPN goes through the
        // gate, never straight to iroh-live's handler, which accepts anyone.
        let mut router = Router::builder(endpoint.clone()).accept(
            ADMIN_ALPN,
            AdminHandler {
                inner: inner.clone(),
            },
        );
        let gate = MemberGate {
            inner: inner.clone(),
            moq: live.protocol_handler(),
        };
        for alpn in iroh_moq::alpns() {
            router = router.accept(alpn, gate.clone());
        }
        let router = router.spawn();

        tokio::spawn(crate::mesh::run(inner.clone(), roster_rx));
        tokio::spawn(admin::roster_heartbeat(inner.clone()));

        match tokio::time::timeout(ONLINE_TIMEOUT, endpoint.online()).await {
            Ok(()) => info!("online"),
            Err(_) => warn!(
                "no relay after {}s; peers may not be able to reach us yet",
                ONLINE_TIMEOUT.as_secs()
            ),
        }

        Ok(Self {
            inner,
            _router: Arc::new(router),
        })
    }

    pub fn id(&self) -> EndpointId {
        self.inner.endpoint.id()
    }

    /// A watch that ticks whenever [`snapshot`](Self::snapshot) would change.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.inner.changed.subscribe()
    }

    pub fn notices(&self) -> broadcast::Receiver<Notice> {
        self.inner.notices.subscribe()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.snapshot()
    }

    pub fn set_name(&self, name: &str) {
        let name = room::display_name(name, &self.id().to_string());
        *self.inner.name.lock().unwrap() = name.clone();
        let own_id = self.id().to_string();
        let mut room = self.inner.room.lock().unwrap();
        // The coordinator can rename itself in place; a member's new name
        // reaches others the next time it knocks.
        if room.is_coordinator(&own_id) {
            room.admit(&own_id, &name);
            drop(room);
            self.inner.roster_changed(None);
        } else {
            drop(room);
            self.inner.bump();
        }
    }

    /// Creates a room with this node as its coordinator, and joins its call.
    pub async fn create(&self, name: Option<String>) -> Result<()> {
        self.leave().await;
        let own_id = self.id().to_string();
        let own_name = self.inner.name();
        self.inner.room.lock().unwrap().create(name, &own_id, &own_name);
        self.inner.start_local()?;
        self.inner.roster_changed(None);
        Ok(())
    }

    /// Mints a single-use invite. Coordinator only.
    pub fn invite(&self, ttl: Option<chrono::Duration>) -> Result<String> {
        let ttl = ttl.unwrap_or(room::DEFAULT_INVITE_TTL);
        let (code, _) =
            self.inner
                .room
                .lock()
                .unwrap()
                .generate_invite(self.id(), ttl, chrono::Utc::now())?;
        Ok(code)
    }

    /// Joins with an invite, or knocks with a room code and waits for an
    /// answer. Leaves any current room first.
    pub async fn join(&self, code: &str) -> Result<()> {
        let code = Code::decode(code)?;
        if code.coordinator() == self.id() {
            anyhow::bail!("that code is for a room you coordinate");
        }
        self.leave().await;

        let admission = admin::join(&self.inner.endpoint, code, self.inner.name()).await?;
        let own_id = self.id().to_string();
        self.inner.room.lock().unwrap().set_joined(
            &code.coordinator().to_string(),
            &own_id,
            &self.inner.name(),
            admission.room_name,
            admission.members,
        );
        self.inner.start_local()?;
        self.inner.roster_changed(None);
        Ok(())
    }

    /// Leaves the room. A coordinator leaving closes the room for everyone,
    /// since nobody else could admit anyone or keep the roster.
    pub async fn leave(&self) {
        let (coordinator, roster, own_id) = {
            let room = self.inner.room.lock().unwrap();
            if !room.is_active() {
                return;
            }
            (
                room.coordinator_id.clone(),
                room.member_ids(),
                self.id().to_string(),
            )
        };

        let endpoint = &self.inner.endpoint;
        if coordinator.as_deref() == Some(own_id.as_str()) {
            let notices = roster
                .into_iter()
                .filter(|id| *id != self.id())
                .map(|id| admin::notify_closed(endpoint.clone(), id));
            futures_join_all(notices).await;
        } else if let Some(coordinator) = coordinator.and_then(|c| c.parse().ok())
            && let Err(e) = admin::notify_leave(endpoint, coordinator).await
        {
            warn!("could not tell the coordinator we left: {e:#}");
        }

        self.inner.leave_local().await;
    }

    /// Removes a member. Coordinator only.
    pub async fn kick(&self, who: &str) -> Result<()> {
        let own_id = self.id().to_string();
        let member = {
            let mut room = self.inner.room.lock().unwrap();
            if !room.is_coordinator(&own_id) {
                anyhow::bail!("only the coordinator can remove people");
            }
            room.kick(who, &own_id).map_err(|e| anyhow::anyhow!(e))?
        };
        info!(id = %member.id, name = %member.name, "kicked");
        self.inner.roster_changed(Some(&member.id));
        if let Ok(id) = member.id.parse()
            && let Err(e) = admin::notify_kicked(&self.inner.endpoint, id).await
        {
            warn!("could not tell {} they were removed: {e:#}", member.name);
        }
        Ok(())
    }

    /// Answers someone knocking.
    pub fn answer_knock(&self, id: &str, approve: bool) -> Result<()> {
        let knock = self
            .inner
            .knocks
            .lock()
            .unwrap()
            .remove(id)
            .context("nobody with that id is waiting")?;
        // The handler holding the knocker's stream does the admitting, so the
        // roster changes in exactly one place.
        let _ = knock.reply.send(approve);
        self.inner.bump();
        Ok(())
    }

    pub fn set_mic(&self, on: bool) {
        let mut local = self.inner.local.lock().unwrap();
        local.mic_on = on;
        if let Some(broadcast) = &local.broadcast {
            match on {
                true => media::set_mic(broadcast, self.inner.mic),
                false => media::mute_mic(broadcast),
            }
        }
        drop(local);
        self.inner.bump();
    }

    /// Starts sharing `screen`, or stops sharing with `None`.
    pub fn share_screen(&self, screen: Option<Screen>) -> Result<()> {
        let max = *self.inner.max_quality.lock().unwrap();
        let mut local = self.inner.local.lock().unwrap();
        local.screen = screen;
        if let Some(broadcast) = &local.broadcast {
            match &local.screen {
                Some(screen) => media::share_screen(broadcast, screen, max)?,
                None => media::stop_screen(broadcast),
            }
        }
        drop(local);
        self.inner.bump();
        Ok(())
    }

    /// Caps what this node publishes. Restarts a share in progress, since the
    /// ladder is fixed when a share starts.
    pub fn set_max_quality(&self, max: Quality) -> Result<()> {
        *self.inner.max_quality.lock().unwrap() = max;
        let local = self.inner.local.lock().unwrap();
        if let (Some(broadcast), Some(screen)) = (&local.broadcast, &local.screen) {
            media::share_screen(broadcast, screen, max)?;
        }
        drop(local);
        self.inner.bump();
        Ok(())
    }

    /// Chooses how a participant's screen share is received.
    pub fn set_peer_quality(&self, id: EndpointId, mode: QualityMode) {
        self.inner.quality_modes.lock().unwrap().insert(id, mode);
        self.inner.bump();
    }

    /// The latest-frame slot of a participant's screen share, while they share.
    pub fn frames(&self, id: &EndpointId) -> Option<Arc<FrameReceiver<Frame>>> {
        self.inner.tiles.lock().unwrap().get(id).cloned()
    }

    /// What our own screen share looks like, for a self view.
    pub fn preview(&self) -> Option<Arc<FrameReceiver<Arc<Frame>>>> {
        self.inner
            .local
            .lock()
            .unwrap()
            .broadcast
            .as_ref()
            .and_then(|b| b.preview())
    }

    pub async fn shutdown(&self) {
        self.leave().await;
        self.inner.live.shutdown().await;
    }
}

/// Runs futures concurrently and waits for all of them, without pulling in a
/// futures crate for one call site.
async fn futures_join_all<F: std::future::Future<Output = ()> + Send + 'static>(
    futures: impl IntoIterator<Item = F>,
) {
    let mut set = tokio::task::JoinSet::new();
    for f in futures {
        set.spawn(f);
    }
    while set.join_next().await.is_some() {}
}

impl Inner {
    pub fn own_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn name(&self) -> String {
        self.name.lock().unwrap().clone()
    }

    pub fn bump(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn notice(&self, notice: Notice) {
        let _ = self.notices.send(notice);
    }

    /// Call after any change to the roster: the mesh follows it, the frontend
    /// redraws, and on the coordinator every member hears about it (except
    /// `subject`, who learned the outcome from the reply that caused it).
    pub fn roster_changed(&self, subject: Option<&str>) {
        let (ids, is_coordinator) = {
            let room = self.room.lock().unwrap();
            (room.member_ids(), room.is_coordinator(&self.own_id().to_string()))
        };
        self.roster_tx.send_replace(ids);
        if is_coordinator {
            admin::broadcast_roster(self, subject);
        }
        self.bump();
    }

    /// Starts publishing our broadcast for the room just entered.
    fn start_local(&self) -> Result<()> {
        let path = crate::broadcast_path(&self.own_id());
        let max = *self.max_quality.lock().unwrap();
        let mut local = self.local.lock().unwrap();
        let broadcast = self
            .live
            .publish(path)
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        if local.mic_on {
            media::set_mic(&broadcast, self.mic);
        }
        if let Some(screen) = &local.screen {
            media::share_screen(&broadcast, screen, max)?;
        }
        local.broadcast = Some(broadcast);
        Ok(())
    }

    /// Drops out of the room locally: stops publishing, forgets the roster,
    /// and lets the mesh close every link. Telling others is the caller's job.
    pub async fn leave_local(&self) {
        let broadcast = {
            let mut local = self.local.lock().unwrap();
            local.screen = None;
            local.broadcast.take()
        };
        if let Some(broadcast) = broadcast {
            broadcast.finish().await;
        }
        self.room.lock().unwrap().leave();
        // Anyone still waiting at the door of a room that no longer exists is
        // turned away.
        self.knocks.lock().unwrap().clear();
        self.quality_modes.lock().unwrap().clear();
        self.roster_changed(None);
        info!("left the room");
    }

    fn snapshot(&self) -> Snapshot {
        let own_id = self.own_id();
        let own = own_id.to_string();
        let local = self.local.lock().unwrap();
        let room = self.room.lock().unwrap();
        let peers = self.peers.lock().unwrap();
        let modes = self.quality_modes.lock().unwrap();

        let room_view = room.is_active().then(|| {
            let is_coordinator = room.is_coordinator(&own);
            RoomView {
                name: room.name.clone(),
                is_coordinator,
                room_code: is_coordinator.then(|| room::room_code(&own_id)),
                members: room
                    .roster()
                    .into_iter()
                    .map(|Member { id, name }| {
                        let parsed: Option<EndpointId> = id.parse().ok();
                        let mut peer = parsed
                            .and_then(|p| peers.get(&p).cloned())
                            .unwrap_or_default();
                        peer.quality = parsed
                            .and_then(|p| modes.get(&p).copied())
                            .unwrap_or_default();
                        MemberView {
                            is_self: id == own,
                            is_coordinator: room.coordinator_id.as_deref() == Some(id.as_str()),
                            id,
                            name,
                            peer,
                        }
                    })
                    .collect(),
            }
        });

        let mut knocks: Vec<KnockView> = self
            .knocks
            .lock()
            .unwrap()
            .iter()
            .map(|(id, k)| KnockView {
                id: id.clone(),
                name: k.name.clone(),
            })
            .collect();
        knocks.sort_by(|a, b| a.id.cmp(&b.id));

        Snapshot {
            id: own,
            name: self.name(),
            room: room_view,
            mic_on: local.mic_on,
            sharing: local.screen.is_some(),
            max_quality: *self.max_quality.lock().unwrap(),
            knocks,
        }
    }

    /// Registers a knock, refusing when too many are already waiting. A second
    /// knock from the same node replaces its first, which is turned away.
    pub fn add_knock(&self, id: &str, name: String) -> Option<oneshot::Receiver<bool>> {
        let mut knocks = self.knocks.lock().unwrap();
        if knocks.len() >= MAX_PENDING_KNOCKS && !knocks.contains_key(id) {
            return None;
        }
        let (reply, answer) = oneshot::channel();
        if let Some(previous) = knocks.insert(id.to_string(), PendingKnock { name, reply }) {
            let _ = previous.reply.send(false);
        }
        drop(knocks);
        self.bump();
        Some(answer)
    }

    /// Drops a knock that was never answered (it timed out, or the knocker went
    /// away).
    pub fn forget_knock(&self, id: &str) {
        if self.knocks.lock().unwrap().remove(id).is_some() {
            self.bump();
        }
    }
}
