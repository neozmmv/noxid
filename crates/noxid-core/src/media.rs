//! What this node publishes: one broadcast carrying the microphone and,
//! optionally, a screen share.
//!
//! A call is voice first. Joining publishes the microphone (unless muted) and
//! nothing else; sharing a screen is a separate, explicit action that adds a
//! simulcast ladder to the same broadcast and can be stopped again mid-call.
//!
//! Every rung of the ladder is fed by the same capture, and an encoder runs only
//! while some subscriber is watching its rung. That is what makes a ladder
//! affordable in a full mesh: an audience watching thumbnails never pays for
//! the full-resolution encode.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh_live::media::{
    publish::{AudioSource, LocalBroadcast, VideoRendition, VideoSource},
    video::capture,
};
use serde::{Deserialize, Serialize};

/// The video vocabulary a frontend needs to draw what the mesh decodes.
pub use iroh_live::media::frame_channel::FrameReceiver;
pub use iroh_live::media::video::{Color, Frame, Size};

/// How often each rung inserts a keyframe.
///
/// A subscriber draws nothing until the next keyframe, so this bounds how long
/// someone joining waits for the shared screen and how long a rendition switch
/// takes to land. But the encoder runs at a constant bitrate, and a keyframe of
/// a full screen costs many times a normal frame, so every keyframe leaves less
/// for the frames around it. Two seconds (the encoder default) keeps text
/// sharp at a wait nobody minds.
const KEYFRAME_INTERVAL: Duration = Duration::from_secs(2);

/// The size the generated test pattern is painted at.
#[cfg(feature = "test-source")]
fn test_pattern() -> Size {
    Size::new(1280, 720)
}

/// One rung of the ladder. Ordered from smallest to largest, so `max` comparisons
/// read naturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Low,
    Mid,
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Low, Quality::Mid, Quality::High];

    /// The rendition name this rung has in the catalog.
    pub fn rendition(self) -> &'static str {
        match self {
            Quality::Low => "screen/low",
            Quality::Mid => "screen/mid",
            Quality::High => "screen/high",
        }
    }

    pub fn from_rendition(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.rendition() == name)
    }

    /// The encoded height. Taller than a camera ladder would use, because a
    /// shared screen is mostly text and text stops being legible quickly.
    pub fn height(self) -> u32 {
        match self {
            Quality::Low => 360,
            Quality::Mid => 720,
            Quality::High => 1080,
        }
    }

    /// Target bitrate in bits per second, at 30 fps.
    ///
    /// Screens are mostly fine text, which H.264 smears quickly when starved;
    /// these are about what streaming services use for 1080p and 720p desktop
    /// content. In a full mesh the sender uploads one copy per viewer, so the
    /// top rung at 60 fps to five people is about 40 Mbit/s up.
    pub fn bitrate(self) -> u64 {
        match self {
            Quality::Low => 600_000,
            Quality::Mid => 2_500_000,
            Quality::High => 5_000_000,
        }
    }
}

impl std::str::FromStr for Quality {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "low" => Ok(Quality::Low),
            "mid" => Ok(Quality::Mid),
            "high" => Ok(Quality::High),
            other => anyhow::bail!("unknown quality {other:?}; expected low, mid or high"),
        }
    }
}

/// How many pictures a second a share sends: 24, 30 or 60.
///
/// Every rung of the ladder runs at it. 60 is for motion (games, video); 24
/// saves bandwidth and CPU on a slide deck or a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Framerate(u32);

impl Framerate {
    pub const ALL: [u32; 3] = [24, 30, 60];

    pub fn fps(self) -> u32 {
        self.0
    }

    /// The bitrate a rung needs at this rate, relative to 30 fps. Less than
    /// linear: between two frames close in time there is less to encode.
    fn bitrate_scale(self) -> f64 {
        match self.0 {
            24 => 0.85,
            60 => 1.5,
            _ => 1.0,
        }
    }
}

impl Default for Framerate {
    fn default() -> Self {
        Self(30)
    }
}

impl TryFrom<u32> for Framerate {
    type Error = anyhow::Error;

