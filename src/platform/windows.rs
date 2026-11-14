//! Windows: parent the wallpaper window to the desktop's WorkerW (behind the
//! icons, the technique used by Lively / Wallpaper Engine) and use
//! IDesktopWallpaper for the static system wallpaper.
//!
//! NOTE: written against the documented Win32 APIs but not yet exercised on a
//! real Windows machine — treat as a starting point.

use super::SystemWallpaperOptions;
use crate::displays::{DisplayInfo, Insets};
use rune::raw_window_handle::RawWindowHandle;
use std::path::{Path, PathBuf};
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_FILL, DWPOS_FIT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetCursorPos, GetSystemMetrics, SendMessageTimeoutW,
    SetParent, SetWindowPos, HWND_TOP, SMTO_NORMAL, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_SHOWWINDOW,
};

/// `MONITORINFO::dwFlags`: this is the primary display.
const MONITORINFOF_PRIMARY: u32 = 1;

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<DisplayInfo>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if !GetMonitorInfoW(
        monitor,
        &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
    )
    .as_bool()
    {
        return BOOL(1);
    }
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    let scale = (dpi_x as f64 / 96.0).max(0.5);
    let (m, w) = (info.monitorInfo.rcMonitor, info.monitorInfo.rcWork);
    let len = info
        .szDevice
        .iter()
        .position(|c| *c == 0)
        .unwrap_or(info.szDevice.len());
    let device = String::from_utf16_lossy(&info.szDevice[..len]);
    let name = if device.is_empty() {
        format!("Display {}", out.len() + 1)
    } else {
        device
    };
    let px = |v: i32| (v.max(0) as f64) / scale;
    out.push(DisplayInfo {
        key: name.clone(),
        name,
        label: String::new(),
        x: m.left as f64 / scale,
        y: m.top as f64 / scale,
        width: (m.right - m.left) as f64 / scale,
        height: (m.bottom - m.top) as f64 / scale,
        physical_x: m.left,
        physical_y: m.top,
        physical_width: (m.right - m.left).max(0) as u32,
        physical_height: (m.bottom - m.top).max(0) as u32,
        scale,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        insets: Insets {
            top: px(w.top - m.top),
            left: px(w.left - m.left),
            bottom: px(m.bottom - w.bottom),
            right: px(m.right - w.right),
        },
    });
    BOOL(1)
}

/// Every monitor, with the area the taskbar covers.
pub fn list_displays() -> Vec<DisplayInfo> {
    let mut out: Vec<DisplayInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut out as *mut Vec<DisplayInfo> as isize),
        );
    }
    out
}

pub fn display_under_pointer(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    displays
        .iter()
        .find(|d| {
            p.x >= d.physical_x
                && p.x < d.physical_x + d.physical_width as i32
                && p.y >= d.physical_y
                && p.y < d.physical_y + d.physical_height as i32
        })
        .cloned()
}

fn hwnd_of(window: &rune::WindowHandle) -> Option<HWND> {
    window
        .with_raw_handle(|handle| match handle.as_raw() {
            RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as _)),
            _ => None,
        })
        .ok()
        .flatten()
}

unsafe extern "system" fn find_worker(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let shell = FindWindowExW(
        Some(hwnd),
        None,
        &HSTRING::from("SHELLDLL_DefView"),
        PCWSTR::null(),
    );
    if shell.map(|h| !h.is_invalid()).unwrap_or(false) {
        if let Ok(worker) =
            FindWindowExW(None, Some(hwnd), &HSTRING::from("WorkerW"), PCWSTR::null())
        {
            *(lparam.0 as *mut HWND) = worker;
        }
    }
    BOOL(1)
}

/// Find (or ask Explorer to create) the window that sits between the wallpaper and the icons.
fn desktop_worker() -> Option<HWND> {
    unsafe {
        let progman = FindWindowW(&HSTRING::from("Progman"), PCWSTR::null()).ok()?;
        let mut _res = 0usize;
        SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0xD),
            LPARAM(0x1),
            SMTO_NORMAL,
            1000,
            Some(&mut _res),
        );
        let mut worker = HWND::default();
        let _ = EnumWindows(Some(find_worker), LPARAM(&mut worker as *mut HWND as isize));
        if worker.is_invalid() {
            // Windows 11 24H2+: the WorkerW is a child of Progman.
            worker = FindWindowExW(
                Some(progman),
                None,
                &HSTRING::from("WorkerW"),
                PCWSTR::null(),
            )
            .unwrap_or_default();
        }
        (!worker.is_invalid()).then_some(worker)
    }
}

