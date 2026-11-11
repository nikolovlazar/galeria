//! Windows: parent the wallpaper window to the desktop's WorkerW (behind the
//! icons, the technique used by Lively / Wallpaper Engine) and use
//! IDesktopWallpaper for the static system wallpaper.
//!
//! NOTE: written against the documented Win32 APIs but not yet exercised on a
//! real Windows machine — treat as a starting point.

use super::SystemWallpaperOptions;
use crate::displays::DisplayInfo;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, WebviewWindow};
use windows::core::{BOOL, HSTRING, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{DesktopWallpaper, IDesktopWallpaper, DWPOS_FILL, DWPOS_FIT};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowExW, FindWindowW, GetSystemMetrics, SendMessageTimeoutW, SetParent, SetWindowPos,
    HWND_TOP, SMTO_NORMAL, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_SHOWWINDOW,
};

pub fn decorate_displays(_app: &AppHandle, _displays: &mut Vec<DisplayInfo>) {}

fn hwnd_of(window: &WebviewWindow) -> Option<HWND> {
    window.hwnd().ok().map(|h| HWND(h.0 as _))
}

unsafe extern "system" fn find_worker(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let shell = FindWindowExW(Some(hwnd), None, &HSTRING::from("SHELLDLL_DefView"), PCWSTR::null());
    if shell.map(|h| !h.is_invalid()).unwrap_or(false) {
        if let Ok(worker) = FindWindowExW(None, Some(hwnd), &HSTRING::from("WorkerW"), PCWSTR::null()) {
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
        SendMessageTimeoutW(progman, 0x052C, WPARAM(0xD), LPARAM(0x1), SMTO_NORMAL, 1000, Some(&mut _res));
        let mut worker = HWND::default();
        let _ = EnumWindows(Some(find_worker), LPARAM(&mut worker as *mut HWND as isize));
        if worker.is_invalid() {
            // Windows 11 24H2+: the WorkerW is a child of Progman.
            worker = FindWindowExW(Some(progman), None, &HSTRING::from("WorkerW"), PCWSTR::null()).unwrap_or_default();
        }
        (!worker.is_invalid()).then_some(worker)
    }
}

fn place(hwnd: HWND, d: &DisplayInfo, parented: bool) {
    unsafe {
        let (ox, oy) = if parented {
            (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN))
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

pub fn prepare_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(hwnd) = hwnd_of(&w) else { return };
        let parented = match desktop_worker() {
            Some(worker) => unsafe { SetParent(hwnd, Some(worker)).is_ok() },
            None => false,
        };
        place(hwnd, &d, parented);
        let _ = w.show();
    });
}

pub fn place_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        if let Some(hwnd) = hwnd_of(&w) {
            place(hwnd, &d, desktop_worker().is_some());
        }
    });
}

pub fn set_interactive(window: &WebviewWindow, d: &DisplayInfo, interactive: bool) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(hwnd) = hwnd_of(&w) else { return };
        unsafe {
            if interactive {
                let _ = SetParent(hwnd, None);
                place(hwnd, &d, false);
                let _ = w.set_always_on_top(true);
                let _ = w.set_focus();
            } else {
                let _ = w.set_always_on_top(false);
                let parented = match desktop_worker() {
                    Some(worker) => SetParent(hwnd, Some(worker)).is_ok(),
                    None => false,
                };
                place(hwnd, &d, parented);
            }
        }
    });
}

fn with_wallpaper<T>(f: impl FnOnce(&IDesktopWallpaper) -> windows::core::Result<T>) -> Result<T, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dw: IDesktopWallpaper = CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
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

pub fn set_system_wallpaper(d: &DisplayInfo, path: &Path, opts: SystemWallpaperOptions) -> Result<(), String> {
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
        let Some(id) = monitor_id(dw, d)? else { return Ok(None) };
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

pub fn capture_webview(_window: &WebviewWindow, _path: PathBuf) -> std::sync::mpsc::Receiver<Result<(), String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = tx.send(Err("window capture is only implemented on macOS".into()));
    rx
}

pub fn live_wallpaper_supported() -> bool {
    true
}