    fn try_from(fps: u32) -> Result<Self> {
        anyhow::ensure!(Self::ALL.contains(&fps), "{fps} fps is not offered; expected 24, 30 or 60");
        Ok(Self(fps))
    }
}

impl From<Framerate> for u32 {
    fn from(rate: Framerate) -> u32 {
        rate.0
    }
}

impl std::str::FromStr for Framerate {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        s.trim_end_matches("fps").parse::<u32>()?.try_into()
    }
}

/// Scales `source` to `height`, keeping its aspect ratio.
///
/// Screens are not all 16:9, and the encoder stretches to whatever size it is
/// given, so every rung is derived from the real source. Never scales up: a
/// rung taller than the source is published at the source's own size. Both
/// sides come out even, which 4:2:0 chroma requires.
fn fit(source: Size, height: u32) -> Size {
    let height = height.min(source.height);
    let width = (u64::from(source.width) * u64::from(height) / u64::from(source.height)) as u32;
    Size::new(width & !1, height & !1)
}

/// The lowest and highest bitrate a share may be set to, in bits per second.
pub const MIN_BITRATE: u64 = 200_000;
pub const MAX_BITRATE: u64 = 20_000_000;

/// What a share sends: the top resolution, the framerate, and the top rung's
/// bitrate (`None` for that resolution's default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing {
    pub max: Quality,
    pub rate: Framerate,
    pub bitrate: Option<u64>,
}

impl Default for Outgoing {
    fn default() -> Self {
        Self {
            max: Quality::High,
            rate: Framerate::default(),
            bitrate: None,
        }
    }
}

impl Outgoing {
    /// The default bitrate of `quality` at this framerate.
    pub fn default_bitrate(&self, quality: Quality) -> u64 {
        (quality.bitrate() as f64 * self.rate.bitrate_scale()) as u64
    }

    /// What the top rung is encoded at.
    pub fn top_bitrate(&self) -> u64 {
        self.bitrate.unwrap_or_else(|| self.default_bitrate(self.max))
    }
}

/// The renditions published for a `source`-sized share: every rung up to and
/// including `send.max`. Rungs that collapse to the same size on a small source
/// are dropped, keeping the smaller name.
///
/// Each rung gets its own default bitrate, except the top one, which gets the
/// chosen bitrate when there is one. No lower rung is ever given more than the
/// top: a choice below a lower rung's default pulls that rung down with it.
pub fn ladder(source: Size, send: Outgoing) -> Vec<VideoRendition> {
    let top = send.top_bitrate();
    let mut seen = Vec::new();
    let mut ladder: Vec<VideoRendition> = Quality::ALL
        .into_iter()
        .filter(|q| *q <= send.max)
        .filter_map(|q| {
            let size = fit(source, q.height());
            if seen.contains(&size) {
                return None;
            }
            seen.push(size);
            Some(
                VideoRendition::new(q.rendition())
                    .with_size(size)
                    .with_bitrate(send.default_bitrate(q).min(top))
                    .with_keyframe_interval(KEYFRAME_INTERVAL),
            )
        })
        .collect();
    // The top rung that survived, whatever its name: on a small source the
    // largest names can collapse away.
    if let Some(last) = ladder.last_mut() {
        last.bitrate = Some(top);
    }
    ladder
}

/// Whether a share shows a whole display or a single window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Display,
    Window,
}

/// A display or window that can be shared.
#[derive(Debug, Clone)]
pub struct Display {
    pub id: String,
    /// The display's name, or the window's title.
    pub name: String,
    /// The application a window belongs to; empty for a display.
    pub app: String,
    pub size: Size,
    pub kind: Kind,
}

/// The one display offered on Wayland, standing for whichever screen is
/// picked in the desktop's own dialog.
///
/// Wayland lets an app capture the screen only through the desktop portal,
/// which shows its own picker when the capture starts. XWayland still lists
/// monitors and windows, but a capture through it sees only other X11 apps,
/// not the screen, so a share of one never sends a picture.
const PORTAL: &str = "portal";

