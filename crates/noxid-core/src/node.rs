//! The one handle a frontend holds: commands go in as method calls, and state
//! comes out as a [`Snapshot`] that is rebuilt whenever something changes.
//!
//! The CLI and the Tauri app both drive this, so neither carries any call logic
//! of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use crate::avatar::{self, AVATAR_ALPN, AvatarHandler, Avatars};
use crate::files::{self, FILES_ALPN, FilesHandler, TransferView, Transfers};
use crate::audio::{self, SystemAudio, Voice};
use crate::gate::MemberGate;
use crate::media::{self, CameraSource, Framerate, Mic, Outgoing, Quality, Screen};
use crate::room::{self, Code, Member, Room};

/// How long a node waits for a relay at startup before carrying on. Until it
/// has one, its address is not published and nobody can dial it.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many knocks may wait for an answer at once. Anyone holding the room code
/// can knock, so this bounds what they can make the coordinator hold open.
const MAX_PENDING_KNOCKS: usize = 8;

/// The loudest a participant's voice can be turned up, as a gain.
pub const MAX_VOLUME: f32 = 2.0;

/// The loudest a share's sound can be turned up. Not above 1.0: the PC's sound
/// arrives at full scale already, and playback cannot amplify.
pub const MAX_SCREEN_VOLUME: f32 = 1.0;

/// How the receiver picks a participant's screen share rendition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "quality", rename_all = "lowercase")]
pub enum QualityMode {
    /// Follow the link: iroh-live's adaptation switches as conditions change.
    /// The default until the frontend picks: the app holds the share on stage
    /// at the top rung instead.
    #[default]
    Auto,
    /// Hold one rendition whatever the link does.
    Pinned(Quality),
}

/// Which of a participant's sounds a volume applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Voice,
    Screen,
}

/// How loud we hear one participant. Local to this node: nobody else is told.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct PeerAudio {
    pub voice_volume: f32,
    pub voice_muted: bool,
    pub screen_volume: f32,
    pub screen_muted: bool,
}

impl Default for PeerAudio {
    fn default() -> Self {
        Self {
            voice_volume: 1.0,
            voice_muted: false,
            screen_volume: 1.0,
            screen_muted: false,
        }
    }
}

impl PeerAudio {
    pub fn voice_gain(&self) -> f32 {
        if self.voice_muted { 0.0 } else { self.voice_volume }
    }

    pub fn screen_gain(&self) -> f32 {
        if self.screen_muted { 0.0 } else { self.screen_volume }
    }
}

/// Everything a frontend shows, in one serializable value.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub id: String,
    pub name: String,
    /// Our profile picture's hash, to fetch with `avatar`.
    pub avatar: Option<String>,
    pub room: Option<RoomView>,
    pub mic_on: bool,
    /// Hearing nobody (and, while it lasts, mic off).
    pub deafened: bool,
    pub sharing: bool,
    pub max_quality: Quality,
    /// Pictures per second a share sends.
    pub framerate: Framerate,
    /// The top rung's bitrate in bits per second, if one was chosen; `None`
    /// means `default_bitrate`.
    pub bitrate: Option<u64>,
    /// What the top rung is sent at by default, for the chosen resolution and
    /// framerate.
    pub default_bitrate: u64,
    /// People waiting at the door. Only ever non-empty on the coordinator.
    pub knocks: Vec<KnockView>,
    /// Chosen devices, `None` meaning the system default.
    pub mic_device: Option<String>,
    pub output_device: Option<String>,
    /// Whether a share includes the PC's sound.
    pub screen_audio: bool,
    pub screen_audio_supported: bool,
    /// Whether sharing one window sends that window's sound alone, rather
    /// than everything the PC plays.
    pub window_audio_supported: bool,
    /// Whether the share in progress is of one window.
    pub sharing_window: bool,
    /// Whether our camera is on, and which one, by its id from the camera list.
    pub camera_on: bool,
    pub camera_device: Option<String>,
    /// Files being offered, sent or received, and recently finished ones.
    pub transfers: Vec<TransferView>,
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
    /// Their picture's hash, once we have a copy to show.
    pub avatar: Option<String>,
    pub is_self: bool,
    pub is_coordinator: bool,
    #[serde(flatten)]
    pub peer: PeerView,
    #[serde(flatten)]
    pub audio: PeerAudio,
}

