//! Linux: on X11 the wallpaper window gets the `_NET_WM_WINDOW_TYPE_DESKTOP`
//! hint; the static wallpaper is set through whatever the desktop provides
//! (GNOME/Cinnamon/MATE gsettings, KDE, XFCE, sway, Hyprland/swww, feh).
//! Wayland compositors don't allow desktop-type windows, so live mode is
//! disabled there and Galeria falls back to the system wallpaper.
//!
//! NOTE: not yet exercised on a real Linux machine — treat as a starting point.

use super::SystemWallpaperOptions;
use crate::displays::DisplayInfo;
use gtk::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, WebviewWindow};

pub fn decorate_displays(_app: &AppHandle, _displays: &mut Vec<DisplayInfo>) {}

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
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let forced_x11 = std::env::var("GDK_BACKEND").map(|b| b.contains("x11")).unwrap_or(false);
    !wayland || forced_x11
}

pub fn prepare_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(gw) = w.gtk_window() else { return };
        gw.hide(); // the type hint only applies before the window is mapped
        gw.set_type_hint(gtk::gdk::WindowTypeHint::Desktop);
        gw.set_keep_below(true);
        gw.set_skip_taskbar_hint(true);
        gw.set_skip_pager_hint(true);
        gw.set_accept_focus(false);
        gw.set_decorated(false);
        gw.stick();
        gw.move_(d.x as i32, d.y as i32);
        gw.resize(d.width as i32, d.height as i32);
        gw.show_all();
    });
}

pub fn place_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(gw) = w.gtk_window() {
            gw.move_(d.x as i32, d.y as i32);
            gw.resize(d.width as i32, d.height as i32);
        }
    });
}

pub fn set_interactive(window: &WebviewWindow, d: &DisplayInfo, interactive: bool) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Ok(gw) = w.gtk_window() else { return };
        gw.hide();
        if interactive {
            gw.set_type_hint(gtk::gdk::WindowTypeHint::Normal);
            gw.set_keep_below(false);
            gw.set_keep_above(true);
            gw.set_accept_focus(true);
            gw.show_all();
            gw.present();
        } else {
            gw.set_type_hint(gtk::gdk::WindowTypeHint::Desktop);
            gw.set_keep_above(false);
            gw.set_keep_below(true);
            gw.set_accept_focus(false);
            gw.show_all();
        }
        gw.move_(d.x as i32, d.y as i32);
        gw.resize(d.width as i32, d.height as i32);
    });
}

fn has(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd).args(args).status().map_err(|e| format!("{cmd}: {e}"))?;
    status.success().then_some(()).ok_or_else(|| format!("{cmd} exited with {status}"))
}

fn hex((r, g, b): (f64, f64, f64)) -> String {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

pub fn set_system_wallpaper(_d: &DisplayInfo, path: &Path, opts: SystemWallpaperOptions) -> Result<(), String> {
    let p = path.to_string_lossy().to_string();
    let uri = format!("file://{p}");
    let color = hex(opts.color);
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();

    if desktop.contains("gnome") || desktop.contains("unity") || desktop.contains("budgie") || desktop.contains("pantheon") {
        let mode = if opts.fill { "zoom" } else { "scaled" };
        run("gsettings", &["set", "org.gnome.desktop.background", "picture-uri", &uri])?;
        let _ = run("gsettings", &["set", "org.gnome.desktop.background", "picture-uri-dark", &uri]);
        let _ = run("gsettings", &["set", "org.gnome.desktop.background", "picture-options", mode]);
        let _ = run("gsettings", &["set", "org.gnome.desktop.background", "primary-color", &color]);
        return Ok(());
    }
    if desktop.contains("cinnamon") {
        return run("gsettings", &["set", "org.cinnamon.desktop.background", "picture-uri", &uri]);
    }
    if desktop.contains("mate") {
        return run("gsettings", &["set", "org.mate.background", "picture-filename", &p]);
    }
    if desktop.contains("kde") && has("plasma-apply-wallpaperimage") {
        return run("plasma-apply-wallpaperimage", &[&p]);
    }
    if desktop.contains("xfce") {
        let out = Command::new("xfconf-query").args(["-c", "xfce4-desktop", "-l"]).output().map_err(|e| e.to_string())?;
        for prop in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.ends_with("/last-image")) {
            let _ = run("xfconf-query", &["-c", "xfce4-desktop", "-p", prop, "-s", &p]);
        }
        return Ok(());
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() || has("swww") {
        if has("swww") {
            let resize = if opts.fill { "crop" } else { "fit" };
            let fill = color.trim_start_matches('#').to_string();
            return run("swww", &["img", &p, "--resize", resize, "--fill-color", &fill]);
        }
        if has("hyprctl") {
            let _ = run("hyprctl", &["hyprpaper", "preload", &p]);
            return run("hyprctl", &["hyprpaper", "wallpaper", &format!(",{p}")]);
        }
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        let mode = if opts.fill { "fill" } else { "fit" };
        return run("swaymsg", &["output", "*", "bg", &p, mode, &color]);
    }
    if has("feh") {
        let mode = if opts.fill { "--bg-fill" } else { "--bg-max" };
        return run("feh", &["--image-bg", &color, mode, &p]);
    }
    Err("don't know how to set the wallpaper on this desktop (install feh or swww)".into())
}

pub fn current_system_wallpaper(_d: &DisplayInfo) -> Option<PathBuf> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.background", "picture-uri"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().trim_matches('\'').to_string();
    s.strip_prefix("file://").map(PathBuf::from)
}