/// Opens the desktop portal's D-Bus connection on the caller's runtime, which
/// has to be one that lives as long as the app.
///
/// ashpd keeps one connection for the whole process, and with tokio its
/// reader is a task on whichever runtime opened it first. Left to itself
/// that is the first share's capture, which runs on a runtime of its own
/// that ends with the share; the connection is then still handed out but
/// nothing reads its replies, so the next share waits on the portal forever.
#[cfg(all(target_os = "linux", feature = "pipewire"))]
pub fn keep_portal_connection() {
    tokio::spawn(async {
        if let Err(e) = ashpd::desktop::screencast::Screencast::new().await {
            tracing::debug!("the screen cast portal is not available: {e}");
        }
    });
}

/// Whether this is a Wayland session, where only the portal can capture.
fn wayland() -> bool {
    cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some()
}

/// A guess at the size of the screen the portal will capture, which the
/// ladder is cut from before the capture starts. XWayland's monitors are the
/// best hint there is; the biggest is the safe one, since the ladder only
/// steps down from it.
async fn portal_display() -> Display {
    let size = capture::displays()
        .await
        .ok()
        .and_then(|all| {
            all.into_iter()
                .map(|d| Size::new(d.width, d.height))
                .max_by_key(|s| u64::from(s.width) * u64::from(s.height))
        })
        .unwrap_or(Size::new(1920, 1080));
    Display {
        id: PORTAL.to_string(),
        name: "Choose a screen".to_string(),
        app: String::new(),
        size,
        kind: Kind::Display,
    }
}

/// The displays this machine offers, in the order an index selects them.
pub async fn displays() -> Result<Vec<Display>> {
    if wayland() {
        return Ok(vec![portal_display().await]);
    }
    let displays = capture::displays().await.map_err(|e| anyhow::anyhow!("{e:#}"))?;
    Ok(displays
        .into_iter()
        .map(|d| Display {
            id: d.id,
            name: d.name,
            app: String::new(),
            size: Size::new(d.width, d.height),
            kind: Kind::Display,
        })
        .collect())
}

/// Smaller than this, a "window" is a minimized one (Windows reports those as
/// a title-bar stub) or a tool strip, and neither captures as anything useful.
const MIN_WINDOW: Size = Size {
    width: 200,
    height: 120,
};

/// The windows that can be shared on their own. Left out: Noxid's (sharing the
/// call into itself shows nobody anything new), minimized ones, and the
/// desktop itself, which is what sharing the display is for.
///
/// Windows and X11 only. On Wayland, and anywhere else listing fails, the
/// list is empty and only whole displays are offered.
pub async fn windows() -> Vec<Display> {
    if wayland() {
        return Vec::new();
    }
    let own = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
        .unwrap_or_default();
    match capture::windows().await {
        Ok(windows) => windows
            .into_iter()
            .filter(|w| w.app.to_lowercase() != own)
            .filter(|w| w.width >= MIN_WINDOW.width && w.height >= MIN_WINDOW.height)
            .filter(|w| !(w.app.eq_ignore_ascii_case("explorer") && w.title == "Program Manager"))
            .map(|w| Display {
                id: w.id,
                name: w.title,
                app: w.app,
                size: Size::new(w.width, w.height),
                kind: Kind::Window,
            })
            .collect(),
        Err(e) => {
            tracing::debug!("windows cannot be listed here: {e:#}");
            Vec::new()
        }
    }
}

/// Finds a display by index, exact id, or part of its name. `None` is the
/// first display. A window is found by its exact id.
pub async fn find_display(spec: Option<&str>) -> Result<Display> {
    if let Some(spec) = spec
        && let Some(window) = windows().await.into_iter().find(|w| w.id == spec)
    {
        return Ok(window);
    }
    let displays = displays().await?;
    let found = match spec {
        None => displays.into_iter().next(),
        Some(spec) => match spec.parse::<usize>() {
            Ok(index) => displays.into_iter().nth(index),
            Err(_) => {
                let wanted = spec.to_lowercase();
                displays
                    .into_iter()
                    .find(|d| d.id == spec || d.name.to_lowercase().contains(&wanted))
            }
        },
    };
    found.with_context(|| match spec {
        None => "no display to share".to_string(),
        Some(spec) => format!("no display matching {spec:?}"),
    })
}

