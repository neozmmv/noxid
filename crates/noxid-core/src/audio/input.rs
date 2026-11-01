//! Opening a cpal input stream that hands over `f32` samples whatever the
//! device's native format, on a thread that owns it.
//!
//! cpal streams are not `Send` on every platform, so each one lives on its own
//! thread for its whole life and is dropped there when its owner says stop.

use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// The layout a stream delivers.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Format {
    pub sample_rate: u32,
    pub channels: u32,
}

/// Keeps an input stream running; dropping it stops the stream.
pub(crate) struct Running {
    _stop: mpsc::Sender<()>,
}

/// Finds an input device by the id `capture::devices` reports, or the system
/// default for `None`. Ids are host-qualified, so the lookup goes to the host
/// that issued the id, as moq-audio's own resolver does.
pub(crate) fn find_input(id: Option<&str>) -> Result<cpal::Device> {
    match id {
        None => cpal::default_host()
            .default_input_device()
            .context("no default input device"),
        Some(id) => {
            let wanted: cpal::DeviceId = id.parse().map_err(|e| anyhow!("{id:?}: {e}"))?;
            let host = cpal::host_from_id(wanted.host()).map_err(|e| anyhow!("{id:?}: {e}"))?;
            host.input_devices()?
                .find(|d| d.id().ok().as_ref() == Some(&wanted))
                .with_context(|| format!("input device {id:?} not found"))
        }
    }
}

/// The format [`open`] will deliver from `device`.
pub(crate) fn format(device: &cpal::Device) -> Result<Format> {
    let config = device.default_input_config()?.config();
    Ok(Format {
        sample_rate: config.sample_rate,
        channels: u32::from(config.channels),
    })
}

/// Opens `device` and calls `on_data` with every buffer, as interleaved `f32`.
/// Returns once the stream is running (or failed to start).
pub(crate) fn open<F>(device: cpal::Device, on_data: F) -> Result<(Running, Format)>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<Format>>(1);
    let (stop_tx, stop_rx) = mpsc::channel::<()>();

    std::thread::Builder::new()
        .name("noxid-audio-input".into())
        .spawn(move || {
            let stream = match build(&device, on_data) {
                Ok((stream, format)) => match stream.play() {
                    Ok(()) => {
                        let _ = ready_tx.send(Ok(format));
                        stream
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(anyhow!("could not start the stream: {e}")));
                        return;
                    }
                },
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Parked until the owner drops its handle.
            let _ = stop_rx.recv();
            drop(stream);
        })?;

    let format = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("the audio device did not start in time"))??;
    Ok((Running { _stop: stop_tx }, format))
}

fn build<F>(device: &cpal::Device, mut on_data: F) -> Result<(cpal::Stream, Format)>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    let supported = device.default_input_config()?;
    let config: cpal::StreamConfig = supported.config();
    let format = Format {
        sample_rate: config.sample_rate,
        channels: u32::from(config.channels),
    };
    let on_error = |e: cpal::Error| tracing::debug!("audio input stream error: {e}");

    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => {
            device.build_input_stream(config, move |data: &[f32], _: &_| on_data(data), on_error, None)?
        }
        cpal::SampleFormat::I16 => {
            let mut buffer = Vec::new();
            device.build_input_stream(
                config,
                move |data: &[i16], _: &_| {
                    buffer.clear();
                    buffer.extend(data.iter().map(|s| f32::from(*s) / 32768.0));
                    on_data(&buffer);
                },
                on_error,
                None,
            )?
        }
        cpal::SampleFormat::U16 => {
            let mut buffer = Vec::new();
            device.build_input_stream(
                config,
                move |data: &[u16], _: &_| {
                    buffer.clear();
                    buffer.extend(data.iter().map(|s| (f32::from(*s) - 32768.0) / 32768.0));
                    on_data(&buffer);
                },
                on_error,
                None,
            )?
        }
        other => anyhow::bail!("unsupported sample format {other:?}"),
    };
    Ok((stream, format))
}