/// What the mesh knows about one remote participant.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PeerView {
    pub connected: bool,
    pub has_audio: bool,
    pub has_video: bool,
    /// Whether their share carries the PC's sound.
    pub has_screen_audio: bool,
    /// Whether their camera is on.
    pub has_camera: bool,
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
    pub mic_device: Option<String>,
    pub output_device: Option<String>,
}

/// Someone waiting at the door, and the way to answer them.
pub(crate) struct PendingKnock {
    pub name: String,
    pub reply: oneshot::Sender<bool>,
}

/// The local half of the call: what we publish.
struct Local {
    /// Our voice. Exists while we are in a room.
    voice: Option<LocalBroadcast>,
    /// Our screen share. Exists while we share.
    screen_broadcast: Option<LocalBroadcast>,
    /// Our camera. Exists while it is on in a room.
    camera_broadcast: Option<LocalBroadcast>,
    /// The camera wanted on. Kept outside a room too, like the screen, so it
    /// starts when the next room does.
    camera: Option<CameraSource>,
    mic_on: bool,
    mic_device: Option<String>,
    output_device: Option<String>,
    screen: Option<Screen>,
    /// Counts the shares started, so a share's watcher can tell it has been
    /// replaced (see [`watch_share`]).
    share_generation: u64,
    screen_audio: bool,
    /// The open microphone and the device it was opened on. Open while the
    /// mic is on in a room, or while a mic test is running; it also measures
    /// our own level.
    mic: Option<(Option<String>, Voice)>,
    /// Whether the voice broadcast is carrying `mic` right now.
    mic_published: bool,
    /// Whether the audio settings want a live mic level outside a call.
    mic_test: bool,
    /// Whether the mic was on when we deafened, to put it back afterwards.
    mic_before_deafen: Option<bool>,
}

pub(crate) struct Inner {
    pub endpoint: Endpoint,
    pub live: Live,
    pub mic: Mic,
    name: Mutex<String>,
    /// Our profile picture's hash, if we have one.
    avatar: Mutex<Option<String>>,
    /// Copies of other members' pictures.
    pub(crate) avatars: Mutex<Avatars>,
    pub room: Mutex<Room>,
    local: Mutex<Local>,
    /// What a share sends: resolution, framerate, bitrate.
    send: Mutex<Outgoing>,
    pub knocks: Mutex<HashMap<String, PendingKnock>>,
    /// Filled in by the mesh, read when building snapshots.
    pub peers: Mutex<HashMap<EndpointId, PeerView>>,
    /// Each participant's latest voice level, refreshed fast for the speaking
    /// indicator. Kept out of the snapshot so it can update without a redraw of
    /// everything else.
    pub levels: Mutex<HashMap<EndpointId, f32>>,
    /// Decoded screen share frames per participant, for a renderer.
    pub tiles: Mutex<HashMap<EndpointId, Arc<FrameReceiver<Frame>>>>,
    /// Decoded camera frames per participant.
    pub camera_tiles: Mutex<HashMap<EndpointId, Arc<FrameReceiver<Frame>>>>,
    /// Receiver-side rendition choice per participant.
    pub quality_modes: Mutex<HashMap<EndpointId, QualityMode>>,
    /// How loud we hear each participant.
    pub audio_prefs: Mutex<HashMap<EndpointId, PeerAudio>>,
    /// Hear nobody, whatever their own volume says.
    pub deafened: std::sync::atomic::AtomicBool,
    /// File transfers, in and out.
    pub(crate) transfers: Transfers,
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

        // Open the output on the chosen device before anything plays, so the
        // first voice does not start on the default and then jump.
        if config.output_device.is_some()
            && let Err(e) = audio::set_output(config.output_device.clone()).await
        {
            warn!("could not open the chosen output device, using the default: {e:#}");
        }