/// Whether a shared window is still open. A capture of a closed window does
/// not end, it just stops sending pictures, so viewers would be left on its
/// last one; the share watches for this instead.
///
/// Minimized windows count as open. Where windows cannot be listed, a window
/// is taken to be open, so a share is never ended on a guess.
pub async fn window_exists(id: &str) -> bool {
    #[cfg(windows)]
    {
        crate::still::window_exists(id)
    }
    #[cfg(not(windows))]
    {
        match capture::windows().await {
            Ok(windows) => windows.iter().any(|w| w.id == id),
            Err(_) => true,
        }
    }
}

/// `screen` as it is now. What a share shows can change size while it is
/// shared (a game switching resolution, a resized window), and the ladder is
/// cut from its size. Left as it was if it cannot be found.
pub async fn refreshed(screen: &Screen) -> Screen {
    match screen {
        Screen::Display(target) => {
            let found = match target.kind {
                Kind::Window => windows().await.into_iter().find(|w| w.id == target.id),
                Kind::Display => displays()
                    .await
                    .ok()
                    .and_then(|all| all.into_iter().find(|d| d.id == target.id)),
            };
            let mut target = target.clone();
            if let Some(found) = found {
                target.size = found.size;
            }
            Screen::Display(target)
        }
        #[cfg(feature = "test-source")]
        Screen::Test => Screen::Test,
    }
}

/// The capture source for a display or window.
fn capture_source(target: &Display) -> capture::Source {
    match target.kind {
        // No id: on Wayland that is what opens the portal's picker.
        Kind::Display if target.id == PORTAL => capture::Source::Display(None),
        Kind::Display => capture::Source::Display(Some(target.id.clone())),
        Kind::Window => capture::Source::Window(target.id.clone()),
    }
}

/// How long a preview waits for its first picture. A window that is
/// minimized or on another virtual desktop may never send one.
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(3);

/// How long a camera runs before its preview picture is taken. A camera's
/// first frames come before its exposure has settled, often dark or green.
const CAMERA_SETTLE: Duration = Duration::from_millis(700);

/// A still of what sharing `id` (a display or window id) would show, scaled
/// to at most `width` pixels wide, as its size and tightly packed RGBA.
///
/// On Windows the still comes from GDI (see `still`), which unlike a capture
/// session flashes no yellow border. Elsewhere it opens a capture just for
/// the one picture and closes it again.
pub async fn preview(id: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    // A still would need the portal, and so its dialog, just for the picker.
    anyhow::ensure!(id != PORTAL, "the desktop's own dialog shows the screens");
    let target = find_display(Some(id)).await?;
    #[cfg(windows)]
    {
        tokio::task::spawn_blocking(move || crate::still::grab(&target, width)).await?
    }
    #[cfg(not(windows))]
    {
        let mut config = capture::Config::default();
        config.source = capture_source(&target);
        config.cursor = false;
        capture_still(&config, width, Duration::ZERO).await
    }
}

/// A still from camera `id`, like [`preview`]. Cameras are opened one at a
/// time: many cannot be opened twice at once, and several previews asked for
/// together would otherwise fight over a shared device.
pub async fn camera_preview(id: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = ONE_AT_A_TIME.lock().await;
    #[cfg(windows)]
    if let Some(name) = id.strip_prefix(DSHOW_PREFIX) {
        return dshow_still(name, width).await;
    }
    let native = capture_still(&camera_config(Some(id)), width, CAMERA_SETTLE).await;
    // The same fallback as `resolve_camera`: a camera Media Foundation will
    // not start may work through DirectShow.
    #[cfg(windows)]
    if native.is_err()
        && let Some(name) = native_name(id).await
        && let Ok(still) = dshow_still(&name, width).await
    {
        return Ok(still);
    }
    native
}

