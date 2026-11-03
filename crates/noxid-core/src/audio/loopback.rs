//! Windows: the PC's sound through WASAPI process loopback. Either everything
//! the PC plays except Noxid itself (`EXCLUDE_TARGET_PROCESS_TREE` on our own
//! process id), or only what one application plays (`INCLUDE_TARGET_PROCESS_TREE`
//! on the process that owns a shared window). Needs Windows 10 version 2004 or
//! later.
//!
//! The flow follows Microsoft's ApplicationLoopback sample: activate a virtual
//! audio device asynchronously, initialise it as an event-driven loopback
//! client in a fixed float format, and drain packets as they arrive.

use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use tokio::sync::mpsc;
use windows::Win32::Foundation::{CloseHandle, E_FAIL, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, ActivateAudioInterfaceAsync, PROCESS_LOOPBACK_MODE, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;
use windows_core::{HRESULT, IUnknown, Interface, Ref, implement};

/// The format asked of the loopback client; WASAPI converts to it.
pub(super) const SAMPLE_RATE: u32 = 48_000;
pub(super) const CHANNELS: u32 = 2;

/// How long the capture thread waits for a packet before checking whether it
/// should fill silence or stop.
const POLL: Duration = Duration::from_millis(10);

/// Process loopback delivers nothing while nothing plays. The encoder needs an
/// unbroken timeline, so after this long without packets the gap is filled
/// with silence.
const SILENCE_AFTER: Duration = Duration::from_millis(20);

/// Whose sound is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    /// Everything the PC plays, except Noxid and its children.
    AllButUs,
    /// Only what this process and its children play.
    Process(u32),
}

impl Target {
    fn params(self) -> (u32, PROCESS_LOOPBACK_MODE) {
        match self {
            Target::AllButUs => (
                unsafe { GetCurrentProcessId() },
                PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
            ),
            Target::Process(pid) => (pid, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE),
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(std_mpsc::SyncSender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _operation: Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows_core::Result<()> {
        let _ = self.0.try_send(());
        Ok(())
    }
}

/// Starts capturing on a thread of its own. Returns once audio is flowing, or
/// with the reason it could not start. The thread stops when `tx`'s receiver
/// is dropped.
pub(super) fn start(tx: mpsc::Sender<Vec<f32>>, target: Target) -> Result<()> {
    let (ready_tx, ready_rx) = std_mpsc::sync_channel::<Result<()>>(1);
    std::thread::Builder::new()
        .name("noxid-loopback".into())
        .spawn(move || {
            // SAFETY: COM is initialised for this thread inside `run`, and every
            // interface it touches is created and released on this thread.
            if let Err(e) = unsafe { run(&tx, &ready_tx, target) } {
                // Reaches the caller if it is still waiting; otherwise the
                // stream simply ends.
                tracing::warn!("PC sound capture stopped: {e:#}");
                let _ = ready_tx.try_send(Err(e));
            }
        })?;
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("the PC sound capture did not start in time"))?
}

/// What has to outlive the activation: the completion handler is signalled from
/// inside `ActivateCompleted`, and releasing it (or the operation that holds
/// it) while that callback is still returning on the system's thread is a
/// use-after-free that takes the whole process down. Held until capture ends.
struct Activation {
    _operation: IActivateAudioInterfaceAsyncOperation,
    _handler: IActivateAudioInterfaceCompletionHandler,
}

/// Activates the process-loopback virtual device for `target`.
unsafe fn activate(target: Target) -> Result<(IAudioClient, Activation)> {
    let (process, mode) = target.params();
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: process,
                ProcessLoopbackMode: mode,
            },
        },
    };
    // A PROPVARIANT pointing at `params` on our stack, which outlives the call.
    // ManuallyDrop because the windows crate gives PROPVARIANT a Drop that runs
    // PropVariantClear, which would hand our stack pointer to CoTaskMemFree and
    // corrupt the heap (STATUS_HEAP_CORRUPTION, seen in testing).
    let prop = std::mem::ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: &mut params as *mut _ as *mut u8,
                    },
                },
            }),
        },
    });

    let (done_tx, done_rx) = std_mpsc::sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done_tx).into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*prop as *const PROPVARIANT),
            &handler,
        )?
    };
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("audio activation did not complete"))?;

    let mut result = HRESULT(0);
    let mut interface: Option<IUnknown> = None;
    unsafe { operation.GetActivateResult(&mut result, &mut interface)? };
    result
        .ok()
        .map_err(|e| anyhow!("process loopback is unavailable (Windows 10 2004 or later is required): {e}"))?;
    let interface = interface.ok_or_else(|| windows_core::Error::from(E_FAIL))?;
    let client: IAudioClient = interface.cast()?;
    Ok((
        client,
        Activation {
            _operation: operation,
            _handler: handler,
        },
    ))
}

unsafe fn run(
    tx: &mpsc::Sender<Vec<f32>>,
    ready: &std_mpsc::SyncSender<Result<()>>,
    target: Target,
) -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    let result = unsafe { capture(tx, ready, target) };
    unsafe { CoUninitialize() };
    result
}

