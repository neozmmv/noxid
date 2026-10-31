//! How loud we are, for our own speaking indicator.
//!
//! iroh-live keeps its capture stream private, so the level of what we publish
//! is not readable from it. This opens the same microphone a second time just
//! to measure it. Sound servers and WASAPI shared mode both allow that, and it
//! costs a peak computation per buffer.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;

use super::input;

pub struct MicMeter {
    level: Arc<AtomicU32>,
    _running: input::Running,
}

impl std::fmt::Debug for MicMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MicMeter").field("level", &self.level()).finish()
    }
}

impl MicMeter {
    /// Starts metering `device` (an id from [`super::devices`]), or the default
    /// microphone for `None`.
    pub fn start(device: Option<&str>) -> Result<Self> {
        let level = Arc::new(AtomicU32::new(0));
        let writer = level.clone();
        let device = input::find_input(device)?;
        let (running, _) = input::open(device, move |samples| {
            let peak = samples.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            writer.store(peak.min(1.0).to_bits(), Ordering::Relaxed);
        })?;
        Ok(Self {
            level,
            _running: running,
        })
    }

    /// The peak of the most recent buffer, in `0.0..=1.0`.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}
