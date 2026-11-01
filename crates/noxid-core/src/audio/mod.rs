//! Audio beyond what iroh-live does for us: choosing devices, processing and
//! metering our own microphone, and capturing the PC's sound for a screen
//! share.

mod input;
#[cfg(windows)]
mod loopback;
mod pcm;
mod system;
mod voice;

use anyhow::Result;
use serde::Serialize;

pub(crate) use pcm::pcm_source;
pub use system::{SystemAudio, system_audio_supported};
pub use voice::Voice;

/// An input or output device, as the device pickers show it.
#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    /// Host-qualified id (for example `wasapi:...`), which is what selects it.
    pub id: String,
    pub name: String,
    /// Whether the system currently uses it by default.
    pub default: bool,
}

/// Microphones and outputs. A host that cannot be listed gives an empty list
/// rather than an error, so one broken host does not hide the other side.
pub async fn devices() -> (Vec<AudioDevice>, Vec<AudioDevice>) {
    use iroh_live::media::audio::{capture, playback};

    let inputs = capture::devices()
        .await
        .map(|list| {
            list.into_iter()
                .map(|d| AudioDevice {
                    id: d.id,
                    name: d.name,
                    default: d.default,
                })
                .collect()
        })
        .unwrap_or_default();
    let outputs = playback::devices()
        .await
        .map(|list| {
            list.into_iter()
                .map(|d| AudioDevice {
                    id: d.id,
                    name: d.name,
                    default: d.default,
                })
                .collect()
        })
        .unwrap_or_default();
    (dedupe(inputs), dedupe(outputs))
}

/// The same hardware is listed once per audio host (WASAPI and ASIO, or
/// PipeWire, PulseAudio and ALSA). A picker wants it once: keep the first
/// entry for each name, which is the preferred host's.
fn dedupe(devices: Vec<AudioDevice>) -> Vec<AudioDevice> {
    let mut seen = std::collections::HashSet::new();
    devices
        .into_iter()
        .filter(|d| seen.insert(d.name.clone()))
        .collect()
}

/// Moves every playing participant to another output device (`None` is the
/// system default). Opens the output on it if nothing is playing yet.
pub async fn set_output(device: Option<String>) -> Result<()> {
    use iroh_live::media::{audio::playback::Config, playback};

    let mut config = Config::default();
    config.device = device;
    // `open` only does something the first time; after that the engine exists
    // and `switch` is what moves it.
    playback::open(config.clone())
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
    playback::switch(config)
        .await
        .map_err(|e| anyhow::anyhow!("{e:#}"))
}