unsafe fn capture(
    tx: &mpsc::Sender<Vec<f32>>,
    ready: &std_mpsc::SyncSender<Result<()>>,
    target: Target,
) -> Result<()> {
    let (client, _activation) = unsafe { activate(target)? };

    let block_align = (CHANNELS * 4) as u16;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: CHANNELS as u16,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    // 200 ms of buffer, in 100 ns units.
    let buffer_duration = 2_000_000;
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            buffer_duration,
            0,
            &format,
            None,
        )?
    };
    let event = unsafe { CreateEventW(None, false, false, None)? };
    let result = unsafe { pump(&client, event, tx, ready) };
    unsafe {
        let _ = client.Stop();
        let _ = CloseHandle(event);
    }
    result
}

unsafe fn pump(
    client: &IAudioClient,
    event: windows::Win32::Foundation::HANDLE,
    tx: &mpsc::Sender<Vec<f32>>,
    ready: &std_mpsc::SyncSender<Result<()>>,
) -> Result<()> {
    unsafe { client.SetEventHandle(event)? };
    let reader: IAudioCaptureClient = unsafe { client.GetService()? };
    unsafe { client.Start()? };
    let _ = ready.try_send(Ok(()));

    let channels = CHANNELS as usize;
    let mut last_audio = Instant::now();
    loop {
        if tx.is_closed() {
            return Ok(());
        }
        let woke = unsafe { WaitForSingleObject(event, POLL.as_millis() as u32) } == WAIT_OBJECT_0;

        if woke {
            loop {
                let pending = unsafe { reader.GetNextPacketSize()? };
                if pending == 0 {
                    break;
                }
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                unsafe { reader.GetBuffer(&mut data, &mut frames, &mut flags, None, None)? };
                let count = frames as usize * channels;
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                let chunk = if silent || data.is_null() {
                    vec![0.0f32; count]
                } else {
                    // SAFETY: WASAPI hands over `frames` frames of the format we
                    // asked for (f32, CHANNELS interleaved), valid until release.
                    unsafe { std::slice::from_raw_parts(data as *const f32, count) }.to_vec()
                };
                unsafe { reader.ReleaseBuffer(frames)? };
                last_audio = Instant::now();
                if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(chunk) {
                    return Ok(());
                }
            }
        }

        // Nothing playing: keep the timeline going with silence, in whole
        // 10 ms steps so the gap is filled exactly.
        let quiet = last_audio.elapsed();
        if quiet >= SILENCE_AFTER {
            let steps = (quiet.as_millis() / 10) as u32;
            let samples = (SAMPLE_RATE as usize / 100) * channels * steps as usize;
            last_audio += Duration::from_millis(10) * steps;
            if let Err(mpsc::error::TrySendError::Closed(_)) = tx.try_send(vec![0.0f32; samples]) {
                return Ok(());
            }
        }
    }
}

/// The process that plays a window's sound, from a window id as the share
/// picker lists it (`window:<handle>` or the bare handle).
///
/// Store apps are the exception to "the window's owner": their frame belongs
/// to `ApplicationFrameHost.exe`, and the app itself owns a child window
/// inside it. That child's process is the one to capture.
pub(super) fn window_process(id: &str) -> Result<u32> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetWindowThreadProcessId, IsWindow};
    use windows_core::BOOL;

    let handle: usize = id
        .strip_prefix("window:")
        .unwrap_or(id)
        .parse()
        .map_err(|_| anyhow!("bad window id {id:?}"))?;
    let hwnd = HWND(handle as *mut _);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        anyhow::bail!("that window is gone");
    }
    let owner = |hwnd: HWND| {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        pid
    };
    let frame = owner(hwnd);
    if frame == 0 {
        anyhow::bail!("the window's process could not be found");
    }
    if !is_frame_host(frame) {
        return Ok(frame);
    }

    struct Search {
        frame: u32,
        app: u32,
    }
    unsafe extern "system" fn visit(child: HWND, state: LPARAM) -> BOOL {
        // SAFETY: `state` is the `Search` below, alive for the whole enumeration.
        let search = unsafe { &mut *(state.0 as *mut Search) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(child, Some(&mut pid)) };
        if pid != 0 && pid != search.frame {
            search.app = pid;
            return false.into();
        }
        true.into()
    }
    let mut search = Search { frame, app: 0 };
    unsafe {
        let _ = EnumChildWindows(Some(hwnd), Some(visit), LPARAM(&mut search as *mut _ as isize));
    }
    Ok(if search.app != 0 { search.app } else { frame })
}

/// Whether `pid` is the host that frames Store apps' windows.
fn is_frame_host(pid: u32) -> bool {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_core::PWSTR;

    let Ok(process) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    let mut buffer = [0u16; 1024];
    let mut len = buffer.len() as u32;
    let named = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len)
    };
    unsafe {
        let _ = CloseHandle(process);
    }
    named.is_ok()
        && String::from_utf16_lossy(&buffer[..len as usize])
            .to_lowercase()
            .ends_with("\\applicationframehost.exe")
}