#[cfg(windows)]
async fn dshow_still(name: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    let owned = name.to_string();
    let mut frames = tokio::task::spawn_blocking(move || crate::dshow::open(&owned)).await??;
    let read = async {
        let started = std::time::Instant::now();
        loop {
            let frame = frames
                .recv()
                .await
                .context("the camera stopped before its first picture")?;
            if started.elapsed() >= CAMERA_SETTLE {
                return anyhow::Ok(frame);
            }
        }
    };
    let frame = tokio::time::timeout(PREVIEW_TIMEOUT + CAMERA_SETTLE, read)
        .await
        .context("no picture arrived in time")??;
    drop(frames);
    shrink(&frame, width)
}

/// Opens `config`, keeps the newest picture after `settle` (or the first one,
/// for no settling), and closes the capture again.
async fn capture_still(config: &capture::Config, width: u32, settle: Duration) -> Result<(Size, Vec<u8>)> {
    let mut stream = capture::open(config).await.map_err(|e| anyhow::anyhow!("{e:#}"))?;
    let read = async {
        let started = std::time::Instant::now();
        loop {
            let frame = stream
                .read()
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))?
                .context("the capture ended before its first picture")?;
            if started.elapsed() >= settle {
                return anyhow::Ok(frame);
            }
        }
    };
    let frame = tokio::time::timeout(PREVIEW_TIMEOUT + settle, read)
        .await
        .context("no picture arrived in time")??;
    drop(stream);
    shrink(&frame, width)
}

/// Scales `frame` to `width` pixels wide, as its size and tightly packed RGBA.
fn shrink(frame: &Frame, width: u32) -> Result<(Size, Vec<u8>)> {
    use iroh_live::media::video::{Output, convert, resize};

    let source = frame.size();
    let height = (u64::from(source.height) * u64::from(width) / u64::from(source.width.max(1))) as u32;
    let size = fit(source, height.max(2));
    let mut scale = resize::Config::default();
    scale.output = Output::Cpu;
    let small = frame.resize(size, &scale).map_err(|e| anyhow::anyhow!("{e:#}"))?;
    let rgba = small
        .surface
        .to_rgba(&convert::Config::default())
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
    Ok((rgba.size(), rgba.into_data()))
}

/// What a screen share shows.
#[derive(Debug, Clone)]
pub enum Screen {
    Display(Display),
    /// A generated moving pattern, for testing without a desktop to capture.
    #[cfg(feature = "test-source")]
    Test,
}

impl Screen {
    /// The window whose sound alone goes with this share, if it is a window.
    pub fn window(&self) -> Option<&str> {
        match self {
            Screen::Display(display) if display.kind == Kind::Window => Some(&display.id),
            _ => None,
        }
    }

    /// The size the share's ladder is cut from.
    pub fn size(&self) -> Size {
        match self {
            Screen::Display(display) => display.size,
            #[cfg(feature = "test-source")]
            Screen::Test => test_pattern(),
        }
    }

    fn source(&self, rate: Framerate) -> VideoSource {
        match self {
            Screen::Display(display) => {
                let mut config = capture::Config::default();
                config.framerate = iroh_live::media::video::Rate::new(rate.fps(), 1).ok();
                config.source = capture_source(display);
                VideoSource::Capture(config)
            }
            #[cfg(feature = "test-source")]
            Screen::Test => iroh_live::media::test_source::video(test_pattern(), rate.fps()),
        }
    }
}

/// A camera that can be shared.
#[derive(Debug, Clone, Serialize)]
pub struct CameraInfo {
    /// What selects it: an opaque id from the platform's camera API.
    pub id: String,
    pub name: String,
}

/// Ids of cameras reached through DirectShow rather than the media stack:
/// this prefix and the camera's name.
#[cfg(windows)]
const DSHOW_PREFIX: &str = "dshow:";

