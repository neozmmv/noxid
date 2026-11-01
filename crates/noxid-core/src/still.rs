//! Stills of displays and windows for the share picker, taken with GDI on
//! Windows.
//!
//! The obvious way, a short Windows Graphics Capture session per still, makes
//! Windows flash its yellow capture border around every screen and window in
//! the list while the picker opens. GDI draws no border: `PrintWindow` asks a
//! window to paint itself (so one covered by others still comes out whole),
//! and the screen is read straight from the desktop. Both are scaled down by
//! GDI on the way out, so only the small still is ever copied.

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, HALFTONE, HBITMAP, HDC,
    HGDIOBJ, HMONITOR, MONITORINFO, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, IsWindow, PW_RENDERFULLCONTENT};
use windows_core::BOOL;

use crate::media::{Display, Kind, Size};

/// A still of `target`, at most `width` pixels wide, as tightly packed RGBA.
pub(crate) fn grab(target: &Display, width: u32) -> Result<(Size, Vec<u8>)> {
    // Physical pixels throughout, as the capture itself uses: a DPI-unaware
    // thread would get scaled coordinates on a high-DPI screen.
    let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let result = match target.kind {
        Kind::Display => display(&target.id, width),
        Kind::Window => window(&target.id, width),
    };
    unsafe { SetThreadDpiAwarenessContext(previous) };
    result
}

fn display(id: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    let index: usize = id.strip_prefix("display:").unwrap_or(id).parse().context("bad display id")?;
    let rect = monitors()?.get(index).copied().context("that display is gone")?;
    let screen = Dc::screen();
    shrink(screen.0, rect, width)
}

fn window(id: &str, width: u32) -> Result<(Size, Vec<u8>)> {
    let handle: usize = id.strip_prefix("window:").unwrap_or(id).parse().context("bad window id")?;
    let hwnd = HWND(handle as *mut _);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        bail!("that window is gone");
    }
    let mut outer = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut outer) }?;
    let (w, h) = (outer.right - outer.left, outer.bottom - outer.top);
    if w < 2 || h < 2 {
        bail!("that window has no size");
    }

    // The whole window, painted by itself into memory.
    let screen = Dc::screen();
    let full = Bitmap::compatible(screen.0, w, h)?;
    if !unsafe { PrintWindow(hwnd, full.dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) }.as_bool() {
        bail!("the window would not paint");
    }

    // Without the invisible resize borders Windows 10 and 11 put around
    // windows, which would show as a dark frame.
    let mut visible = outer;
    let _ = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut visible as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    };
    let crop = RECT {
        left: (visible.left - outer.left).clamp(0, w),
        top: (visible.top - outer.top).clamp(0, h),
        right: (visible.right - outer.left).clamp(0, w),
        bottom: (visible.bottom - outer.top).clamp(0, h),
    };
    let crop = if crop.right - crop.left >= 2 && crop.bottom - crop.top >= 2 {
        crop
    } else {
        RECT { left: 0, top: 0, right: w, bottom: h }
    };
    shrink(full.dc, crop, width)
}

/// Copies `rect` of `source` into a new image at most `width` wide, keeping
/// the aspect ratio, and returns it as RGBA.
fn shrink(source: HDC, rect: RECT, width: u32) -> Result<(Size, Vec<u8>)> {
    let (sw, sh) = (rect.right - rect.left, rect.bottom - rect.top);
    let tw = (width as i32).min(sw).max(1);
    let th = ((i64::from(sh) * i64::from(tw) / i64::from(sw)) as i32).max(1);

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: tw,
            // Negative: rows top to bottom, as RGBA consumers expect.
            biHeight: -th,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let dib = unsafe { CreateDIBSection(Some(source), &info, DIB_RGB_COLORS, &mut bits, None, 0) }?;
    let small = Bitmap::wrap(dib)?;
    unsafe {
        // HALFTONE averages instead of dropping pixels, which keeps text from
        // turning to noise at a fifth of its size.
        SetStretchBltMode(small.dc, HALFTONE);
        let _ = SetBrushOrgEx(small.dc, 0, 0, None);
        StretchBlt(small.dc, 0, 0, tw, th, Some(source), rect.left, rect.top, sw, sh, SRCCOPY)
            .ok()
            .context("could not copy the picture")?;
    }

    let len = (tw * th * 4) as usize;
    // SAFETY: the DIB section is tw x th at 32 bits per pixel and lives until
    // `small` drops, after this copy.
    let bgra = unsafe { std::slice::from_raw_parts(bits as *const u8, len) };
    let mut rgba = Vec::with_capacity(len);
    for px in bgra.as_chunks::<4>().0 {
        rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    Ok((Size::new(tw as u32, th as u32), rgba))
}

/// Monitor rectangles in the order the capture library numbers displays:
/// the primary first, then the others as the system lists them.
fn monitors() -> Result<Vec<RECT>> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        true.into()
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe { EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut handles as *mut _ as isize)) }
        .ok()
        .context("could not list the displays")?;
    let mut monitors = Vec::new();
    for handle in handles {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
            monitors.push(info);
        }
    }
    // MONITORINFOF_PRIMARY is 1; a stable sort keeps the rest in order.
    monitors.sort_by_key(|m| m.dwFlags & 1 == 0);
    Ok(monitors.into_iter().map(|m| m.rcMonitor).collect())
}

/// The screen's device context, released on drop.
struct Dc(HDC);

impl Dc {
    fn screen() -> Self {
        Self(unsafe { GetDC(None) })
    }
}

impl Drop for Dc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A bitmap selected into its own memory DC, both freed on drop.
struct Bitmap {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
}

impl Bitmap {
    fn compatible(screen: HDC, w: i32, h: i32) -> Result<Self> {
        let bitmap = unsafe { CreateCompatibleBitmap(screen, w, h) };
        if bitmap.is_invalid() {
            bail!("could not make a {w}x{h} bitmap");
        }
        Self::wrap(bitmap)
    }

    fn wrap(bitmap: HBITMAP) -> Result<Self> {
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
            bail!("could not make a drawing context");
        }
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        Ok(Self { dc, bitmap, previous })
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
