//! Windows cameras through DirectShow.
//!
//! The media stack opens cameras through Media Foundation, which only sees
//! cameras with a Media Foundation driver. Most virtual cameras (OBS, NVIDIA
//! Broadcast, DroidCam's classic source) are DirectShow filters and invisible
//! to it, and some that are visible refuse to start there while their
//! DirectShow side works (DroidCam's Windows 11 source, seen in testing).
//!
//! The graph is the classic one: the camera's filter, then the Sample Grabber
//! asking for RGB32 (DirectShow inserts whatever decoder or converter gets it
//! there), then the Null Renderer. Each picture the grabber sees is turned
//! upright, converted to I420 and sent on as a frame. The Sample Grabber is
//! deprecated but still ships with Windows, and the SDK headers that declared
//! it are gone, so its two interfaces are declared here.

// The two COM interfaces declared here keep their Windows method names, as
// every binding in the windows crate does.
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use iroh_live::media::video::{Frame, Size, Surface};
use tokio::sync::mpsc;
use windows::Win32::Media::DirectShow::{
    IAMStreamConfig, IBaseFilter, ICaptureGraphBuilder2, ICreateDevEnum, IGraphBuilder, IMediaControl,
};
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, CLSID_CaptureGraphBuilder2, CLSID_FilterGraph, CLSID_SystemDeviceEnum,
    CLSID_VideoInputDeviceCategory, FORMAT_VideoInfo, MEDIASUBTYPE_RGB32, MEDIATYPE_Video, PIN_CATEGORY_CAPTURE,
    VIDEOINFOHEADER,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    IEnumMoniker, IMoniker,
};
use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VariantClear};
use windows_core::{BOOL, GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, implement, interface, w};

/// Pictures waiting between the grabber and the encoder. A camera faster
/// than the encoder loses pictures rather than building delay.
const QUEUE: usize = 2;

/// How long opening a camera may take before giving up on it.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the capture thread checks whether anyone still wants pictures.
const POLL: Duration = Duration::from_millis(100);

/// The height a camera is asked for, when it offers a choice.
const WANTED_HEIGHT: i32 = 720;

const CLSID_SAMPLE_GRABBER: GUID = GUID::from_u128(0xC1F400A0_3F08_11D3_9F0B_006008039E37);
const CLSID_NULL_RENDERER: GUID = GUID::from_u128(0xC1F400A4_3F08_11D3_9F0B_006008039E37);

/// qedit.h's ISampleGrabber, in vtable order.
#[interface("6B652FFF-11FE-4fce-92AD-0266B5D7C78F")]
unsafe trait ISampleGrabber: IUnknown {
    fn SetOneShot(&self, one_shot: BOOL) -> HRESULT;
    fn SetMediaType(&self, kind: *const AM_MEDIA_TYPE) -> HRESULT;
    fn GetConnectedMediaType(&self, kind: *mut AM_MEDIA_TYPE) -> HRESULT;
    fn SetBufferSamples(&self, buffer: BOOL) -> HRESULT;
    fn GetCurrentBuffer(&self, size: *mut i32, buffer: *mut i32) -> HRESULT;
    fn GetCurrentSample(&self, sample: *mut *mut c_void) -> HRESULT;
    fn SetCallback(&self, callback: *mut c_void, which: i32) -> HRESULT;
}

/// qedit.h's ISampleGrabberCB, in vtable order.
#[interface("0579154A-2B53-4994-B0D0-E773148EFF85")]
unsafe trait ISampleGrabberCB: IUnknown {
    fn SampleCB(&self, time: f64, sample: *mut c_void) -> HRESULT;
    fn BufferCB(&self, time: f64, buffer: *mut u8, len: i32) -> HRESULT;
}

/// `SetCallback`'s second argument: call `BufferCB` with the raw bytes.
const BUFFER_CALLBACK: i32 = 1;

/// Turns each grabbed RGB32 picture into a frame.
#[implement(ISampleGrabberCB)]
struct Grabber {
    tx: mpsc::Sender<Frame>,
    width: u32,
    height: u32,
    /// RGB32 rows run bottom to top unless the format says otherwise.
    bottom_up: bool,
    started: Instant,
}

