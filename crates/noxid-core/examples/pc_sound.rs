//! Captures the PC's sound for a few seconds and reports how much arrived.
//! A hardware check for the share-with-sound path, outside any call.
//!
//! Pass a window id (`window:<handle>`) to capture only that window's
//! application.

use std::time::{Duration, Instant};

use n0_future::StreamExt;
use noxid_core::audio::SystemAudio;
use iroh_live::media::publish::AudioSource;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    noxid_core::log::init(None);
    let start = Instant::now();
    let window = std::env::args().nth(1);
    let source = SystemAudio::start(window.as_deref(), move || start.elapsed())?;
    let AudioSource::Frames { input, mut frames } = source else {
        anyhow::bail!("expected PCM frames");
    };
    println!("capturing: {} Hz, {:?}", input.sample_rate, input.layout);

    let deadline = Instant::now() + Duration::from_secs(3);
    let (mut count, mut peak) = (0usize, 0.0f32);
    while Instant::now() < deadline {
        let Some(frame) = tokio::time::timeout(Duration::from_secs(1), frames.next()).await? else {
            break;
        };
        count += 1;
        for sample in frame.data.as_chunks::<4>().0 {
            peak = peak.max(f32::from_le_bytes(*sample).abs());
        }
    }
    println!("{count} frames of 20 ms in 3 s, peak {peak:.3}");
    Ok(())
}