/// The cameras this machine has. A machine without any, or where they cannot
/// be listed, gets an empty list.
///
/// On Windows, DirectShow cameras the media stack cannot see (most virtual
/// ones: OBS, NVIDIA Broadcast) are added after its own, once per name.
pub async fn cameras() -> Vec<CameraInfo> {
    #[allow(unused_mut, reason = "only Windows adds to it")]
    let mut list: Vec<CameraInfo> = match capture::cameras().await {
        Ok(cameras) => cameras
            .into_iter()
            .map(|c| CameraInfo { id: c.id, name: c.name })
            .collect(),
        Err(e) => {
            tracing::debug!("cameras cannot be listed here: {e:#}");
            Vec::new()
        }
    };
    #[cfg(windows)]
    match tokio::task::spawn_blocking(crate::dshow::cameras).await {
        Ok(Ok(names)) => {
            for name in names {
                if !list.iter().any(|c| c.name == name) {
                    list.push(CameraInfo {
                        id: format!("{DSHOW_PREFIX}{name}"),
                        name,
                    });
                }
            }
        }
        Ok(Err(e)) => tracing::debug!("DirectShow cameras cannot be listed: {e:#}"),
        Err(e) => tracing::debug!("DirectShow listing failed: {e}"),
    }
    list
}

/// A camera that has been checked and is known to open, and the way to open
/// it. Made by [`resolve_camera`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSource {
    /// The id it was chosen by, as [`cameras`] listed it.
    pub id: String,
    backend: Backend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Backend {
    /// The media stack's own capture (Media Foundation on Windows).
    Native,
    /// DirectShow, by camera name.
    #[cfg(windows)]
    DirectShow(String),
}

/// The name the media stack lists camera `id` under.
#[cfg(windows)]
async fn native_name(id: &str) -> Option<String> {
    capture::cameras()
        .await
        .ok()?
        .into_iter()
        .find(|c| c.id == id)
        .map(|c| c.name)
}

/// Finds out how camera `id` opens, by opening it once and closing it again.
/// Publishing opens the camera in a task of its own, so without this a camera
/// that is missing, busy or (for a virtual one) not running would look
/// switched on while sending nothing.
///
/// On Windows a camera the media stack lists but cannot start is tried again
/// through DirectShow under the same name: DroidCam's Windows 11 source, for
/// one, fails in Media Foundation (Windows' own Camera app included) while
/// its DirectShow side works.
pub async fn resolve_camera(id: &str) -> Result<CameraSource> {
    #[cfg(windows)]
    if let Some(name) = id.strip_prefix(DSHOW_PREFIX) {
        check_dshow(name).await?;
        return Ok(CameraSource {
            id: id.to_string(),
            backend: Backend::DirectShow(name.to_string()),
        });
    }

    let native = capture::open(&camera_config(Some(id))).await.map(drop);
    let Err(error) = native else {
        return Ok(CameraSource {
            id: id.to_string(),
            backend: Backend::Native,
        });
    };
    let detail = format!("{error:#}");

    #[cfg(windows)]
    if let Some(name) = native_name(id).await {
        match check_dshow(&name).await {
            Ok(()) => {
                tracing::info!(%name, "camera opens through DirectShow instead: {detail}");
                return Ok(CameraSource {
                    id: id.to_string(),
                    backend: Backend::DirectShow(name),
                });
            }
            Err(e) => tracing::debug!(%name, "nor through DirectShow: {e:#}"),
        }
    }

    // Windows found the camera but would not start it, by either route.
    if detail.contains("activate camera") {
        anyhow::bail!(
            "Windows would not start this camera. If it is a virtual camera (DroidCam, OBS), make \
             sure its app is running and connected; otherwise close any other app using it. \
             ({detail})"
        )
    }
    anyhow::bail!(detail)
}

/// Opens a DirectShow camera and closes it again.
#[cfg(windows)]
async fn check_dshow(name: &str) -> Result<()> {
    let name = name.to_string();
    let frames = tokio::task::spawn_blocking(move || crate::dshow::open(&name)).await??;
    drop(frames);
    // The capture thread notices the drop on its next poll; give it time to
    // let go, or a camera that allows one user would refuse the real open.
    tokio::time::sleep(Duration::from_millis(300)).await;
    Ok(())
}

