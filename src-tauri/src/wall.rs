//! Window management: per-display wallpaper windows, the full-screen viewer and
//! the settings window.

use crate::core::{self, Core};
use crate::displays::DisplayInfo;
use crate::platform;
use tauri::webview::Color;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const VIEWER: &str = "viewer";
pub const SETTINGS: &str = "settings";

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

/// Create, move or destroy wallpaper windows so there is exactly one per display.
pub fn reconcile(app: &AppHandle) {
    let core = app.state::<Core>();
    let (displays, live) = {
        let i = core.lock();
        (i.displays.clone(), i.settings.wallpaper.live && platform::live_wallpaper_supported())
    };
    let wanted: Vec<&DisplayInfo> = if live { displays.iter().collect() } else { Vec::new() };
    for (label, w) in app.webview_windows() {
        if label.starts_with("wall-") && !wanted.iter().any(|d| d.label == label) {
            let _ = w.destroy();
        }
    }
    for d in wanted {
        match app.get_webview_window(&d.label) {
            Some(w) => platform::place_desktop_window(&w, d),
            None => {
                if let Err(e) = create_wallpaper(app, d) {
                    log::error!("could not create wallpaper window for {}: {e}", d.name);
                }
            }
        }
    }
}

fn create_wallpaper(app: &AppHandle, d: &DisplayInfo) -> tauri::Result<()> {
    let url = WebviewUrl::App(format!("wallpaper.html?display={}", enc(&d.key)).into());
    let w = WebviewWindowBuilder::new(app, &d.label, url)
        .title("Galeria Wallpaper")
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .shadow(false)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .background_color(Color(14, 14, 14, 255))
        .position(d.x, d.y)
        .inner_size(d.width, d.height)
        .build()?;
    let _ = w.set_ignore_cursor_events(true);
    platform::prepare_desktop_window(&w, d);
    Ok(())
}

/// Arrange mode lifts the wallpaper windows above the desktop icons so the
/// plate can be dragged; leaving it drops them back behind everything.
pub fn set_arrange(app: &AppHandle, on: bool) {
    let core = app.state::<Core>();
    {
        let mut i = core.lock();
        if on && !(i.settings.wallpaper.live && platform::live_wallpaper_supported()) {
            return;
        }
        i.arrange = on;
    }
    let displays = core.lock().displays.clone();
    for (label, w) in app.webview_windows() {
        if let Some(d) = displays.iter().find(|d| d.label == label) {
            let _ = w.set_ignore_cursor_events(!on);
            platform::set_interactive(&w, d, on);
        }
    }
    core::publish(app);
}

fn display_under_cursor(app: &AppHandle, displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    let pos = app.cursor_position().ok()?;
    displays
        .iter()
        .find(|d| {
            let (x, y) = (pos.x / d.scale, pos.y / d.scale);
            x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height
        })
        .cloned()
}

pub fn open_viewer(app: &AppHandle, painting: Option<String>, display: Option<String>) -> Result<(), String> {
    let core = app.state::<Core>();
    let displays = core.lock().displays.clone();
    let d = display
        .and_then(|k| displays.iter().find(|d| d.key == k).cloned())
        .or_else(|| display_under_cursor(app, &displays))
        .or_else(|| displays.first().cloned())
        .ok_or("no display")?;
    let id = {
        let i = core.lock();
        painting
            .or_else(|| i.persisted.assignments.get(&d.key).map(|a| a.painting.clone()))
            .filter(|id| i.library.get(id).is_some())
            .or_else(|| i.library.paintings.first().map(|p| p.id.clone()))
            .ok_or("the library is empty")?
    };
    if let Some(w) = app.get_webview_window(VIEWER) {
        let _ = w.emit("viewer-show", &id);
        let _ = w.set_focus();
        return Ok(());
    }
    let url = WebviewUrl::App(format!("viewer.html?id={}&display={}", enc(&id), enc(&d.key)).into());
    let b = WebviewWindowBuilder::new(app, VIEWER, url)
        .title("Galeria")
        .decorations(false)
        .position(d.x, d.y)
        .inner_size(d.width, d.height)
        .background_color(Color(0, 0, 0, 255))
        .focused(true);
    // macOS: "simple" full screen — instant, stays in the current Space, hides
    // the menu bar and Dock, and avoids WebKit's title-bar content inset.
    #[cfg(not(target_os = "macos"))]
    let b = b.fullscreen(true);
    let w = b.build().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let _ = w.set_simple_fullscreen(true);
    let _ = w.set_focus();
    Ok(())
}

pub fn close_viewer(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(VIEWER) {
        #[cfg(target_os = "macos")]
        let _ = w.set_simple_fullscreen(false);
        let _ = w.close();
    }
}

pub fn open_settings(app: &AppHandle, section: Option<String>) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(SETTINGS) {
        if let Some(s) = section {
            let _ = w.emit("navigate", s);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let url = WebviewUrl::App(format!("settings.html#{}", section.unwrap_or_default()).into());
    #[allow(unused_mut)]
    let mut b = WebviewWindowBuilder::new(app, SETTINGS, url)
        .title("Galeria")
        .inner_size(1080.0, 760.0)
        .min_inner_size(880.0, 580.0)
        .center()
        .background_color(Color(24, 23, 22, 255))
        .focused(true);
    #[cfg(target_os = "macos")]
    {
        b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    }
    let w = b.build().map_err(|e| e.to_string())?;
    let _ = w.set_focus();
    Ok(())
}

/// Window creation from menu handlers must not run on the main thread on Windows.
pub fn spawn_open_settings(app: &AppHandle, section: Option<String>) {
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = open_settings(&a, section) {
            log::error!("settings window: {e}");
        }
    });
}

pub fn spawn_open_viewer(app: &AppHandle, painting: Option<String>, display: Option<String>) {
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = open_viewer(&a, painting, display) {
            log::error!("viewer window: {e}");
        }
    });
}