impl ISampleGrabberCB_Impl for Grabber_Impl {
    unsafe fn SampleCB(&self, _time: f64, _sample: *mut c_void) -> HRESULT {
        HRESULT(0)
    }

    unsafe fn BufferCB(&self, _time: f64, buffer: *mut u8, len: i32) -> HRESULT {
        let stride = self.width as usize * 4;
        if buffer.is_null() || (len as usize) < stride * self.height as usize {
            return HRESULT(0);
        }
        // A full queue means the encoder is behind: this picture is skipped.
        let Ok(permit) = self.tx.try_reserve() else {
            return HRESULT(0);
        };
        // SAFETY: DirectShow hands over `len` bytes, valid for this call, and
        // the check above made sure they hold the whole picture.
        let bgra = unsafe { std::slice::from_raw_parts(buffer, len as usize) };
        if let Some(frame) = self.frame(bgra) {
            permit.send(frame);
        }
        HRESULT(0)
    }
}

impl Grabber {
    /// BGRA (possibly bottom-up) to an upright RGBA frame. 4:2:0 needs even
    /// sides, so an odd last row or column is cropped.
    fn frame(&self, bgra: &[u8]) -> Option<Frame> {
        let stride = self.width as usize * 4;
        let (width, height) = (self.width & !1, self.height & !1);
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in 0..height as usize {
            let source = if self.bottom_up { self.height as usize - 1 - row } else { row };
            let line = &bgra[source * stride..source * stride + width as usize * 4];
            for pixel in line.as_chunks::<4>().0 {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
            }
        }
        let surface = Surface::rgba(&rgba, Size::new(width, height)).ok()?;
        let timestamp = moq_net::Timestamp::from_micros(self.started.elapsed().as_micros() as u64).ok()?;
        Some(Frame::new(surface, timestamp))
    }
}

/// The DirectShow cameras, by friendly name, in the system's order.
pub(crate) fn cameras() -> Result<Vec<String>> {
    with_com(|| {
        let mut names = Vec::new();
        for moniker in monikers()? {
            if let Ok(name) = friendly_name(&moniker) {
                names.push(name);
            }
        }
        Ok(names)
    })
}

/// Opens the camera called `name` and starts it. Pictures arrive on the
/// returned receiver until it is dropped, which stops the camera.
pub(crate) fn open(name: &str) -> Result<mpsc::Receiver<Frame>> {
    let (tx, rx) = mpsc::channel(QUEUE);
    let (ready_tx, ready_rx) = std_mpsc::sync_channel::<Result<()>>(1);
    let name = name.to_string();
    std::thread::Builder::new()
        .name("noxid-dshow".into())
        .spawn(move || {
            let result = with_com(|| {
                // SAFETY: COM is initialised on this thread for the call, and
                // every interface is created, used and released on it.
                let graph = unsafe { start(&name, tx.clone()) };
                let (control, _graph) = match graph {
                    Ok(running) => running,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return Ok(());
                    }
                };
                let _ = ready_tx.send(Ok(()));
                while !tx.is_closed() {
                    std::thread::sleep(POLL);
                }
                unsafe {
                    let _ = control.Stop();
                }
                Ok(())
            });
            if let Err(e) = result {
                tracing::warn!("DirectShow camera stopped: {e:#}");
            }
        })?;
    ready_rx
        .recv_timeout(START_TIMEOUT)
        .map_err(|_| anyhow!("the camera did not start in time"))??;
    Ok(rx)
}

fn with_com<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let result = f();
    unsafe { CoUninitialize() };
    result
}

