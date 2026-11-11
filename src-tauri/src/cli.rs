//! Command-line control. Running `galeria <flag>` while Galeria is already
//! running forwards the flag to the running instance (via the single-instance
//! plugin), which makes it easy to bind hotkeys with Raycast, skhd, etc.
//!
//!   --next / --prev            change the painting on every display
//!   --show <id>                show one painting now (its manifest id, e.g. `nga:100922`)
//!   --viewer                   open the full-screen viewer
//!   --settings [section]       open settings (now, browse, discover, sources, wallpaper, plate, rotation, library, general)
//!   --toggle-plate             show / hide the info plate
//!   --toggle-panning           pause / resume panning
//!   --arrange                  drag the plate on the desktop
//!   --done                     leave arrange mode
//!   --menu <id>                trigger a menu-bar item by id (e.g. `style:glass`, `fit:pan`, `speed:10`)
//!   --theme light|dark|auto    pretend the system is in that appearance (testing light/dark matching)
//!   --ui click:<css selector>  click an element in the settings window (testing without a mouse)
//!   --ui type:<selector>=<text>
//!   --capture <dir>            write PNG snapshots of every Galeria window (macOS, debugging)
//!   --debug                    every window logs its render state to the app log

use crate::core::{self, Core};
use crate::{platform, wall};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn handle(app: &AppHandle, args: &[String]) -> bool {
    let mut it = args.iter().skip(1).peekable();
    let mut handled = false;
    while let Some(a) = it.next() {
        handled = true;
        let next_value = |it: &mut std::iter::Peekable<
            std::iter::Skip<std::slice::Iter<'_, String>>,
        >| { it.next_if(|v| !v.starts_with("--")).cloned() };
        match a.as_str() {
            "--next" => core::advance(app, None, true),
            "--prev" | "--previous" => core::advance(app, None, false),
            "--viewer" => wall::spawn_open_viewer(app, None, None),
            "--show" => {
                if let Some(id) = next_value(&mut it) {
                    if let Err(e) = core::show(app, &id, None) {
                        log::warn!("--show {id}: {e}");
                    }
                }
            }
            "--settings" => {
                let section = next_value(&mut it);
                wall::spawn_open_settings(app, section)
            }
            "--toggle-plate" => {
                let v = app.state::<Core>().lock().settings.plate.visible;
                let _ =
                    core::patch_settings(app, &serde_json::json!({ "plate": { "visible": !v } }));
            }
            "--toggle-panning" => {
                let v = app.state::<Core>().lock().settings.wallpaper.pan_paused;
                let _ = core::patch_settings(
                    app,
                    &serde_json::json!({ "wallpaper": { "panPaused": !v } }),
                );
            }
            "--arrange" => wall::set_arrange(app, true),
            "--done" => wall::set_arrange(app, false),
            "--menu" => {
                // Trigger any menu-bar item by id, e.g. `--menu style:glass`.
                if let Some(id) = next_value(&mut it) {
                    crate::tray::on_menu(app, &id);
                }
            }
            "--debug" => {
                use tauri::Emitter;
                let _ = app.emit("debug-dump", ());
                log::info!("[core] {}", core::debug_summary(app));
            }
            "--close-viewer" => wall::close_viewer(app),
            "--theme" => match next_value(&mut it).as_deref() {
                Some("dark") => core::force_dark(app, Some(true)),
                Some("light") => core::force_dark(app, Some(false)),
                _ => core::force_dark(app, None),
            },
            "--ui" => {
                use tauri::Emitter;
                if let Some(v) = next_value(&mut it) {
                    let _ = app.emit("debug-ui", v);
                }
            }
            "--debug-pointer" => {
                // `--debug-pointer [down|up:]0.1,0.5` synthesizes a pointer event in every
                // window (fractions of the window size) for testing without a mouse.
                use tauri::Emitter;
                if let Some(v) = next_value(&mut it) {
                    let (kind, xy) = v.split_once(':').unwrap_or(("move", v.as_str()));
                    if let Some((x, y)) = xy.split_once(',') {
                        let p = (
                            kind.to_string(),
                            x.parse::<f64>().unwrap_or(0.5),
                            y.parse::<f64>().unwrap_or(0.5),
                        );
                        let _ = app.emit("debug-pointer", p);
                    }
                }
            }
            "--capture" => {
                let dir = PathBuf::from(next_value(&mut it).unwrap_or_else(|| ".".into()));
                capture_all(app, dir);
            }
            "--autostart" => handled = false,
            _ => {
                handled = false;
                log::debug!("ignoring argument {a}");
            }
        }
    }
    handled
}

fn capture_all(app: &AppHandle, dir: PathBuf) {
    let _ = std::fs::create_dir_all(&dir);
    for (label, w) in app.webview_windows() {
        let path = dir.join(format!("{label}.png"));
        let rx = platform::capture_webview(&w, path.clone());
        std::thread::spawn(
            move || match rx.recv_timeout(std::time::Duration::from_secs(20)) {
                Ok(Ok(())) => log::info!("captured {label} → {}", path.display()),
                Ok(Err(e)) => log::warn!("capture {label}: {e}"),
                Err(_) => log::warn!("capture {label}: timed out"),
            },
        );
    }
}
