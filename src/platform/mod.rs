//! Platform glue. Each OS module implements the same small surface:
//!
//! - `list_displays`             every screen, with friendly names and safe-area insets
//! - `display_under_pointer`
//! - `desktop_window_options`    how a wallpaper window is created
//! - `prepare_desktop_window`    turn a Rune window into a desktop-level wallpaper
//! - `place_desktop_window`      re-fit it to its display
//! - `set_interactive`           temporarily raise it so the plate can be dragged
//! - `viewer_window_options`, `enter_viewer`, `leave_viewer`   the full-screen viewer
//! - `set_system_wallpaper` / `current_system_wallpaper`
//! - `backup_wallpaper_store` / `restore_wallpaper_store`
//! - `screen_saver_supported` / `set_screen_saver`   macOS only
//! - `system_dark`, `live_wallpaper_supported`
//!
//! Everything that touches a window or a screen runs on the UI thread.

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