fn monikers() -> Result<Vec<IMoniker>> {
    let devices: ICreateDevEnum = unsafe { CoCreateInstance(&CLSID_SystemDeviceEnum, None, CLSCTX_INPROC_SERVER)? };
    let mut list: Option<IEnumMoniker> = None;
    unsafe { devices.CreateClassEnumerator(&CLSID_VideoInputDeviceCategory, &mut list, 0)? };
    // No cameras at all is S_FALSE with no enumerator, not an error.
    let Some(list) = list else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    loop {
        let mut slot = [None];
        let mut fetched = 0u32;
        let hr = unsafe { list.Next(&mut slot, Some(&mut fetched)) };
        if hr.is_err() || fetched == 0 {
            break;
        }
        if let Some(moniker) = slot[0].take() {
            out.push(moniker);
        }
    }
    Ok(out)
}

fn friendly_name(moniker: &IMoniker) -> Result<String> {
    let bag: IPropertyBag = unsafe { moniker.BindToStorage(None, None)? };
    let mut value = VARIANT::default();
    unsafe { bag.Read(w!("FriendlyName"), &mut value, None)? };
    // SAFETY: the union is read only after checking it holds a BSTR.
    let name = unsafe {
        let inner = &value.Anonymous.Anonymous;
        if inner.vt == VT_BSTR {
            Some(inner.Anonymous.bstrVal.to_string())
        } else {
            None
        }
    };
    unsafe {
        let _ = VariantClear(&mut value);
    }
    name.context("the camera has no name")
}

/// Builds and runs the graph for camera `name`. Returns what has to stay
/// alive while it runs.
unsafe fn start(name: &str, tx: mpsc::Sender<Frame>) -> Result<(IMediaControl, Graph)> {
    let moniker = monikers()?
        .into_iter()
        .find(|m| friendly_name(m).is_ok_and(|n| n == name))
        .with_context(|| format!("no camera called {name:?}"))?;
    let source: IBaseFilter = unsafe { moniker.BindToObject(None, None)? };

    let graph: IGraphBuilder = unsafe { CoCreateInstance(&CLSID_FilterGraph, None, CLSCTX_INPROC_SERVER)? };
    let builder: ICaptureGraphBuilder2 =
        unsafe { CoCreateInstance(&CLSID_CaptureGraphBuilder2, None, CLSCTX_INPROC_SERVER)? };
    unsafe { builder.SetFiltergraph(&graph)? };
    unsafe { graph.AddFilter(&source, w!("camera"))? };
    unsafe { prefer_720p(&builder, &source) };

    let grabber_filter: IBaseFilter = unsafe { CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER) }
        .context("the Sample Grabber is not available on this system")?;
    let grabber: ISampleGrabber = grabber_filter.cast()?;
    let wanted = AM_MEDIA_TYPE {
        majortype: MEDIATYPE_Video,
        subtype: MEDIASUBTYPE_RGB32,
        formattype: FORMAT_VideoInfo,
        ..Default::default()
    };
    unsafe { grabber.SetMediaType(&wanted).ok()? };
    unsafe { graph.AddFilter(&grabber_filter, w!("grabber"))? };
    let sink: IBaseFilter = unsafe { CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)? };
    unsafe { graph.AddFilter(&sink, w!("sink"))? };

    unsafe {
        builder
            .RenderStream(Some(&PIN_CATEGORY_CAPTURE), &MEDIATYPE_Video, &source, &grabber_filter, &sink)
            .context("the camera's picture could not be converted")?
    };

    // What the grabber actually gets, now the graph is connected.
    let mut connected = AM_MEDIA_TYPE::default();
    unsafe { grabber.GetConnectedMediaType(&mut connected).ok()? };
    let header = (connected.formattype == FORMAT_VideoInfo
        && connected.cbFormat as usize >= std::mem::size_of::<VIDEOINFOHEADER>()
        && !connected.pbFormat.is_null())
    .then(|| unsafe { (*(connected.pbFormat as *const VIDEOINFOHEADER)).bmiHeader });
    unsafe { free_media_type(&mut connected) };
    let header = header.context("the camera reported no picture format")?;
    let (width, height) = (header.biWidth.unsigned_abs(), header.biHeight.unsigned_abs());
    if width < 2 || height < 2 {
        bail!("the camera reported a {width}x{height} picture");
    }

    let callback: ISampleGrabberCB = Grabber {
        tx,
        width,
        height,
        bottom_up: header.biHeight > 0,
        started: Instant::now(),
    }
    .into();
    unsafe {
        grabber.SetBufferSamples(false.into()).ok()?;
        grabber.SetOneShot(false.into()).ok()?;
        grabber.SetCallback(callback.as_raw(), BUFFER_CALLBACK).ok()?;
    }

    let control: IMediaControl = graph.cast()?;
    unsafe { control.Run().context("the camera would not start")? };
    Ok((
        control,
        Graph {
            _graph: graph,
            _builder: builder,
            _source: source,
            _grabber: grabber_filter,
            _sink: sink,
            _callback: callback,
        },
    ))
}

