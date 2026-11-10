//! Shared windows through Windows Graphics Capture.
//!
//! The media stack captures a single window with GDI: it copies the window's
//! device context and asks the window to paint itself over the copy. That
//! sees only what the desktop compositor draws. A borderless fullscreen game
//! in the foreground is handed the screen directly (independent flip), past
//! the compositor, so GDI keeps returning the last picture composed before
//! the game took over: viewers see it freeze whenever the game has focus, and
//! come back to life when it is alt-tabbed away.
//!
//! Windows Graphics Capture is the API made for this (Discord and OBS use
//! it): Windows keeps composing a captured window, so its pictures arrive
//! whatever path it presents through. Each picture is copied off the GPU,
//! converted to I420 the way the media stack converts its own screen
//! captures, and sent on as a frame.

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use iroh_live::media::video::{Color, Frame, I420, Size, Surface};
use tokio::sync::mpsc;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize};
use windows::Win32::UI::WindowsAndMessaging::IsWindow;
use windows_core::Interface;

/// Pictures waiting between the capture and the encoder. A capture faster
/// than the encoder loses pictures rather than building delay.
const QUEUE: usize = 2;

/// Buffers in the frame pool: one being read while the next is drawn.
const POOL_BUFFERS: i32 = 2;

/// How long opening a capture may take before giving up on it.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a new window size has to hold before the stream ends so the share
/// can reopen at it. A drag of the window's edge changes the size many times
/// a second; reopening for each would drop the encoder over and over.
const RESIZE_HOLD: Duration = Duration::from_millis(250);

/// Whether this Windows has Graphics Capture (1803 and later).
pub(crate) fn supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

/// Starts capturing window `id` (`window:<handle>`) at `fps` pictures a
/// second, each stamped by `now`. The capture runs on a thread of its own and
/// stops when the receiver is dropped, the window closes, or it changes size.
pub(crate) fn open(
    id: &str,
    fps: u32,
    now: impl Fn() -> moq_net::Timestamp + Send + 'static,
) -> Result<mpsc::Receiver<Frame>> {
    let handle: usize = id
        .strip_prefix("window:")
        .unwrap_or(id)
        .parse()
        .context("bad window id")?;
    let (tx, rx) = mpsc::channel(QUEUE);
    let (ready_tx, ready_rx) = std_mpsc::sync_channel::<Result<()>>(1);
    std::thread::Builder::new()
        .name("window-capture".into())
        .spawn(move || {
            if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
                let _ = ready_tx.send(Err(anyhow::anyhow!("RoInitialize: {e}")));
                return;
            }
            match Capture::start(HWND(handle as *mut _), fps) {
                Ok(capture) => {
                    let _ = ready_tx.send(Ok(()));
                    capture.run(&tx, now);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
            unsafe { RoUninitialize() };
        })
        .context("spawn the window capture thread")?;
    ready_rx
        .recv_timeout(START_TIMEOUT)
        .context("the window capture did not start in time")??;
    Ok(rx)
}

struct Capture {
    window: HWND,
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// The size the stream opened at, even on both sides; the encoder is
    /// built for it and cannot change.
    size: Size,
    interval: Duration,
    staging: Option<ID3D11Texture2D>,
}

impl Capture {
    fn start(window: HWND, fps: u32) -> Result<Self> {
        if !unsafe { IsWindow(Some(window)) }.as_bool() {
            bail!("that window is gone");
        }
        let (device, context) = d3d_device()?;
        let winrt_device: IDirect3DDevice = unsafe {
            CreateDirect3D11DeviceFromDXGIDevice(&device.cast::<IDXGIDevice>()?)?.cast()?
        };
        let interop = windows_core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForWindow(window) }.context("Windows would not capture that window")?;
        let item_size = item.Size()?;
        let size = Size::new(item_size.Width.max(0) as u32 & !1, item_size.Height.max(0) as u32 & !1);
        if size.width == 0 || size.height == 0 {
            bail!("that window has no size to capture");
        }
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            item_size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        // Neither is on every Windows: the cursor setting arrived in 2004 and
        // the border one in Windows 11. Without them the capture still works,
        // with the cursor drawn and a yellow border around the window.
        let _ = session.SetIsCursorCaptureEnabled(true);
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture()?;
        tracing::info!(%size, fps, "opened Windows Graphics Capture of a window");
        Ok(Self {
            window,
            context,
            device,
            pool,
            session,
            size,
            interval: Duration::from_secs(1) / fps.max(1),
            staging: None,
        })
    }

    /// Sends a picture every interval until nobody wants them, the window
    /// closes, or it settles at a new size. A window that draws nothing new
    /// (minimized, or a still screen) has its last picture sent again, so the
    /// stream keeps the pace the encoder was set up for.
    fn run(mut self, tx: &mpsc::Sender<Frame>, now: impl Fn() -> moq_net::Timestamp) {
        let mut last: Option<I420> = None;
        let mut resized: Option<(Size, Instant)> = None;
        let mut next = Instant::now();
        loop {
            let wait = next.saturating_duration_since(Instant::now());
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            next = (next + self.interval).max(Instant::now());

            if tx.is_closed() {
                break;
            }
            if !unsafe { IsWindow(Some(self.window)) }.as_bool() {
                tracing::info!("the shared window closed; ending its capture");
                break;
            }

            match self.latest() {
                Ok(Some((content, picture))) => {
                    if content == self.size {
                        resized = None;
                    } else {
                        match resized {
                            Some((pending, since)) if pending == content => {
                                if since.elapsed() >= RESIZE_HOLD {
                                    tracing::info!(
                                        from = %self.size,
                                        to = %content,
                                        "the shared window changed size; ending its capture"
                                    );
                                    break;
                                }
                            }
                            _ => resized = Some((content, Instant::now())),
                        }
                    }
                    if let Some(picture) = picture {
                        last = Some(picture);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("window capture failed: {e:#}");
                    break;
                }
            }

            let Some(picture) = &last else { continue };
            let frame = Frame::new(Surface::I420(picture.clone()), now());
            if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(frame) {
                break;
            }
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }

    /// The newest picture the window has drawn since the last call, if any,
    /// with the size its content has now. The picture is left out while that
    /// size differs from the one the stream opened at.
    fn latest(&mut self) -> Result<Option<(Size, Option<I420>)>> {
        // Older pictures are dropped as soon as a newer one is there, which
        // hands their buffers straight back to the pool.
        let mut newest = None;
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            newest = Some(frame);
        }
        let Some(frame) = newest else { return Ok(None) };
        let content = frame.ContentSize()?;
        let content = Size::new(content.Width.max(0) as u32 & !1, content.Height.max(0) as u32 & !1);
        if content != self.size {
            return Ok(Some((content, None)));
        }
        let texture: ID3D11Texture2D = unsafe {
            frame
                .Surface()?
                .cast::<IDirect3DDxgiInterfaceAccess>()?
                .GetInterface()?
        };
        let picture = self.convert(&texture)?;
        let _ = frame.Close();
        Ok(Some((content, Some(picture))))
    }

    /// Copies `texture` off the GPU and converts the opened size of it to I420.
    fn convert(&mut self, texture: &ID3D11Texture2D) -> Result<I420> {
        let staging = self.staging(texture)?;
        unsafe { self.context.CopyResource(&staging, texture) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
            .context("map the captured picture")?;
        let pitch = mapped.RowPitch;
        let bgra = unsafe {
            std::slice::from_raw_parts(mapped.pData as *const u8, pitch as usize * self.size.height as usize)
        };
        let result = bgra_to_i420(bgra, pitch, self.size);
        unsafe { self.context.Unmap(&staging, 0) };
        result
    }

    /// A CPU-readable texture the shape of the pool's, made on first use.
    fn staging(&mut self, texture: &ID3D11Texture2D) -> Result<ID3D11Texture2D> {
        if let Some(staging) = &self.staging {
            return Ok(staging.clone());
        }
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        desc.ArraySize = 1;
        desc.MipLevels = 1;
        let mut staging = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging)) }
            .context("create the staging texture")?;
        let staging = staging.context("no staging texture")?;
        self.staging = Some(staging.clone());
        Ok(staging)
    }
}

