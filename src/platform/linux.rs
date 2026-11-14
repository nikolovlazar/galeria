//! Linux: on X11 the wallpaper window gets the `_NET_WM_WINDOW_TYPE_DESKTOP`
//! hint; the static wallpaper is set through whatever the desktop provides
//! (GNOME/Cinnamon/MATE gsettings, KDE, XFCE, sway, Hyprland/swww, feh).
//! Wayland compositors don't allow desktop-type windows, so live mode is
//! disabled there and Galeria falls back to the system wallpaper.
//!
//! NOTE: not yet exercised on a real Linux machine — treat as a starting point.

use super::SystemWallpaperOptions;
use crate::displays::{DisplayInfo, Insets};
use rune::raw_window_handle::RawWindowHandle;
use std::path::{Path, PathBuf};
use std::process::Command;
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as _, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub fn backup_wallpaper_store(_dest: &Path) -> bool {
    false
}

pub fn restore_wallpaper_store(_backup: &Path) -> Result<bool, String> {
    Ok(false)
}

/// Desktops differ too much to ask directly; the windows report what Rune
/// learns from the desktop's settings portal instead (see `ui::report_dark`).
pub fn system_dark() -> Option<bool> {
    None
}

fn wayland() -> bool {
    let forced_x11 = std::env::var("WINIT_UNIX_BACKEND")
        .map(|b| b.eq_ignore_ascii_case("x11"))
        .unwrap_or(false);
    std::env::var_os("WAYLAND_DISPLAY").is_some() && !forced_x11
}

pub fn live_wallpaper_supported() -> bool {
    !wayland()
}

fn connect() -> Option<(RustConnection, u32)> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    Some((conn, root))
}

fn atom(conn: &RustConnection, name: &str) -> Option<u32> {
    Some(
        conn.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom,
    )
}

/// `Xft.dpi` from the X resource database, as a scale factor (96 dpi = 1).
fn xft_scale(conn: &RustConnection, root: u32) -> f64 {
    let resources = conn
        .get_property(
            false,
            root,
            AtomEnum::RESOURCE_MANAGER,
            AtomEnum::STRING,
            0,
            1 << 20,
        )
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| String::from_utf8_lossy(&r.value).into_owned())
        .unwrap_or_default();
    resources
        .lines()
        .find_map(|l| l.strip_prefix("Xft.dpi:"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|dpi| (dpi / 96.0).clamp(0.5, 4.0))
        .unwrap_or(1.0)
}

/// Every monitor RandR knows (through XWayland too, where it runs).
pub fn list_displays() -> Vec<DisplayInfo> {
    let found = x11_displays();
    if !found.is_empty() {
        return found;
    }
    // No X server to ask (a Wayland session without XWayland). Rotation and the
    // system wallpaper still need something to hang paintings on.
    vec![DisplayInfo {
        key: "Display".into(),
        name: "Display".into(),
        label: String::new(),
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
        physical_x: 0,
        physical_y: 0,
        physical_width: 1920,
        physical_height: 1080,
        scale: 1.0,
        primary: true,
        insets: Insets::default(),
    }]
}

fn x11_displays() -> Vec<DisplayInfo> {
    let Some((conn, root)) = connect() else {
        return Vec::new();
    };
    let scale = xft_scale(&conn, root);
    let Some(reply) = conn
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return Vec::new();
    };
    reply
        .monitors
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let name = conn
                .get_atom_name(m.name)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| String::from_utf8_lossy(&r.name).into_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Display {}", i + 1));
            DisplayInfo {
                key: name.clone(),
                name,
                label: String::new(),
                x: m.x as f64 / scale,
                y: m.y as f64 / scale,
                width: m.width as f64 / scale,
                height: m.height as f64 / scale,
                physical_x: m.x as i32,
                physical_y: m.y as i32,
                physical_width: m.width as u32,
                physical_height: m.height as u32,
                scale,
                primary: m.primary,
                insets: Insets::default(),
            }
        })
        .collect()
}

pub fn display_under_pointer(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    let (conn, root) = connect()?;
    let p = conn.query_pointer(root).ok()?.reply().ok()?;
    let (x, y) = (p.root_x as i32, p.root_y as i32);
    displays
        .iter()
        .find(|d| {
            x >= d.physical_x
                && x < d.physical_x + d.physical_width as i32
                && y >= d.physical_y
                && y < d.physical_y + d.physical_height as i32
        })
        .cloned()
}

fn xid(window: &rune::WindowHandle) -> Option<u32> {
    window
        .with_raw_handle(|handle| match handle.as_raw() {
            RawWindowHandle::Xlib(h) => Some(h.window as u32),
            RawWindowHandle::Xcb(h) => Some(h.window.get()),
            _ => None,
        })
        .ok()
        .flatten()
}