/// Everything the running graph needs kept alive.
struct Graph {
    _graph: IGraphBuilder,
    _builder: ICaptureGraphBuilder2,
    _source: IBaseFilter,
    _grabber: IBaseFilter,
    _sink: IBaseFilter,
    _callback: ISampleGrabberCB,
}

/// Picks the camera mode nearest to 720p, if the camera offers a list.
/// Best effort: a camera without the interface keeps its own default.
unsafe fn prefer_720p(builder: &ICaptureGraphBuilder2, source: &IBaseFilter) {
    let mut config: *mut c_void = std::ptr::null_mut();
    let found = unsafe {
        builder.FindInterface(
            Some(&PIN_CATEGORY_CAPTURE),
            Some(&MEDIATYPE_Video),
            source,
            &IAMStreamConfig::IID,
            &mut config,
        )
    };
    if found.is_err() || config.is_null() {
        return;
    }
    // SAFETY: FindInterface handed over one reference to an IAMStreamConfig.
    let config = unsafe { IAMStreamConfig::from_raw(config) };
    let (mut count, mut size) = (0i32, 0i32);
    if unsafe { config.GetNumberOfCapabilities(&mut count, &mut size) }.is_err() {
        return;
    }
    let mut caps = vec![0u8; size.max(0) as usize];
    let mut best: Option<(*mut AM_MEDIA_TYPE, i32)> = None;
    for index in 0..count {
        let mut kind: *mut AM_MEDIA_TYPE = std::ptr::null_mut();
        if unsafe { config.GetStreamCaps(index, &mut kind, caps.as_mut_ptr()) }.is_err() || kind.is_null() {
            continue;
        }
        let height = unsafe {
            let k = &*kind;
            (k.formattype == FORMAT_VideoInfo && !k.pbFormat.is_null())
                .then(|| (*(k.pbFormat as *const VIDEOINFOHEADER)).bmiHeader.biHeight.abs())
        };
        let distance = height.map(|h| (h - WANTED_HEIGHT).abs());
        match (distance, best) {
            (Some(d), Some((_, best_d))) if d >= best_d => unsafe { delete_media_type(kind) },
            (Some(d), previous) => {
                if let Some((old, _)) = previous {
                    unsafe { delete_media_type(old) };
                }
                best = Some((kind, d));
            }
            (None, _) => unsafe { delete_media_type(kind) },
        }
    }
    if let Some((kind, _)) = best {
        unsafe {
            let _ = config.SetFormat(kind);
            delete_media_type(kind);
        }
    }
}

/// Frees what an AM_MEDIA_TYPE points to (FreeMediaType in the old SDK).
unsafe fn free_media_type(kind: &mut AM_MEDIA_TYPE) {
    if !kind.pbFormat.is_null() {
        unsafe { CoTaskMemFree(Some(kind.pbFormat as *const c_void)) };
        kind.pbFormat = std::ptr::null_mut();
        kind.cbFormat = 0;
    }
    unsafe { std::mem::ManuallyDrop::drop(&mut kind.pUnk) };
}

/// Frees an AM_MEDIA_TYPE that was itself allocated by COM (DeleteMediaType).
unsafe fn delete_media_type(kind: *mut AM_MEDIA_TYPE) {
    unsafe {
        free_media_type(&mut *kind);
        CoTaskMemFree(Some(kind as *const c_void));
    }
}
