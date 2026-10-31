//! Captures the PC's sound for a few seconds and reports how much arrived.
//! A hardware check for the share-with-sound path, outside any call.

use std::time::{Duration, Instant};

use n0_future::StreamExt;
use noxid_core::audio::SystemAudio;
use iroh_live::media::publish::AudioSource;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    noxid_core::log::init(None);
    let start = Instant::now();
    let source = SystemAudio::start(move || start.elapsed())?;
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
        for chunk in frame.data.chunks_exact(4) {
            peak = peak.max(f32::from_le_bytes(chunk.try_into().unwrap()).abs());
        }
    }
    println!("{count} frames of 20 ms in 3 s, peak {peak:.3}");
    Ok(())
}