        let live = Live::builder(endpoint.clone()).spawn();
        let (roster_tx, roster_rx) = watch::channel(Vec::new());
        let inner = Arc::new(Inner {
            endpoint: endpoint.clone(),
            live: live.clone(),
            mic: config.mic,
            name: Mutex::new(room::display_name(&config.name, &id.to_string())),
            avatar: Mutex::new(avatar::load_own()),
            avatars: Mutex::new(Avatars::default()),
            room: Mutex::new(Room::default()),
            local: Mutex::new(Local {
                voice: None,
                screen_broadcast: None,
                camera_broadcast: None,
                camera: None,
                mic_on: !config.start_muted,
                mic_device: config.mic_device,
                output_device: config.output_device,
                screen: None,
                share_generation: 0,
                screen_audio: audio::system_audio_supported(),
                mic: None,
                mic_published: false,
                mic_test: false,
                mic_before_deafen: None,
            }),
            send: Mutex::new(Outgoing::default()),
            knocks: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            levels: Mutex::new(HashMap::new()),
            tiles: Mutex::new(HashMap::new()),
            camera_tiles: Mutex::new(HashMap::new()),
            quality_modes: Mutex::new(HashMap::new()),
            audio_prefs: Mutex::new(HashMap::new()),
            deafened: std::sync::atomic::AtomicBool::new(false),
            transfers: Transfers::default(),
            roster_tx,
            changed: watch::channel(0).0,
            notices: broadcast::channel(16).0,
        });

