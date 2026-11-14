//! Command-line control. Running `galeria <flag>` while Galeria is already
//! running forwards the flag to the running instance (see `instance.rs`),
//! which makes it easy to bind hotkeys with Raycast, skhd, etc.
//!
//!   --next / --prev            change the painting on every display
//!   --show <id>                show one painting now (its manifest id, e.g. `nga:100922`)
//!   --viewer                   open the full-screen viewer
//!   --close-viewer
//!   --settings [section]       open settings (now, browse, discover, sources, wallpaper, plate, rotation, library, general)
//!   --toggle-plate             show / hide the info plate
//!   --toggle-panning           pause / resume panning
//!   --arrange                  drag the plate on the desktop
//!   --done                     leave arrange mode
//!   --menu <id>                trigger a menu-bar item by id (e.g. `style:glass`, `fit:pan`, `speed:10`)
//!   --theme light|dark|auto    pretend the system is in that appearance (testing light/dark matching)
//!   --debug                    log what is showing and why
//!   --quit
//!
//! `galeria ui …` is separate: it renders the windows without opening them
//! (snapshots, element trees, scripted input) and never reaches a running instance.

use crate::core;
use crate::host::AppHandle;
use serde_json::json;

/// Carry out the flags in `args` (the first entry is the program name).
/// Returns false when there was nothing Galeria understood.
pub fn handle(app: &AppHandle, args: &[String]) -> bool {
    let mut it = args.iter().skip(1).peekable();
    let mut handled = false;
    while let Some(a) = it.next() {
        handled = true;
        let next_value = |it: &mut std::iter::Peekable<
            std::iter::Skip<std::slice::Iter<'_, String>>,
        >| { it.next_if(|v| !v.starts_with("--")).cloned() };
        let shell = app.shell();
        match a.as_str() {
            "--next" => core::advance(app, None, true),
            "--prev" | "--previous" => core::advance(app, None, false),
            "--viewer" => {
                if let Some(s) = shell {
                    s.open_viewer(None, None);
                }
            }
            "--show" => {
                if let Some(id) = next_value(&mut it) {
                    if let Err(e) = core::show(app, &id, None) {
                        log::warn!("--show {id}: {e}");
                    }
                }
            }
            "--settings" => {
                let section = next_value(&mut it);
                if let Some(s) = shell {
                    s.open_settings(section);
                }
            }
            "--toggle-plate" => {
                let v = app.core().lock().settings.plate.visible;
                let _ = core::patch_settings(app, &json!({ "plate": { "visible": !v } }));
            }
            "--toggle-panning" => {
                let v = app.core().lock().settings.wallpaper.pan_paused;
                let _ = core::patch_settings(app, &json!({ "wallpaper": { "panPaused": !v } }));
            }
            "--arrange" => core::set_arrange(app, true),
            "--done" => core::set_arrange(app, false),
            "--menu" => {
                // Trigger any menu-bar item by id, e.g. `--menu style:glass`.
                if let Some(id) = next_value(&mut it) {
                    crate::menu::on_menu(app, &id);
                }
            }
            "--debug" => log::info!("[core] {}", core::debug_summary(app)),
            "--close-viewer" => {
                if let Some(s) = shell {
                    s.close_viewer();
                }
            }
            "--theme" => match next_value(&mut it).as_deref() {
                Some("dark") => core::force_dark(app, Some(true)),
                Some("light") => core::force_dark(app, Some(false)),
                _ => core::force_dark(app, None),
            },
            "--quit" => core::quit(app),
            crate::autostart::FLAG => handled = false,
            _ => {
                handled = false;
                log::debug!("ignoring argument {a}");
            }
        }
    }
    handled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_change_settings() {
        let dir = std::env::temp_dir().join(format!("galeria-cli-{}", fastrand::u64(..)));
        let app = AppHandle::new(core::Core::new(core::Paths::in_dir(&dir).unwrap()));
        let args = |a: &[&str]| -> Vec<String> {
            std::iter::once("galeria")
                .chain(a.iter().copied())
                .map(String::from)
                .collect()
        };
        assert!(handle(&app, &args(&["--toggle-plate"])));
        assert!(!app.core().lock().settings.plate.visible);
        assert!(handle(
            &app,
            &args(&["--menu", "frame:gilded", "--toggle-panning"])
        ));
        {
            let i = app.core().lock();
            assert_eq!(
                i.settings.wallpaper.frame,
                crate::settings::FrameStyle::Gilded
            );
            assert!(i.settings.wallpaper.pan_paused);
        }
        assert!(handle(&app, &args(&["--theme", "dark"])));
        assert_eq!(app.core().lock().dark_forced, Some(true));
        // A plain launch, or one at login, is not a command.
        assert!(!handle(&app, &args(&[])));
        assert!(!handle(&app, &args(&["--autostart"])));
        std::fs::remove_dir_all(&dir).ok();
    }
}
