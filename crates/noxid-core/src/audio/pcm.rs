//! Turns raw PCM from a capture thread into the timestamped frames the
//! publisher encodes.

use std::time::Duration;

use bytes::Bytes;
use iroh_live::media::audio::{Frame, Layout, encode::Input};
use iroh_live::media::publish::AudioSource;
use tokio::sync::mpsc;

/// One frame per 20 ms, the Opus frame length, so the encoder takes each whole.
const FRAME: Duration = Duration::from_millis(20);

/// How far the sample-count timeline may wander from the broadcast clock
/// before it is pulled back. Capture clocks drift, and a share that runs for an
/// hour would otherwise end up seconds away from its own picture.
const MAX_DRIFT: Duration = Duration::from_millis(200);

struct State {
    rx: mpsc::Receiver<Vec<f32>>,
    pending: Vec<f32>,
    /// Broadcast time of the first sample of the current timeline.
    anchor: Option<Duration>,
    /// Samples per channel emitted since `anchor`.
    emitted: u64,
}

/// A publishable source from interleaved `f32` chunks of any size arriving on
/// `rx`. `now` reads the broadcast clock, which is what puts this audio on the
/// same timeline as the picture it accompanies. The stream ends when the
/// sender goes away.
pub(crate) fn pcm_source(
    rx: mpsc::Receiver<Vec<f32>>,
    sample_rate: u32,
    channels: u32,
    now: impl Fn() -> Duration + Send + Sync + 'static,
) -> AudioSource {
    let layout = match channels {
        1 => Layout::Mono,
        2 => Layout::Stereo,
        n => Layout::Discrete(n),
    };
    let input = Input::new(sample_rate, layout);
    let per_channel = (u64::from(sample_rate) * FRAME.as_millis() as u64 / 1000) as usize;
    let per_frame = per_channel * channels as usize;

    let state = State {
        rx,
        pending: Vec::with_capacity(per_frame * 2),
        anchor: None,
        emitted: 0,
    };
    let now = std::sync::Arc::new(now);
    let frames = n0_future::stream::unfold(state, move |mut state| {
        let now = now.clone();
        async move {
            while state.pending.len() < per_frame {
                let chunk = state.rx.recv().await?;
                state.pending.extend_from_slice(&chunk);
            }
            let samples: Vec<f32> = state.pending.drain(..per_frame).collect();

            let clock = now();
            let elapsed = Duration::from_micros(state.emitted * 1_000_000 / u64::from(sample_rate));
            let on_timeline = state.anchor.map(|a| a + elapsed);
            let timestamp = match on_timeline {
                Some(t) if t.abs_diff(clock) <= MAX_DRIFT => t,
                _ => {
                    // First frame, or drifted too far: start a new timeline here.
                    state.anchor = Some(clock);
                    state.emitted = 0;
                    clock
                }
            };
            state.emitted += per_channel as u64;

            let mut data = Vec::with_capacity(samples.len() * 4);
            for s in samples {
                data.extend_from_slice(&s.to_le_bytes());
            }
            let timestamp = moq_net::Timestamp::from_micros(timestamp.as_micros() as u64).ok()?;
            Some((Frame::new(Bytes::from(data), timestamp), state))
        }
    });

    AudioSource::Frames {
        input,
        frames: Box::pin(frames),
    }
}