fn d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .context("create a Direct3D 11 device")?;
    Ok((device.context("no Direct3D device")?, context.context("no Direct3D context")?))
}

/// BGRA rows `stride` bytes apart to I420, in the color space the media
/// stack's own screen captures use for this size, so a window and a display
/// shared at the same size look the same.
fn bgra_to_i420(bgra: &[u8], stride: u32, size: Size) -> Result<I420> {
    use yuv::{YuvChromaSubsampling, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

    // `infer` only ever picks one of the two limited-range spaces.
    let color = Color::infer(size);
    let matrix = match color {
        Color::Bt601Limited => YuvStandardMatrix::Bt601,
        _ => YuvStandardMatrix::Bt709,
    };
    let range = YuvRange::Limited;
    let mut planar = YuvPlanarImageMut::alloc(size.width, size.height, YuvChromaSubsampling::Yuv420);
    yuv::bgra_to_yuv420(&mut planar, bgra, stride, range, matrix, YuvConversionMode::Balanced)
        .map_err(|e| anyhow::anyhow!("convert the captured picture: {e}"))?;
    let mut data = Vec::with_capacity(size.width as usize * size.height as usize * 3 / 2);
    data.extend_from_slice(planar.y_plane.borrow());
    data.extend_from_slice(planar.u_plane.borrow());
    data.extend_from_slice(planar.v_plane.borrow());
    Ok(I420::new(size, data)?.with_color(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captures the first shareable window for a second. Needs a desktop, so
    /// run by hand: `cargo test -p noxid-core wgc -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn captures_a_real_window() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let window = runtime
            .block_on(crate::media::windows())
            .into_iter()
            .next()
            .expect("no window to capture");
        println!("capturing {:?} ({}), {}", window.name, window.app, window.size);
        let started = Instant::now();
        let mut frames = open(&window.id, 30, move || {
            moq_net::Timestamp::from_micros(started.elapsed().as_micros() as u64).unwrap()
        })
        .unwrap();
        let first = frames.blocking_recv().expect("no first frame");
        println!("first frame after {:?}", started.elapsed());
        let mut size = Some(first.size());
        let mut count = 0;
        let counting = Instant::now();
        while counting.elapsed() < Duration::from_secs(1) {
            let frame = frames.blocking_recv().expect("the capture ended");
            size = Some(frame.size());
            count += 1;
        }
        println!("{count} frames of {size:?} in a second");
        assert!(count >= 20, "only {count} frames in a second");
    }

    #[test]
    fn converts_a_padded_picture() {
        // 4x2 white, rows padded to 24 bytes as a mapped texture's may be.
        let size = Size::new(4, 2);
        let stride = 24u32;
        let mut bgra = vec![0u8; stride as usize * 2];
        for row in 0..2 {
            bgra[row * stride as usize..row * stride as usize + 16].fill(255);
        }
        let picture = bgra_to_i420(&bgra, stride, size).unwrap();
        assert_eq!(picture.size(), size);
        // Limited range white: luma 235, neutral chroma.
        assert!(picture.y().iter().all(|&y| (234..=236).contains(&y)), "{:?}", picture.y());
        assert!(picture.u().iter().all(|&u| (127..=129).contains(&u)));
        assert!(picture.v().iter().all(|&v| (127..=129).contains(&v)));
    }
}