/// A DirectShow camera as a stream of frames. The camera opens on the first
/// poll, off the async threads, and closes when the stream is dropped.
#[cfg(windows)]
fn dshow_frames(name: String) -> n0_future::boxed::BoxStream<Frame> {
    use tokio::sync::mpsc::Receiver;
    Box::pin(n0_future::stream::unfold(None::<Receiver<Frame>>, move |open| {
        let name = name.clone();
        async move {
            let mut frames = match open {
                Some(frames) => frames,
                None => match tokio::task::spawn_blocking(move || crate::dshow::open(&name)).await {
                    Ok(Ok(frames)) => frames,
                    Ok(Err(e)) => {
                        tracing::warn!("camera would not open: {e:#}");
                        return None;
                    }
                    Err(_) => return None,
                },
            };
            let frame = frames.recv().await?;
            Some((frame, Some(frames)))
        }
    }))
}

/// The camera's one rendition. A face needs far less than a screen of text,
/// and a camera share is usually watched small, so one 720p rung at a modest
/// bitrate is enough; a ladder would cost a second encode for little gain.
pub const CAMERA_RENDITION: &str = "camera";
const CAMERA_SIZE: Size = Size {
    width: 1280,
    height: 720,
};
const CAMERA_FPS: u32 = 30;
const CAMERA_BITRATE: u64 = 1_500_000;

fn camera_config(device: Option<&str>) -> capture::Config {
    let mut config = capture::Config::default();
    config.source = capture::Source::Camera(device.map(str::to_string));
    // Preferences: a camera that cannot do 720p30 opens at its nearest mode,
    // and the rendition keeps whatever size it delivers (and its aspect).
    config.width = Some(CAMERA_SIZE.width);
    config.height = Some(CAMERA_SIZE.height);
    config.framerate = iroh_live::media::video::Rate::new(CAMERA_FPS, 1).ok();
    config
}

/// Starts sending `camera` on `broadcast`, replacing whatever it sent before.
pub fn share_camera(broadcast: &LocalBroadcast, camera: &CameraSource) -> Result<()> {
    let source = match &camera.backend {
        Backend::Native => VideoSource::Capture(camera_config(Some(&camera.id))),
        #[cfg(windows)]
        Backend::DirectShow(name) => VideoSource::Frames(dshow_frames(name.clone())),
    };
    let rendition = VideoRendition::new(CAMERA_RENDITION)
        .with_bitrate(CAMERA_BITRATE)
        .with_keyframe_interval(KEYFRAME_INTERVAL);
    broadcast
        .video()
        .set_renditions(source, vec![rendition])
        .map_err(|e| anyhow::anyhow!("{e:#}"))
}

/// What the microphone track carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mic {
    /// A real input device; which one is chosen separately.
    Device,
    /// A generated tone, for testing without a microphone.
    #[cfg(feature = "test-source")]
    Test,
}

impl Mic {
    /// `device` is an id from [`crate::audio::devices`], or `None` for the
    /// system default.
    fn source(self, device: Option<&str>) -> AudioSource {
        match self {
            Mic::Device => {
                let mut config = iroh_live::media::audio::capture::Config::default();
                config.source =
                    iroh_live::media::audio::capture::Source::Microphone(device.map(str::to_string));
                AudioSource::Device(config)
            }
            #[cfg(feature = "test-source")]
            Mic::Test => iroh_live::media::test_source::audio(
                440.0,
                48_000,
                iroh_live::media::audio::Layout::Mono,
            ),
        }
    }
}

/// Starts sharing `screen` as `send` says, replacing any share in progress.
pub fn share_screen(broadcast: &LocalBroadcast, screen: &Screen, send: Outgoing) -> Result<()> {
    broadcast
        .video()
        .set_renditions(screen.source(send.rate), ladder(screen.size(), send))
        .map_err(|e| anyhow::anyhow!("{e:#}"))
}

/// Starts (or restarts) the microphone.
pub fn set_mic(broadcast: &LocalBroadcast, mic: Mic, device: Option<&str>) {
    broadcast.audio().set(mic.source(device));
}

