//! Our microphone, cleaned up and levelled before anyone hears it.
//!
//! iroh-live can publish a microphone directly, but it sends what the device
//! gives: a quiet mic stays quiet for everyone, and the listener cannot make up
//! for it because playback volume tops out at 1.0. So the mic goes through the
//! WebRTC audio processing chain (sonora, pure Rust) first: a high-pass filter
//! for rumble, noise suppression, and AGC2's adaptive digital gain, which
//! brings speech to a steady loudness whatever the mic's own level.
//!
//! One pipeline serves both the call and our own speaking indicator, so the
//! level shown is the level sent, and the mic is opened once.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use iroh_live::media::publish::AudioSource;
use sonora::config::{
    AdaptiveDigital, FixedDigital, GainController2, HighPassFilter, NoiseSuppression,
};
use sonora::{AudioProcessing, Config, StreamConfig};
use tokio::sync::mpsc;

use super::input;

/// Processed 10 ms chunks buffered towards the encoder. Short on purpose: the
/// encoder reads continuously, and anything older than this is better dropped
/// than played late.
const QUEUE: usize = 8;

/// Gain on top of the adaptive stage, through AGC2's limiter (which makes it a
/// compressor rather than clipping). Listeners play voices at half scale so
/// that their volume can go to 200%; this is what keeps 100% as loud as an
/// unprocessed full-scale voice. See `mesh::VOICE_HEADROOM`.
const FIXED_GAIN_DB: f32 = 6.0;

type Outlet = Arc<Mutex<Option<mpsc::Sender<Vec<f32>>>>>;

/// A running, processed microphone.
pub struct Voice {
    level: Arc<AtomicU32>,
    outlet: Outlet,
    sample_rate: u32,
    _running: input::Running,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voice")
            .field("sample_rate", &self.sample_rate)
            .finish()
    }
}

impl Voice {
    /// Opens `device` (an id from [`super::devices`]), or the default
    /// microphone for `None`, and starts processing. Nothing is sent anywhere
    /// until [`source`](Self::source) is called.
    pub fn start(device: Option<&str>) -> Result<Self> {
        let device = input::find_input(device)?;
        let level = Arc::new(AtomicU32::new(0));
        let outlet: Outlet = Arc::new(Mutex::new(None));

        let mut chain = Chain::new(input::format(&device)?);
        let writer = level.clone();
        let out = outlet.clone();
        let (running, format) = input::open(device, move |samples| {
            chain.push(samples, |processed| {
                let peak = processed.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
                // Held at the highest peak until read. For non-negative floats
                // the bit patterns order like the values, so an integer max is
                // a float max.
                writer.fetch_max(peak.min(1.0).to_bits(), Ordering::Relaxed);
                // Never block the audio thread: a contended or full outlet
                // drops this chunk.
                if let Ok(slot) = out.try_lock()
                    && let Some(tx) = slot.as_ref()
                {
                    let _ = tx.try_send(processed.to_vec());
                }
            });
        })?;

        Ok(Self {
            level,
            outlet,
            sample_rate: format.sample_rate,
            _running: running,
        })
    }

    /// The highest processed peak since the last call, in `0.0..=1.0`.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.swap(0, Ordering::Relaxed))
    }

    /// A publishable mono source fed from this mic. A later call takes over
    /// from an earlier one, whose stream then ends.
    pub fn source(&self, now: impl Fn() -> Duration + Send + Sync + 'static) -> AudioSource {
        let (tx, rx) = mpsc::channel(QUEUE);
        *self.outlet.lock().unwrap() = Some(tx);
        super::pcm_source(rx, self.sample_rate, 1, now)
    }

    /// Stops feeding any source; the mic keeps running for the level.
    pub fn detach(&self) {
        *self.outlet.lock().unwrap() = None;
    }
}

/// Downmix to mono, cut into 10 ms frames (what the processor takes), process.
struct Chain {
    processor: AudioProcessing,
    channels: usize,
    frame: usize,
    pending: Vec<f32>,
    output: Vec<f32>,
}

impl Chain {
    fn new(format: input::Format) -> Self {
        let config = Config {
            high_pass_filter: Some(HighPassFilter::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            gain_controller2: Some(GainController2 {
                adaptive_digital: Some(AdaptiveDigital::default()),
                fixed_digital: FixedDigital {
                    gain_db: FIXED_GAIN_DB,
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        let processor = AudioProcessing::builder()
            .config(config)
            .capture_config(StreamConfig::new(format.sample_rate, 1))
            .build();
        let frame = (format.sample_rate / 100) as usize;
        Self {
            processor,
            channels: format.channels.max(1) as usize,
            frame,
            pending: Vec::with_capacity(frame * 4),
            output: vec![0.0; frame],
        }
    }

    fn push(&mut self, interleaved: &[f32], mut emit: impl FnMut(&[f32])) {
        let channels = self.channels;
        self.pending.extend(
            interleaved
                .chunks_exact(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
        let mut start = 0;
        while self.pending.len() - start >= self.frame {
            let input = &self.pending[start..start + self.frame];
            match self
                .processor
                .process_capture_f32(&[input], &mut [&mut self.output])
            {
                Ok(()) => emit(&self.output),
                // A frame the processor refuses still goes out, untouched,
                // rather than leaving a hole in the voice.
                Err(e) => {
                    tracing::debug!("voice processing skipped a frame: {e:?}");
                    emit(input);
                }
            }
            start += self.frame;
        }
        self.pending.drain(..start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(sample_rate: u32, channels: u32) -> Chain {
        Chain::new(input::Format {
            sample_rate,
            channels,
        })
    }

    #[test]
    fn output_comes_in_whole_mono_frames() {
        let mut chain = chain(48_000, 2);
        let mut frames = Vec::new();
        // 25 ms of stereo in odd-sized buffers: two whole 10 ms frames out,
        // 5 ms left waiting.
        for size in [700, 900, 800] {
            chain.push(&vec![0.1; size], |f| frames.push(f.len()));
        }
        assert_eq!(frames, [480, 480]);
        assert_eq!(chain.pending.len(), 240);
    }

    #[test]
    fn quiet_speech_is_brought_up() {
        // A tone at -40 dBFS stands in for a quiet mic. After a few seconds
        // the adaptive gain has had time to act.
        let rate = 48_000;
        let mut chain = chain(rate, 1);
        let mut peak = 0.0f32;
        for n in 0..(rate * 6 / 480) {
            let buffer: Vec<f32> = (0..480)
                .map(|i| {
                    let t = (n * 480 + i) as f32 / rate as f32;
                    // A warbling tone, so it reads as voice rather than noise.
                    let f = 220.0 + 80.0 * (t * 3.0 * std::f32::consts::TAU).sin();
                    0.01 * (t * f * std::f32::consts::TAU).sin()
                })
                .collect();
            chain.push(&buffer, |out| {
                if n > rate * 5 / 480 {
                    peak = out.iter().fold(peak, |p, s| p.max(s.abs()));
                }
            });
        }
        assert!(peak > 0.02, "still quiet after six seconds: peak {peak}");
    }
}
