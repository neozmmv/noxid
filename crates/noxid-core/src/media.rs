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
pub use iroh_live::media::video::{Color, Frame, Size};

/// How often each rung inserts a keyframe.
///
/// A subscriber draws nothing until the next keyframe, so this bounds how long
/// someone joining waits for the shared screen and how long a rendition switch
/// takes to land. Shorter than the encoder default of two seconds because a
/// call is joined and refocused far more often than a stream is.
const KEYFRAME_INTERVAL: Duration = Duration::from_secs(1);

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

    /// Target bitrate in bits per second.
    pub fn bitrate(self) -> u64 {
        match self {
            Quality::Low => 300_000,
            Quality::Mid => 1_200_000,
            Quality::High => 2_500_000,
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

/// The renditions published for a `source`-sized share capped at `max`: every
/// rung up to and including it. Rungs that collapse to the same size on a small
/// source are dropped, keeping the smaller name.
pub fn ladder(source: Size, max: Quality, rate: Framerate) -> Vec<VideoRendition> {
    let mut seen = Vec::new();
    Quality::ALL
        .into_iter()
        .filter(|q| *q <= max)
        .filter_map(|q| {
            let size = fit(source, q.height());
            if seen.contains(&size) {
                return None;
            }
            seen.push(size);
            Some(
                VideoRendition::new(q.rendition())
                    .with_size(size)
                    .with_bitrate((q.bitrate() as f64 * rate.bitrate_scale()) as u64)
                    .with_keyframe_interval(KEYFRAME_INTERVAL),
            )
        })
        .collect()
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

/// The displays this machine offers, in the order an index selects them.
pub async fn displays() -> Result<Vec<Display>> {
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

/// The capture source for a display or window.
fn capture_source(target: &Display) -> capture::Source {
    match target.kind {
        Kind::Display => capture::Source::Display(Some(target.id.clone())),
        Kind::Window => capture::Source::Window(target.id.clone()),
    }
}

/// How long a preview waits for its first picture. A window that is
/// minimized or on another virtual desktop may never send one.
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(3);

/// A still of what sharing `id` (a display or window id) would show, scaled
/// to at most `width` pixels wide, as its size and tightly packed RGBA.
///
/// Opens a capture just for the one picture and closes it again, so it costs
/// nothing once the picker is gone.
pub async fn preview(id: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    use iroh_live::media::video::{Output, convert, resize};

    let target = find_display(Some(id)).await?;
    let mut config = capture::Config::default();
    config.source = capture_source(&target);
    config.cursor = false;
    let mut stream = capture::open(&config).await.map_err(|e| anyhow::anyhow!("{e:#}"))?;
    let frame = tokio::time::timeout(PREVIEW_TIMEOUT, stream.read())
        .await
        .context("no picture arrived in time")?
        .map_err(|e| anyhow::anyhow!("{e:#}"))?
        .context("the capture ended before its first picture")?;
    drop(stream);

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
    fn size(&self) -> Size {
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

/// Starts sharing `screen`, capped at `max` and sent at `rate`, replacing any
/// share in progress.
pub fn share_screen(broadcast: &LocalBroadcast, screen: &Screen, max: Quality, rate: Framerate) -> Result<()> {
    broadcast
        .video()
        .set_renditions(screen.source(rate), ladder(screen.size(), max, rate))
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

    fn sizes(source: Size, max: Quality) -> Vec<(String, Size)> {
        ladder(source, max, Framerate::default())
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
        let bitrate = |fps: u32| ladder(Size::new(1920, 1080), Quality::High, fps.try_into().unwrap())[2].bitrate;
        assert!(bitrate(24) < bitrate(30));
        assert!(bitrate(30) < bitrate(60));
    }

    #[test]
    fn rendition_names_round_trip() {
        for q in Quality::ALL {
            assert_eq!(Quality::from_rendition(q.rendition()), Some(q));
        }
        assert_eq!(Quality::from_rendition("video"), None);
    }
}