/// Stops the microphone, which also releases the device.
pub fn mute_mic(broadcast: &LocalBroadcast) {
    broadcast.audio().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(max: Quality) -> Outgoing {
        Outgoing {
            max,
            ..Outgoing::default()
        }
    }

    fn sizes(source: Size, max: Quality) -> Vec<(String, Size)> {
        ladder(source, send(max))
            .into_iter()
            .map(|r| (r.name, r.size.unwrap()))
            .collect()
    }

    #[test]
    fn the_ladder_stops_at_the_cap() {
        let names = |max| {
            sizes(Size::new(1920, 1080), max)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(Quality::High), ["screen/low", "screen/mid", "screen/high"]);
        assert_eq!(names(Quality::Mid), ["screen/low", "screen/mid"]);
        assert_eq!(names(Quality::Low), ["screen/low"]);
    }

    #[test]
    fn rungs_keep_the_screens_aspect_ratio() {
        // A 16:10 laptop panel must not be squashed into 16:9.
        let ladder = sizes(Size::new(2560, 1600), Quality::High);
        assert_eq!(ladder[0].1, Size::new(576, 360));
        assert_eq!(ladder[1].1, Size::new(1152, 720));
        assert_eq!(ladder[2].1, Size::new(1728, 1080));
    }

    #[test]
    fn a_small_screen_is_never_scaled_up() {
        // 1366x768: the 1080 rung would upscale, so it is capped at native.
        let ladder = sizes(Size::new(1366, 768), Quality::High);
        assert_eq!(ladder.len(), 3, "{ladder:?}");
        assert_eq!(ladder[2].1, Size::new(1366, 768));

        let ladder = sizes(Size::new(1024, 600), Quality::High);
        assert_eq!(ladder.last().unwrap().1, Size::new(1024, 600));
        assert_eq!(ladder.len(), 2, "720 and 1080 both collapse to native");
    }

    #[test]
    fn sizes_are_always_even() {
        let ladder = sizes(Size::new(1365, 767), Quality::High);
        for (_, size) in ladder {
            assert_eq!(size.width % 2, 0);
            assert_eq!(size.height % 2, 0);
        }
    }

    #[test]
    fn only_the_offered_framerates_parse() {
        assert_eq!("60".parse::<Framerate>().unwrap().fps(), 60);
        assert_eq!("24fps".parse::<Framerate>().unwrap().fps(), 24);
        assert!("25".parse::<Framerate>().is_err());
        assert!(serde_json::from_str::<Framerate>("120").is_err());
        assert_eq!(serde_json::to_string(&Framerate::default()).unwrap(), "30");
    }

    #[test]
    fn faster_shares_get_more_bits() {
        let bitrate = |fps: u32| {
            let send = Outgoing {
                rate: fps.try_into().unwrap(),
                ..send(Quality::High)
            };
            ladder(Size::new(1920, 1080), send)[2].bitrate
        };
        assert!(bitrate(24) < bitrate(30));
        assert!(bitrate(30) < bitrate(60));
    }

    #[test]
    fn a_chosen_bitrate_sets_the_top_rung_only() {
        let bitrates = |bitrate| {
            let send = Outgoing {
                bitrate,
                ..send(Quality::High)
            };
            ladder(Size::new(1920, 1080), send)
                .into_iter()
                .map(|r| r.bitrate.unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(bitrates(None), [600_000, 2_500_000, 5_000_000]);
        assert_eq!(bitrates(Some(8_000_000)), [600_000, 2_500_000, 8_000_000]);
        // Below the middle rung's default: the middle rung comes down too.
        assert_eq!(bitrates(Some(2_000_000)), [600_000, 2_000_000, 2_000_000]);
    }

    #[test]
    fn a_collapsed_ladder_still_gets_the_chosen_bitrate() {
        // 1280x720: the 1080 rung collapses into the 720 one.
        let send = Outgoing {
            bitrate: Some(4_000_000),
            ..send(Quality::High)
        };
        let ladder = ladder(Size::new(1280, 720), send);
        assert_eq!(ladder.len(), 2);
        assert_eq!(ladder[1].bitrate, Some(4_000_000));
    }

    #[test]
    fn rendition_names_round_trip() {
        for q in Quality::ALL {
            assert_eq!(Quality::from_rendition(q.rendition()), Some(q));
        }
        assert_eq!(Quality::from_rendition("video"), None);
    }
}