        // Media is only ever served to members: every MoQ ALPN goes through the
        // gate, never straight to iroh-live's handler, which accepts anyone.
        let mut router = Router::builder(endpoint.clone())
            .accept(
                ADMIN_ALPN,
                AdminHandler {
                    inner: inner.clone(),
                },
            )
            .accept(
                FILES_ALPN,
                FilesHandler {
                    inner: inner.clone(),
                },
            )
            .accept(
                AVATAR_ALPN,
                AvatarHandler {
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
        tokio::spawn(avatar::run(inner.clone()));

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

    /// Voice levels in `0.0..=1.0` by endpoint id, ours included while the mic
    /// is on. Cheap enough to poll several times a second.
    pub fn levels(&self) -> HashMap<String, f32> {
        let mut levels: HashMap<String, f32> = self
            .inner
            .levels
            .lock()
            .unwrap()
            .iter()
            .map(|(id, level)| (id.to_string(), *level))
            .collect();
        if let Some((_, mic)) = &self.inner.local.lock().unwrap().mic {
            levels.insert(self.id().to_string(), mic.level());
        }
        levels
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

    /// Sets our profile picture to `bytes` (an encoded image, see `avatar`),
    /// or removes it with `None`. Like the name, a member's new picture
    /// reaches others the next time it joins; the coordinator's at once.
    pub fn set_avatar(&self, bytes: Option<&[u8]>) -> Result<()> {
        let hash = match bytes {
            Some(bytes) => Some(avatar::save_own(bytes)?),
            None => {
                avatar::clear_own()?;
                None
            }
        };
        *self.inner.avatar.lock().unwrap() = hash.clone();
        let own_id = self.id().to_string();
        let mut room = self.inner.room.lock().unwrap();
        room.set_avatar(&own_id, hash.as_deref());
        if room.is_coordinator(&own_id) {
            drop(room);
            self.inner.roster_changed(None);
        } else {
            drop(room);
            self.inner.bump();
        }
        Ok(())
    }

    /// The picture with `hash`, ours or a member's, if we have it.
    pub fn avatar(&self, hash: &str) -> Option<Vec<u8>> {
        let own = self.inner.own_avatar();
        let known = own.as_deref() == Some(hash) || self.inner.avatars.lock().unwrap().is_ready(hash);
        known.then(|| avatar::read(own.as_deref(), hash)).flatten()
    }

    /// Creates a room with this node as its coordinator, and joins its call.
    pub async fn create(&self, name: Option<String>) -> Result<()> {
        self.leave().await;
        let own_id = self.id().to_string();
        let own_name = self.inner.name();
        {
            let mut room = self.inner.room.lock().unwrap();
            room.create(name, &own_id, &own_name);
            room.set_avatar(&own_id, self.inner.own_avatar().as_deref());
        }
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

        let own_avatar = self.inner.own_avatar();
        let admission = admin::join(&self.inner.endpoint, code, self.inner.name(), own_avatar.clone()).await?;
        let own_id = self.id().to_string();
        {
            let mut room = self.inner.room.lock().unwrap();
            room.set_joined(
                &code.coordinator().to_string(),
                &own_id,
                &self.inner.name(),
                admission.room_name,
                admission.members,
            );
            room.set_avatar(&own_id, own_avatar.as_deref());
        }
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
            join_all(notices).await;
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
        // Turning the mic on while deafened undeafens, as in other voice apps:
        // talking to people you cannot hear is never what was meant.
        if on && self.inner.deafened.swap(false, std::sync::atomic::Ordering::Relaxed) {
            local.mic_before_deafen = None;
        }
        self.inner.apply_mic(&mut local);
        drop(local);
        self.inner.bump();
    }

    /// Stops hearing everyone (voices and shared sound alike) and turns the
    /// mic off; undeafening puts the mic back the way it was.
    pub fn set_deafened(&self, on: bool) {
        use std::sync::atomic::Ordering;
        let mut local = self.inner.local.lock().unwrap();
        let was = self.inner.deafened.swap(on, Ordering::Relaxed);
        match (was, on) {
            (false, true) => {
                local.mic_before_deafen = Some(local.mic_on);
                local.mic_on = false;
            }
            (true, false) => {
                local.mic_on = local.mic_before_deafen.take().unwrap_or(local.mic_on);
            }
            _ => {}
        }
        self.inner.apply_mic(&mut local);
        drop(local);
        self.inner.bump();
    }

    /// Chooses the microphone (`None` for the system default), switching to it
    /// at once if it is live.
    pub fn set_mic_device(&self, device: Option<String>) {
        let mut local = self.inner.local.lock().unwrap();
        local.mic_device = device;
        self.inner.apply_mic(&mut local);
        drop(local);
        self.inner.bump();
    }

    /// Runs the mic level meter outside a call, so the audio settings can show
    /// it while choosing a microphone and a sensitivity.
    pub fn set_mic_test(&self, on: bool) {
        let mut local = self.inner.local.lock().unwrap();
        local.mic_test = on;
        self.inner.apply_mic(&mut local);
    }

    /// Chooses where everyone is heard (`None` for the system default).
    pub async fn set_output_device(&self, device: Option<String>) -> Result<()> {
        audio::set_output(device.clone()).await?;
        self.inner.local.lock().unwrap().output_device = device;
        self.inner.bump();
        Ok(())
    }

    /// Starts sharing `screen`, or stops sharing with `None`. Switching from
    /// one screen or window to another mid-share keeps the same broadcast,
    /// so viewers carry on watching instead of losing the share and finding
    /// it again.
    pub fn share_screen(&self, screen: Option<Screen>) -> Result<()> {
        let mut local = self.inner.local.lock().unwrap();
        let switching = screen.is_some() && local.screen_broadcast.is_some();
        // A window's share carries that window's sound alone, so moving to
        // another window (or to or from a display) moves the sound too.
        let sound_moves = switching
            && local.screen.as_ref().and_then(Screen::window)
                != screen.as_ref().and_then(Screen::window);
        let watch = screen.is_some();
        local.screen = screen;
        local.share_generation += 1;
        if watch {
            tokio::spawn(watch_share(self.inner.clone(), local.share_generation));
        }
        if sound_moves
            && local.screen_audio
            && let Some(broadcast) = &local.screen_broadcast
        {
            self.inner
                .apply_screen_audio(broadcast, true, local.screen.as_ref());
        }
        drop(local);
        if switching {
            return self.inner.restart_share();
        }
        let mut local = self.inner.local.lock().unwrap();
        let result = if local.voice.is_some() {
            self.inner.apply_screen(&mut local)
        } else {
            Ok(())
        };
        drop(local);
        self.inner.bump();
        result
    }

    /// Turns `camera` on (see [`media::resolve_camera`]), or the camera off
    /// with `None`. Switching cameras keeps the same broadcast, so viewers keep
    /// watching.
    pub fn share_camera(&self, camera: Option<CameraSource>) -> Result<()> {
        let mut local = self.inner.local.lock().unwrap();
        local.camera = camera;
        let result = if local.voice.is_some() {
            self.inner.apply_camera(&mut local)
        } else {
            Ok(())
        };
        if result.is_err() {
            // Nothing is sending, so nothing should say it is.
            local.camera = None;
            if let Some(broadcast) = local.camera_broadcast.take() {
                finish_later(broadcast);
            }
        }
        drop(local);
        self.inner.bump();
        result
    }

    /// Whether a share carries the PC's sound. Applies to a share in progress
    /// immediately.
    pub fn set_screen_audio(&self, on: bool) {
        let mut local = self.inner.local.lock().unwrap();
        local.screen_audio = on && audio::system_audio_supported();
        if let Some(broadcast) = &local.screen_broadcast {
            self.inner
                .apply_screen_audio(broadcast, local.screen_audio, local.screen.as_ref());
        }
        drop(local);
        self.inner.bump();
    }

    /// Caps what this node publishes. Restarts a share in progress, since the
    /// ladder is fixed when a share starts. The bitrate goes back to the new
    /// resolution's default: a number picked for 1080p makes no sense at 360p.
    pub fn set_max_quality(&self, max: Quality) -> Result<()> {
        {
            let mut send = self.inner.send.lock().unwrap();
            send.max = max;
            send.bitrate = None;
        }
        self.inner.restart_share()
    }

    /// Sets how many pictures a second a share sends. Restarts a share in
    /// progress, like [`set_max_quality`](Self::set_max_quality).
    pub fn set_framerate(&self, rate: Framerate) -> Result<()> {
        self.inner.send.lock().unwrap().rate = rate;
        self.inner.restart_share()
    }

    /// Sets the top rung's bitrate in bits per second, clamped to
    /// `MIN_BITRATE..=MAX_BITRATE`, or back to the default with `None`.
    pub fn set_bitrate(&self, bitrate: Option<u64>) -> Result<()> {
        let bitrate = bitrate.map(|b| b.clamp(media::MIN_BITRATE, media::MAX_BITRATE));
        self.inner.send.lock().unwrap().bitrate = bitrate;
        self.inner.restart_share()
    }

    /// Chooses how a participant's screen share is received.
    pub fn set_peer_quality(&self, id: EndpointId, mode: QualityMode) {
        self.inner.quality_modes.lock().unwrap().insert(id, mode);
        self.inner.bump();
    }

    /// Sets how loud we hear one of a participant's sounds. `volume` is a gain,
    /// clamped to `MAX_VOLUME` for a voice and `MAX_SCREEN_VOLUME` for a
    /// share; muting keeps the volume for unmuting.
    pub fn set_peer_audio(&self, id: EndpointId, stream: Stream, volume: f32, muted: bool) {
        let max = match stream {
            Stream::Voice => MAX_VOLUME,
            Stream::Screen => MAX_SCREEN_VOLUME,
        };
        let volume = volume.clamp(0.0, max);
        let mut prefs = self.inner.audio_prefs.lock().unwrap();
        let entry = prefs.entry(id).or_default();
        match stream {
            Stream::Voice => {
                entry.voice_volume = volume;
                entry.voice_muted = muted;
            }
            Stream::Screen => {
                entry.screen_volume = volume;
                entry.screen_muted = muted;
            }
        }
        drop(prefs);
        self.inner.bump();
    }

    /// The latest-frame slot of a participant's screen share, while they share.
    pub fn frames(&self, id: &EndpointId) -> Option<Arc<FrameReceiver<Frame>>> {
        self.inner.tiles.lock().unwrap().get(id).cloned()
    }

    /// Offers the file at `path` to member `peer`, and sends it once they
    /// accept. Returns the transfer's id; its progress is in the snapshot.
    pub async fn send_file(&self, peer: &str, path: std::path::PathBuf) -> Result<u64> {
        let peer: EndpointId = peer.parse().context("not a member id")?;
        files::send(self.inner.clone(), peer, path).await
    }

    /// Accepts or declines a file someone offered us.
    pub fn answer_transfer(&self, id: u64, accept: bool) -> Result<()> {
        let result = self.inner.transfers.answer(id, accept);
        self.inner.bump();
        result
    }

    /// Stops a transfer, sending or receiving, at any stage.
    pub fn cancel_transfer(&self, id: u64) {
        self.inner.transfers.cancel(id);
        self.inner.bump();
    }

    /// Forgets a finished transfer.
    pub fn dismiss_transfer(&self, id: u64) {
        self.inner.transfers.dismiss(id);
        self.inner.bump();
    }

    /// Where a received file was saved.
    pub fn transfer_path(&self, id: u64) -> Option<std::path::PathBuf> {
        self.inner.transfers.saved_to(id)
    }

    /// Decoded camera frames for one participant, while their camera is on.
    pub fn camera_frames(&self, id: &EndpointId) -> Option<Arc<FrameReceiver<Frame>>> {
        self.inner.camera_tiles.lock().unwrap().get(id).cloned()
    }

    /// What our own camera sends, for a self view.
    pub fn camera_preview(&self) -> Option<Arc<FrameReceiver<Arc<Frame>>>> {
        self.inner
            .local
            .lock()
            .unwrap()
            .camera_broadcast
            .as_ref()
            .and_then(|b| b.preview())
    }

    /// What our own screen share looks like, for a self view.
    pub fn preview(&self) -> Option<Arc<FrameReceiver<Arc<Frame>>>> {
        self.inner
            .local
            .lock()
            .unwrap()
            .screen_broadcast
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
async fn join_all<F: std::future::Future<Output = ()> + Send + 'static>(
    futures: impl IntoIterator<Item = F>,
) {
    let mut set = tokio::task::JoinSet::new();
    for f in futures {
        set.spawn(f);
    }
    while set.join_next().await.is_some() {}
}

/// How often a share is checked on: for its window having closed, and for its
/// capture having stopped.
const SHARE_CHECK: Duration = Duration::from_secs(1);

/// The longest a share waits between restarts of a capture that keeps stopping.
const RESTART_MAX: Duration = Duration::from_secs(30);

/// Looks after one share until it is replaced or ended.
///
/// A shared window that is closed ends the share (see [`media::window_exists`]).
///
/// A capture that stops is restarted at the source's new size. That is what a
/// capture does when what it shows changes size: a game switching resolution
/// or going fullscreen, a window resized, a display's mode changed. It ends so
/// it can be reopened at the new size, and nothing below us reopens it, so
/// without this the share would go on sending sound and no picture.
async fn watch_share(inner: Arc<Inner>, generation: u64) {
    let current = |inner: &Inner| {
        let local = inner.local.lock().unwrap();
        if local.share_generation != generation {
            return None;
        }
        let stopped = local
            .screen_broadcast
            .as_ref()
            .and_then(LocalBroadcast::preview)
            .is_some_and(|preview| preview.is_closed());
        local.screen.clone().map(|screen| (screen, stopped))
    };
    let mut restarted: Option<Instant> = None;
    let mut backoff = SHARE_CHECK;
    loop {
        tokio::time::sleep(SHARE_CHECK).await;
        let Some((screen, stopped)) = current(&inner) else { return };

        if let Some(window) = screen.window()
            && !media::window_exists(window).await
        {
            let mut local = inner.local.lock().unwrap();
            // Checked again under the lock: the share may have moved meanwhile.
            if local.share_generation != generation {
                return;
            }
            local.screen = None;
            if let Err(e) = inner.apply_screen(&mut local) {
                warn!("ending the share of a closed window: {e:#}");
            }
            drop(local);
            inner.bump();
            inner.notice(Notice::Error(
                "The window you were sharing was closed, so the share ended.".into(),
            ));
            return;
        }

        if !stopped || restarted.is_some_and(|at| at.elapsed() < backoff) {
            continue;
        }
        let screen = media::refreshed(&screen).await;
        let mut local = inner.local.lock().unwrap();
        if local.share_generation != generation {
            return;
        }
        if let Some(broadcast) = &local.screen_broadcast {
            info!(size = %screen.size(), "the shared capture stopped, restarting it");
            let send = *inner.send.lock().unwrap();
            if let Err(e) = media::share_screen(broadcast, &screen, send) {
                warn!("restarting the share: {e:#}");
            }
        }
        local.screen = Some(screen);
        drop(local);
        inner.bump();
        // Restarts close together mean a capture that cannot stay open, so
        // they spread out; one after a quiet spell is just the next resize.
        backoff = match restarted {
            Some(at) if at.elapsed() < RESTART_MAX => (backoff * 2).min(RESTART_MAX),
            _ => SHARE_CHECK,
        };
        restarted = Some(Instant::now());
    }
}

/// Ends a broadcast off the caller's thread: subscribers see a clean close
/// rather than a dropped one.
fn finish_later(broadcast: LocalBroadcast) {
    tokio::spawn(async move { broadcast.finish().await });
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

    pub fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn own_avatar(&self) -> Option<String> {
        self.avatar.lock().unwrap().clone()
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

    /// Starts publishing for the room just entered: our voice, and a share if
    /// one was chosen before.
    fn start_local(&self) -> Result<()> {
        let path = crate::voice_path(&self.own_id());
        let mut local = self.local.lock().unwrap();
        let voice = self
            .live
            .publish(path)
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        local.voice = Some(voice);
        local.mic_published = false;
        self.apply_mic(&mut local);
        // A camera that will not open costs the camera, not the room.
        if let Err(e) = self.apply_camera(&mut local) {
            warn!("camera unavailable: {e:#}");
            local.camera = None;
            self.notice(Notice::Error(format!("The camera could not be opened: {e:#}")));
        }
        self.apply_screen(&mut local)
    }

    /// Makes the camera broadcast match `local.camera`: started, moved to
    /// another camera, or ended.
    fn apply_camera(&self, local: &mut Local) -> Result<()> {
        let Some(camera) = local.camera.clone() else {
            if let Some(previous) = local.camera_broadcast.take() {
                finish_later(previous);
            }
            return Ok(());
        };
        if let Some(broadcast) = &local.camera_broadcast {
            return media::share_camera(broadcast, &camera);
        }
        let path = crate::camera_path(&self.own_id());
        let broadcast = self
            .live
            .publish(path)
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        media::share_camera(&broadcast, &camera)?;
        local.camera_broadcast = Some(broadcast);
        Ok(())
    }

    /// Makes the open microphone and what the voice broadcast carries match
    /// `local`.
    fn apply_mic(&self, local: &mut Local) {
        let live = local.voice.is_some() && local.mic_on;
        match self.mic {
            Mic::Device => {
                // Kept open across changes that leave the device alone, so the
                // gain it has settled on is not thrown away.
                let wanted = live || local.mic_test;
                let same_device =
                    matches!(&local.mic, Some((device, _)) if *device == local.mic_device);
                if !wanted || !same_device {
                    local.mic = None;
                    local.mic_published = false;
                }
                if wanted && local.mic.is_none() {
                    match Voice::start(local.mic_device.as_deref()) {
                        Ok(mic) => local.mic = Some((local.mic_device.clone(), mic)),
                        Err(e) => {
                            warn!("microphone unavailable: {e:#}");
                            if live {
                                self.notice(Notice::Error(format!(
                                    "The microphone could not be opened: {e:#}"
                                )));
                            }
                        }
                    }
                }
                let Some(voice) = &local.voice else {
                    local.mic_published = false;
                    return;
                };
                match (&local.mic, live) {
                    (Some((_, mic)), true) => {
                        if !local.mic_published {
                            let clock = *voice.clock();
                            voice.audio().set(mic.source(move || clock.now().into()));
                            local.mic_published = true;
                        }
                    }
                    (mic, _) => {
                        if let Some((_, mic)) = mic {
                            mic.detach();
                        }
                        media::mute_mic(voice);
                        local.mic_published = false;
                    }
                }
            }
            #[cfg(feature = "test-source")]
            Mic::Test => {
                if let Some(voice) = &local.voice {
                    match live {
                        true => media::set_mic(voice, Mic::Test, None),
                        false => media::mute_mic(voice),
                    }
                }
            }
        }
    }

    /// Applies the current send settings to a share in progress.
    fn restart_share(&self) -> Result<()> {
        let local = self.local.lock().unwrap();
        if let (Some(broadcast), Some(screen)) = (&local.screen_broadcast, &local.screen) {
            let send = *self.send.lock().unwrap();
            media::share_screen(broadcast, screen, send)?;
        }
        drop(local);
        self.bump();
        Ok(())
    }

    /// Makes the screen broadcast match `local.screen`: started, restarted on
    /// another screen, or ended.
    fn apply_screen(&self, local: &mut Local) -> Result<()> {
        if let Some(previous) = local.screen_broadcast.take() {
            finish_later(previous);
        }
        let Some(screen) = local.screen.clone() else {
            return Ok(());
        };
        let path = crate::screen_path(&self.own_id());
        let broadcast = self
            .live
            .publish(path)
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        let send = *self.send.lock().unwrap();
        media::share_screen(&broadcast, &screen, send)?;
        self.apply_screen_audio(&broadcast, local.screen_audio, Some(&screen));
        local.screen_broadcast = Some(broadcast);
        Ok(())
    }

    /// Adds or removes the PC's sound on a share: only the window's own
    /// sound when `screen` is a window, the whole PC's otherwise. A failure
    /// costs the sound, never the picture, and is reported once.
    fn apply_screen_audio(&self, broadcast: &LocalBroadcast, on: bool, screen: Option<&Screen>) {
        if !on {
            broadcast.audio().clear();
            return;
        }
        let clock = *broadcast.clock();
        let window = screen.and_then(Screen::window);
        match SystemAudio::start(window, move || clock.now().into()) {
            Ok(source) => broadcast.audio().set(source),
            Err(e) => {
                warn!("sharing without sound: {e:#}");
                self.notice(Notice::Error(format!("The share has no sound: {e:#}")));
            }
        }
    }

    /// Drops out of the room locally: stops publishing, forgets the roster,
    /// and lets the mesh close every link. Telling others is the caller's job.
    pub async fn leave_local(&self) {
        // Transfers are between members; leaving ends them.
        files::stop_all(self);
        let (voice, screen, camera) = {
            let mut local = self.local.lock().unwrap();
            local.screen = None;
            local.camera = None;
            (
                local.voice.take(),
                local.screen_broadcast.take(),
                local.camera_broadcast.take(),
            )
        };
        for broadcast in [voice, screen, camera].into_iter().flatten() {
            broadcast.finish().await;
        }
        {
            let mut local = self.local.lock().unwrap();
            self.apply_mic(&mut local);
        }
        self.room.lock().unwrap().leave();
        // Anyone still waiting at the door of a room that no longer exists is
        // turned away.
        self.knocks.lock().unwrap().clear();
        self.quality_modes.lock().unwrap().clear();
        self.audio_prefs.lock().unwrap().clear();
        self.levels.lock().unwrap().clear();
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
        let prefs = self.audio_prefs.lock().unwrap();
        let send = *self.send.lock().unwrap();
        let own_avatar = self.own_avatar();
        let avatars = self.avatars.lock().unwrap();

        let room_view = room.is_active().then(|| {
            let is_coordinator = room.is_coordinator(&own);
            RoomView {
                name: room.name.clone(),
                is_coordinator,
                room_code: is_coordinator.then(|| room::room_code(&own_id)),
                members: room
                    .roster()
                    .into_iter()
                    .map(|Member { id, name, avatar }| {
                        let parsed: Option<EndpointId> = id.parse().ok();
                        let mut peer = parsed
                            .and_then(|p| peers.get(&p).cloned())
                            .unwrap_or_default();
                        peer.quality = parsed
                            .and_then(|p| modes.get(&p).copied())
                            .unwrap_or_default();
                        let audio = parsed
                            .and_then(|p| prefs.get(&p).copied())
                            .unwrap_or_default();
                        // Only a picture we can hand over: ours, or a copy.
                        let avatar = avatar.filter(|a| {
                            own_avatar.as_deref() == Some(a.as_str()) || avatars.is_ready(a)
                        });
                        MemberView {
                            avatar,
                            is_self: id == own,
                            is_coordinator: room.coordinator_id.as_deref() == Some(id.as_str()),
                            id,
                            name,
                            peer,
                            audio,
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
            avatar: own_avatar,
            room: room_view,
            mic_on: local.mic_on,
            deafened: self.deafened.load(std::sync::atomic::Ordering::Relaxed),
            sharing: local.screen.is_some(),
            max_quality: send.max,
            framerate: send.rate,
            bitrate: send.bitrate,
            default_bitrate: send.default_bitrate(send.max),
            knocks,
            mic_device: local.mic_device.clone(),
            output_device: local.output_device.clone(),
            screen_audio: local.screen_audio,
            screen_audio_supported: audio::system_audio_supported(),
            window_audio_supported: audio::window_audio_supported(),
            sharing_window: local.screen.as_ref().and_then(Screen::window).is_some(),
            camera_on: local.camera.is_some(),
            camera_device: local.camera.as_ref().map(|c| c.id.clone()),
            transfers: self.transfers.views(),
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