fn place(hwnd: HWND, d: &DisplayInfo, parented: bool) {
    unsafe {
        let (ox, oy) = if parented {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
            )
        } else {
            (0, 0)
        };
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            d.physical_x - ox,
            d.physical_y - oy,
            d.physical_width as i32,
            d.physical_height as i32,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// How a wallpaper window is created: borderless and hidden until it is parented to the desktop.
pub fn desktop_window_options(d: &DisplayInfo) -> rune::ConfiguredWindowOptions {
    rune::WindowOptions {
        borderless: true,
        click_through: true,
        resizable: false,
        visible: false,
        position: Some((d.physical_x, d.physical_y)),
        physical_size: Some((d.physical_width, d.physical_height)),
        min_size: Some((0.0, 0.0)),
        ..Default::default()
    }
    .into()
}

pub fn prepare_desktop_window(window: &rune::WindowHandle, d: &DisplayInfo) {
    let Some(hwnd) = hwnd_of(window) else { return };
    let parented = match desktop_worker() {
        Some(worker) => unsafe { SetParent(hwnd, Some(worker)).is_ok() },
        None => false,
    };
    place(hwnd, d, parented);
    window.show();
}

pub fn place_desktop_window(window: &rune::WindowHandle, d: &DisplayInfo) {
    if let Some(hwnd) = hwnd_of(window) {
        place(hwnd, d, desktop_worker().is_some());
    }
}

pub fn set_interactive(window: &rune::WindowHandle, d: &DisplayInfo, interactive: bool) {
    let Some(hwnd) = hwnd_of(window) else { return };
    let _ = window.set_click_through(!interactive);
    unsafe {
        if interactive {
            let _ = SetParent(hwnd, None);
            place(hwnd, d, false);
            window.set_always_on_top(true);
            window.focus();
        } else {
            window.set_always_on_top(false);
            let parented = match desktop_worker() {
                Some(worker) => SetParent(hwnd, Some(worker)).is_ok(),
                None => false,
            };
            place(hwnd, d, parented);
        }
    }
}

/// How the viewer window is created: a borderless window the size of its display.
pub fn viewer_window_options(d: &DisplayInfo) -> rune::ConfiguredWindowOptions {
    rune::WindowOptions {
        borderless: true,
        resizable: false,
        visible: false,
        position: Some((d.physical_x, d.physical_y)),
        physical_size: Some((d.physical_width, d.physical_height)),
        min_size: Some((0.0, 0.0)),
        ..Default::default()
    }
    .into()
}

/// Cover the display, taskbar included, and take the keyboard.
pub fn enter_viewer(window: &rune::WindowHandle, d: &DisplayInfo) {
    if let Some(hwnd) = hwnd_of(window) {
        place(hwnd, d, false);
    }
    window.set_always_on_top(true);
    window.focus();
}

pub fn leave_viewer() {}

fn with_wallpaper<T>(
    f: impl FnOnce(&IDesktopWallpaper) -> windows::core::Result<T>,
) -> Result<T, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dw: IDesktopWallpaper =
            CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        f(&dw).map_err(|e| e.to_string())
    }
}

fn monitor_id(dw: &IDesktopWallpaper, d: &DisplayInfo) -> windows::core::Result<Option<HSTRING>> {
    unsafe {
        let n = dw.GetMonitorDevicePathCount()?;
        for i in 0..n {
            let id = dw.GetMonitorDevicePathAt(i)?;
            let s = HSTRING::from_wide(id.as_wide());
            CoTaskMemFree(Some(id.0 as _));
            if let Ok(r) = dw.GetMonitorRECT(&s) {
                if r.left == d.physical_x && r.top == d.physical_y {
                    return Ok(Some(s));
                }
            }
        }
        Ok(None)
    }
}

pub fn set_system_wallpaper(
    d: &DisplayInfo,
    path: &Path,
    opts: SystemWallpaperOptions,
) -> Result<(), String> {
    with_wallpaper(|dw| unsafe {
        let (r, g, b) = opts.color;
        let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        dw.SetBackgroundColor(COLORREF(to8(r) | (to8(g) << 8) | (to8(b) << 16)))?;
        dw.SetPosition(if opts.fill { DWPOS_FILL } else { DWPOS_FIT })?;
        let target = monitor_id(dw, d)?;
        let file = HSTRING::from(path.as_os_str());
        match target {
            Some(id) => dw.SetWallpaper(&id, &file),
            None => dw.SetWallpaper(PCWSTR::null(), &file),
        }
    })
}

pub fn current_system_wallpaper(d: &DisplayInfo) -> Option<PathBuf> {
    with_wallpaper(|dw| unsafe {
        let Some(id) = monitor_id(dw, d)? else {
            return Ok(None);
        };
        let p = dw.GetWallpaper(&id)?;
        let s = String::from_utf16_lossy(p.as_wide());
        CoTaskMemFree(Some(p.0 as _));
        Ok((!s.is_empty()).then(|| PathBuf::from(s)))
    })
    .ok()
    .flatten()
}

pub fn backup_wallpaper_store(_dest: &Path) -> bool {
    false
}

pub fn restore_wallpaper_store(_backup: &Path) -> Result<bool, String> {
    Ok(false)
}

/// Whether apps are set to the dark theme (Settings → Personalization → Colors).
pub fn system_dark() -> Option<bool> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut light, mut size) = (1u32, std::mem::size_of::<u32>() as u32);
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            &HSTRING::from("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut light as *mut u32 as *mut core::ffi::c_void),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(light == 0)
}

pub fn live_wallpaper_supported() -> bool {
    true
}

pub fn screen_saver_supported() -> bool {
    false
}

pub fn set_screen_saver(_folder: &Path) -> Result<(), String> {
    Err("not supported on this system".into())
}
