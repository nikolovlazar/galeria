//! Platform glue. Each OS module implements the same small surface:
//!
//! - `decorate_displays`   friendly names / precise safe-area insets
//! - `prepare_desktop_window` turn a webview window into a desktop-level wallpaper
//! - `place_desktop_window`   re-fit it to its display
//! - `set_interactive`        temporarily raise it so the plate can be dragged
//! - `set_system_wallpaper` / `current_system_wallpaper`
//! - `live_wallpaper_supported`

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

/// How the operating system should scale the static wallpaper image.
#[derive(Debug, Clone, Copy)]
pub struct SystemWallpaperOptions {
    /// true = fill the screen (crop); false = fit the whole image.
    pub fill: bool,
    /// Background colour around a fitted image, 0..1 sRGB.
    pub color: (f64, f64, f64),
}

pub const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};

/// Run `f` on the main thread and wait for its result. Never call this while
/// holding the core lock.
pub fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    if is_main_thread() {
        return Some(f());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(10)).ok()
}

#[cfg(target_os = "macos")]
fn is_main_thread() -> bool {
    objc2::MainThreadMarker::new().is_some()
}

#[cfg(not(target_os = "macos"))]
fn is_main_thread() -> bool {
    // tao runs the event loop on the thread that started the app.
    std::thread::current().name() == Some("main")
}
