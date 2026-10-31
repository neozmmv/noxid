//! The PC's own sound, for a screen share.
//!
//! What matters most is what is *not* in it: our own output. Noxid plays the
//! other participants' voices, and capturing those back into a share would
//! send everyone their own voice a moment late.
//!
//! - Windows: WASAPI process loopback with Noxid's process tree excluded
//!   (Windows 10 2004 and later), so the exclusion is exact.
//! - Linux: the sound server's monitor of the default output. It cannot leave
//!   one application out, so participants' voices are in it too; the share
//!   says so in the UI. A proper fix needs a PipeWire filter graph.
//! - macOS is not a target yet.

use std::time::Duration;

use anyhow::Result;
use iroh_live::media::publish::AudioSource;
use tokio::sync::mpsc;

/// Chunks buffered between the capture thread and the encoder. Plenty for
/// scheduling hiccups; a consumer that falls further behind loses audio rather
/// than adding delay.
const QUEUE: usize = 64;

/// Whether this platform can capture the PC's sound for a share.
pub fn system_audio_supported() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

/// Starts capturing the PC's sound. Capture stops by itself once the returned
/// source (and the publisher holding it) is dropped.
pub struct SystemAudio;

impl SystemAudio {
    pub fn start(now: impl Fn() -> Duration + Send + Sync + 'static) -> Result<AudioSource> {
        let (tx, rx) = mpsc::channel(QUEUE);
        let (sample_rate, channels) = platform::start(tx)?;
        Ok(super::pcm_source(rx, sample_rate, channels, now))
    }
}

#[cfg(windows)]
mod platform {
    use super::*;

    pub(super) fn start(tx: mpsc::Sender<Vec<f32>>) -> Result<(u32, u32)> {
        super::super::loopback::start(tx)?;
        Ok((super::super::loopback::SAMPLE_RATE, super::super::loopback::CHANNELS))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use anyhow::Context;
    use cpal::traits::{DeviceTrait, HostTrait};

    pub(super) fn start(tx: mpsc::Sender<Vec<f32>>) -> Result<(u32, u32)> {
        // PulseAudio and PipeWire's Pulse layer list a "Monitor of <output>"
        // source for every output; the default output's comes first when the
        // server lists the default sink first, which both do.
        let device = cpal::default_host()
            .input_devices()?
            .find(|d| {
                d.description()
                    .map(|desc| desc.name().to_lowercase().contains("monitor"))
                    .unwrap_or(false)
            })
            .context("no monitor source found; is PipeWire or PulseAudio running?")?;

        let sender = tx.clone();
        let (running, format) = super::super::input::open(device, move |samples| {
            // Never block the audio callback: a full queue drops this chunk.
            let _ = sender.try_send(samples.to_vec());
        })?;
        // The stream lives until the encoder stops reading.
        tokio::spawn(async move {
            tx.closed().await;
            drop(running);
        });
        Ok((format.sample_rate, format.channels))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::*;

    pub(super) fn start(_tx: mpsc::Sender<Vec<f32>>) -> Result<(u32, u32)> {
        anyhow::bail!("sharing the PC's sound is not supported on this platform yet")
    }
}
