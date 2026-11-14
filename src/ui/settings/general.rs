//! General: the app itself, the viewer, and what Galeria is.

use super::widgets::{self, Kind};
use super::{app_icon, scroll_page, SettingsUi};
use crate::core;
use crate::ui::theme::Look;
use rune::{col, el, row, text, El, TextStyle};
use serde_json::json;

fn feel(v: f64) -> String {
    if v < 0.2 {
        "Direct".into()
    } else if v < 0.7 {
        "Smooth".into()
    } else {
        "Floaty".into()
    }
}

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

pub fn page(s: &SettingsUi, l: &Look, _: &mut Vec<El>) -> El {
    let snap = &s.snap;
    let g = &snap.settings.general;
    let v = &snap.settings.viewer;
    let general =
        |s: &SettingsUi, patch: serde_json::Value| s.cx.patch(json!({ "general": patch }));
    let viewer = |s: &SettingsUi, patch: serde_json::Value| s.cx.patch(json!({ "viewer": patch }));

    let mut app = Vec::new();
    if snap.platform == "macos" {
        let on = g.show_in_dock;
        app.push(widgets::setting(
            l,
            "dock",
            "Keep in Dock when closed",
            Some("While this window is open, Galeria always appears in the Dock and ⌘Tab.".into()),
            Some(widgets::toggle(l, on, move |s| {
                general(s, json!({ "showInDock": !on }))
            })),
        ));
    }
    let (login, restore) = (g.launch_at_login, g.restore_wallpaper_on_quit);
    app.push(widgets::setting(
        l,
        "login",
        "Launch at login",
        None,
        Some(widgets::toggle(l, login, move |s| {
            general(s, json!({ "launchAtLogin": !login }))
        })),
    ));
    app.push(widgets::setting(
        l,
        "restore",
        "Restore my original wallpaper when quitting",
        None,
        Some(widgets::toggle(l, restore, move |s| {
            general(s, json!({ "restoreWallpaperOnQuit": !restore }))
        })),
    ));

    let (hide, minimap, plate) = (v.hide_cursor, v.minimap, v.show_plate);
    let view = vec![
        widgets::setting(
            l,
            "open",
            "Open the viewer",
            None,
            Some(widgets::button(l, "Open Viewer", Kind::Normal, |s, _| {
                s.cx.run(|app| {
                    if let Some(shell) = app.shell() {
                        shell.open_viewer(None, None);
                    }
                })
            })),
        ),
        widgets::setting(
            l,
            "smoothing",
            "Motion smoothing",
            None,
            Some(widgets::slider(
                l,
                s,
                "viewer-smoothing",
                v.smoothing,
                (0.0, 0.95, 0.05),
                feel,
                move |s, x| viewer(s, json!({ "smoothing": x })),
            )),
        ),
        widgets::setting(
            l,
            "dead-zone",
            "Edge dead zone",
            Some(
                "How close to the screen edge the mouse must go to reach the painting's edge."
                    .into(),
            ),
            Some(widgets::slider(
                l,
                s,
                "viewer-dead-zone",
                v.edge_dead_zone,
                (0.0, 0.3, 0.01),
                percent,
                move |s, x| viewer(s, json!({ "edgeDeadZone": x })),
            )),
        ),
        widgets::setting(
            l,
            "cursor",
            "Hide the cursor over the painting",
            None,
            Some(widgets::toggle(l, hide, move |s| {
                viewer(s, json!({ "hideCursor": !hide }))
            })),
        ),
        widgets::setting(
            l,
            "minimap",
            "Show a minimap while exploring",
            None,
            Some(widgets::toggle(l, minimap, move |s| {
                viewer(s, json!({ "minimap": !minimap }))
            })),
        ),
        widgets::setting(
            l,
            "plate",
            "Show the info plate",
            None,
            Some(widgets::toggle(l, plate, move |s| {
                viewer(s, json!({ "showPlate": !plate }))
            })),
        ),
    ];

    let about = vec![
        widgets::block(
            row()
                .w_full()
                .gap(14.0)
                .center_y()
                .child(el().size(56.0, 56.0).pixels(app_icon()).a11y_hidden())
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            row()
                                .gap(6.0)
                                .items_end()
                                .child(text("Galeria").font(TextStyle::new(15.0, 650.0)).color(l.text))
                                .child(text(snap.version).font(widgets::body()).color(l.muted)),
                        )
                        .child(widgets::hint(
                            l,
                            "galeria: Medieval Latin for a long portico or gallery. Paintings stay in your own folder; Galeria only reads them.",
                        )),
                ),
        ),
        widgets::setting(
            l,
            "quit",
            "Quit Galeria",
            None,
            Some(widgets::button(l, "Quit", Kind::Danger, |s, _| s.cx.run(core::quit))),
        ),
    ];

    scroll_page(
        "general",
        vec![
            widgets::section(l, "app", "App", None, vec![], app),
            widgets::section(
                l,
                "viewer",
                "Viewer",
                Some("Full-screen, distraction-free viewing. Move the mouse to reveal the painting edge to edge.".into()),
                vec![],
                view,
            ),
            widgets::section(l, "about", "About", None, vec![], about),
        ],
    )
}