/// Window type and stacking hints. Window managers read them when the window is
/// mapped, so callers hide the window first and show it again afterwards.
fn set_hints(window: &rune::WindowHandle, d: &DisplayInfo, desktop: bool) -> Option<()> {
    let win = xid(window)?;
    let (conn, _root) = connect()?;
    let kind = if desktop {
        "_NET_WM_WINDOW_TYPE_DESKTOP"
    } else {
        "_NET_WM_WINDOW_TYPE_NORMAL"
    };
    let states: &[&str] = if desktop {
        &[
            "_NET_WM_STATE_BELOW",
            "_NET_WM_STATE_STICKY",
            "_NET_WM_STATE_SKIP_TASKBAR",
            "_NET_WM_STATE_SKIP_PAGER",
        ]
    } else {
        &["_NET_WM_STATE_ABOVE", "_NET_WM_STATE_STICKY"]
    };
    let type_atom = atom(&conn, "_NET_WM_WINDOW_TYPE")?;
    let state_atom = atom(&conn, "_NET_WM_STATE")?;
    let kind = atom(&conn, kind)?;
    let states: Vec<u32> = states.iter().filter_map(|s| atom(&conn, s)).collect();
    conn.change_property32(PropMode::REPLACE, win, type_atom, AtomEnum::ATOM, &[kind])
        .ok()?;
    conn.change_property32(PropMode::REPLACE, win, state_atom, AtomEnum::ATOM, &states)
        .ok()?;
    conn.configure_window(
        win,
        &ConfigureWindowAux::new()
            .x(d.physical_x)
            .y(d.physical_y)
            .width(d.physical_width)
            .height(d.physical_height),
    )
    .ok()?;
    conn.flush().ok()
}

/// How a wallpaper window is created: borderless and hidden until its hints are set.
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
    // The type hint only applies before the window is mapped.
    let _ = set_hints(window, d, true);
    window.show();
}

pub fn place_desktop_window(window: &rune::WindowHandle, d: &DisplayInfo) {
    let Some(win) = xid(window) else { return };
    let Some((conn, _root)) = connect() else {
        return;
    };
    let _ = conn.configure_window(
        win,
        &ConfigureWindowAux::new()
            .x(d.physical_x)
            .y(d.physical_y)
            .width(d.physical_width)
            .height(d.physical_height),
    );
    let _ = conn.flush();
}

pub fn set_interactive(window: &rune::WindowHandle, d: &DisplayInfo, interactive: bool) {
    let _ = window.set_click_through(!interactive);
    window.hide();
    let _ = set_hints(window, d, !interactive);
    window.show();
    if interactive {
        window.focus();
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

/// Cover the display and take the keyboard.
pub fn enter_viewer(window: &rune::WindowHandle, d: &DisplayInfo) {
    place_desktop_window(window, d);
    window.set_always_on_top(true);
    window.focus();
}

pub fn leave_viewer() {}

fn has(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(cmd)
        .args(args)
        .status()
        .map_err(|e| format!("{cmd}: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{cmd} exited with {status}"))
}

fn hex((r, g, b): (f64, f64, f64)) -> String {
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

pub fn set_system_wallpaper(
    _d: &DisplayInfo,
    path: &Path,
    opts: SystemWallpaperOptions,
) -> Result<(), String> {
    let p = path.to_string_lossy().to_string();
    let uri = format!("file://{p}");
    let color = hex(opts.color);
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();

    if desktop.contains("gnome")
        || desktop.contains("unity")
        || desktop.contains("budgie")
        || desktop.contains("pantheon")
    {
        let mode = if opts.fill { "zoom" } else { "scaled" };
        run(
            "gsettings",
            &["set", "org.gnome.desktop.background", "picture-uri", &uri],
        )?;
        let _ = run(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.background",
                "picture-uri-dark",
                &uri,
            ],
        );
        let _ = run(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.background",
                "picture-options",
                mode,
            ],
        );
        let _ = run(
            "gsettings",
            &[
                "set",
                "org.gnome.desktop.background",
                "primary-color",
                &color,
            ],
        );
        return Ok(());
    }
    if desktop.contains("cinnamon") {
        return run(
            "gsettings",
            &[
                "set",
                "org.cinnamon.desktop.background",
                "picture-uri",
                &uri,
            ],
        );
    }
    if desktop.contains("mate") {
        return run(
            "gsettings",
            &["set", "org.mate.background", "picture-filename", &p],
        );
    }
    if desktop.contains("kde") && has("plasma-apply-wallpaperimage") {
        return run("plasma-apply-wallpaperimage", &[&p]);
    }
    if desktop.contains("xfce") {
        let out = Command::new("xfconf-query")
            .args(["-c", "xfce4-desktop", "-l"])
            .output()
            .map_err(|e| e.to_string())?;
        for prop in String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.ends_with("/last-image"))
        {
            let _ = run(
                "xfconf-query",
                &["-c", "xfce4-desktop", "-p", prop, "-s", &p],
            );
        }
        return Ok(());
    }
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() || has("swww") {
        if has("swww") {
            let resize = if opts.fill { "crop" } else { "fit" };
            let fill = color.trim_start_matches('#').to_string();
            return run(
                "swww",
                &["img", &p, "--resize", resize, "--fill-color", &fill],
            );
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
    let s = String::from_utf8_lossy(&out.stdout)
        .trim()
        .trim_matches('\'')
        .to_string();
    s.strip_prefix("file://").map(PathBuf::from)
}

pub fn screen_saver_supported() -> bool {
    false
}

pub fn set_screen_saver(_folder: &Path) -> Result<(), String> {
    Err("not supported on this system".into())
}
