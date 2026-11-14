//! The menu bar item's menu: the main way to control Galeria.
//!
//! [`build`] describes the menu as plain data, so it can be tested and handed to
//! whatever draws menus on each platform (`shell::tray`); [`on_menu`] carries out
//! a chosen item. `galeria --menu <id>` goes through the same handler.

use crate::core::{self, Snapshot};
use crate::host::{self, AppHandle};
use crate::open;
use crate::settings::{FitMode, FrameStyle, Matte, Order, ScopeKind};

/// One entry of the menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Item {
        id: String,
        text: String,
        enabled: bool,
        /// A key equivalent to show beside the item, e.g. `CmdOrCtrl+,`.
        accelerator: Option<&'static str>,
    },
    Check {
        id: String,
        text: String,
        checked: bool,
    },
    Submenu {
        text: String,
        items: Vec<Node>,
    },
    Separator,
}

fn item(id: &str, text: &str, enabled: bool) -> Node {
    Node::Item {
        id: id.into(),
        text: text.into(),
        enabled,
        accelerator: None,
    }
}

fn check(id: &str, text: &str, checked: bool) -> Node {
    Node::Check {
        id: id.into(),
        text: text.into(),
        checked,
    }
}

fn submenu(text: impl Into<String>, items: Vec<Node>) -> Node {
    Node::Submenu {
        text: text.into(),
        items,
    }
}

/// A line of text that does nothing (titles, status).
fn label(text: &str) -> Node {
    item("noop", text, false)
}

fn enum_id<T: serde::Serialize>(v: T, fallback: &str) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| fallback.to_string())
}

const INTERVALS: &[(u32, &str)] = &[
    (0, "Never"),
    (5, "Every 5 Minutes"),
    (15, "Every 15 Minutes"),
    (30, "Every 30 Minutes"),
    (60, "Every Hour"),
    (180, "Every 3 Hours"),
    (720, "Every 12 Hours"),
    (1440, "Every Day"),
];
const SPEEDS: &[(f64, &str)] = &[
    (2.0, "Glacial"),
    (5.0, "Very Slow"),
    (10.0, "Slow"),
    (20.0, "Gentle"),
    (40.0, "Brisk"),
];
const WALLS: &[(&str, &str)] = &[
    ("#1d1c1a", "Charcoal"),
    ("#46443f", "Gallery Grey"),
    ("#5a1e22", "Museum Red"),
    ("#1e2b3a", "Prussian Blue"),
    ("#36402f", "Sage"),
    ("#e6e3dc", "Gallery White"),
    ("#000000", "Black"),
];
const POSITIONS: &[((f64, f64), &str)] = &[
    ((0.0, 0.0), "Top Left"),
    ((0.5, 0.0), "Top Center"),
    ((1.0, 0.0), "Top Right"),
    ((0.0, 1.0), "Bottom Left"),
    ((0.5, 1.0), "Bottom Center"),
    ((1.0, 1.0), "Bottom Right"),
];
const SIZES: &[(f64, &str)] = &[
    (0.8, "Small"),
    (1.0, "Medium"),
    (1.2, "Large"),
    (1.45, "Extra Large"),
];

/// Only the parts of the snapshot that the menu shows; used to avoid
/// rebuilding (and closing) the menu when nothing visible changed.
pub fn signature(s: &Snapshot) -> String {
    let displays: Vec<_> = s
        .displays
        .iter()
        .map(|d| {
            (
                &d.info.key,
                &d.info.name,
                d.history_len > 0,
                d.painting.as_ref().map(|p| {
                    (
                        &p.id,
                        &p.title,
                        &p.artist,
                        &p.date,
                        p.excluded,
                        p.favorite,
                        &p.source_page,
                    )
                }),
            )
        })
        .collect();
    let mut settings = serde_json::to_value(&s.settings).unwrap_or_default();
    if let Some(p) = settings.get_mut("plate") {
        p["positions"] = serde_json::Value::Null;
    }
    serde_json::json!({
        "d": displays,
        "s": settings,
        "a": s.arrange,
        "l": (&s.library.root, &s.library.error, s.library.valid, s.library.missing, s.library.invalid, s.library.unindexed),
        "c": s.custom_styles.iter().map(|c| (&c.id, &c.name)).collect::<Vec<_>>(),
        "o": s.original_wallpapers,
        "live": s.live_supported,
        "g": s.genres.iter().map(|g| (&g.id, g.count)).collect::<Vec<_>>(),
        "m": s.museums.iter().map(|m| (&m.name, m.count)).collect::<Vec<_>>(),
        "r": (s.rotation.count, s.rotation.empty),
        "dl": s.downloads.active,
    })
    .to_string()
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}…",
            s.chars().take(max - 1).collect::<String>().trim_end()
        )
    }
}

/// What to show beside the menu bar icon when the pointer rests on it.
pub fn tooltip(snap: &Snapshot) -> String {
    snap.displays
        .first()
        .and_then(|d| d.painting.as_ref())
        .map(|p| match &p.artist {
            Some(a) => format!("{} — {}", p.title, a),
            None => p.title.clone(),
        })
        .unwrap_or_else(|| "Galeria".into())
}

pub fn build(s: &Snapshot) -> Vec<Node> {
    let st = &s.settings;
    let w = &st.wallpaper;
    let p = &st.plate;
    let live = w.live && s.live_supported;
    let has_lib = s.library.valid > 0;
    let multi = s.displays.len() > 1 && !w.same_on_all_displays;
    let mut m: Vec<Node> = Vec::new();

    // Now showing
    let mut current_id: Option<String> = None;
    for d in &s.displays {
        if multi {
            m.push(label(&d.info.name.to_uppercase()));
        }
        match &d.painting {
            Some(pv) => {
                current_id.get_or_insert(pv.id.clone());
                m.push(label(&ellipsize(&pv.title, 52)));
                let by = [pv.artist.clone(), pv.date.clone()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(", ");
                if !by.is_empty() {
                    m.push(label(&ellipsize(&by, 52)));
                }
                if multi {
                    m.push(submenu(
                        "This Display",
                        vec![
                            item(&format!("next:{}", d.info.key), "Next Painting", true),
                            item(
                                &format!("prev:{}", d.info.key),
                                "Previous Painting",
                                d.history_len > 0,
                            ),
                            item(&format!("viewer:{}", d.info.key), "Open in Viewer", true),
                        ],
                    ));
                }
            }
            None => {
                let msg = if s.library.root.is_none() {
                    "No art folder chosen"
                } else if let Some(e) = &s.library.error {
                    e.as_str()
                } else {
                    "No paintings available"
                };
                m.push(label(&ellipsize(msg, 52)));
            }
        }
        if multi {
            m.push(Node::Separator);
        }
    }
    if !multi {
        m.push(Node::Separator);
    }

    let any_history = s.displays.iter().any(|d| d.history_len > 0);
    m.push(item("next", "Next Painting", has_lib));
    m.push(item("prev", "Previous Painting", any_history));
    m.push(item("viewer", "Open in Viewer…", has_lib));

    let current = current_id.clone().unwrap_or_default();
    let cur_view = s.displays.iter().find_map(|d| d.painting.as_ref());
    m.push(submenu(
        "This Painting",
        vec![
            item(
                "source",
                "Open Museum Page",
                cur_view.is_some_and(|p| p.source_page.is_some()),
            ),
            item(
                "wikipedia",
                "Open on Wikipedia",
                cur_view.is_some_and(|p| p.wikipedia_url.is_some()),
            ),
            item("reveal", "Show File", cur_view.is_some()),
            Node::Separator,
            check("favorite", "Favorite", cur_view.is_some_and(|p| p.favorite)),
            item("exclude", "Never Show Again", !current.is_empty()),
        ],
    ));
    m.push(Node::Separator);

    // Rotation
    let mut rot: Vec<Node> = INTERVALS
        .iter()
        .map(|(min, text)| {
            check(
                &format!("interval:{min}"),
                text,
                st.rotation.interval_minutes == *min,
            )
        })
        .collect();
    rot.extend([
        Node::Separator,
        check(
            "order:shuffle",
            "Shuffle",
            st.rotation.order == Order::Shuffle,
        ),
        check(
            "order:sequential",
            "In Manifest Order",
            st.rotation.order == Order::Sequential,
        ),
        Node::Separator,
        check(
            "same-all",
            "Same Painting on All Displays",
            w.same_on_all_displays,
        ),
        check(
            "match-orientation",
            "Match Display Orientation",
            w.match_orientation,
        ),
    ]);

    // Which paintings take part
    let sc = &st.rotation.scope;
    let filtering = sc.kind == ScopeKind::Filter;
    let scope_name = match sc.kind {
        ScopeKind::All => "All Paintings".to_string(),
        ScopeKind::Favorites => "Favorites".to_string(),
        ScopeKind::Playlist => st
            .playlists
            .iter()
            .find(|p| Some(&p.id) == sc.playlist.as_ref())
            .map(|p| ellipsize(&p.name, 28))
            .unwrap_or_default(),
        ScopeKind::Filter => {
            let parts: Vec<&str> = sc
                .genres
                .iter()
                .map(|g| crate::genres::name(g))
                .chain(sc.museums.iter().map(String::as_str))
                .collect();
            ellipsize(&parts.join(", "), 28)
        }
        ScopeKind::Selection => format!("{} Selected", sc.selection.len()),
    };
    let mut show = vec![
        check(
            "scope:all",
            &format!("All Paintings ({})", s.library.valid),
            sc.kind == ScopeKind::All,
        ),
        check(
            "scope:favorites",
            &format!("Favorites ({})", st.favorites.len()),
            sc.kind == ScopeKind::Favorites,
        ),
    ];
    for pl in &st.playlists {
        let on = sc.kind == ScopeKind::Playlist && sc.playlist.as_ref() == Some(&pl.id);
        show.push(check(
            &format!("scope:playlist:{}", pl.id),
            &format!("{} ({})", ellipsize(&pl.name, 36), pl.items.len()),
            on,
        ));
    }
    if sc.kind == ScopeKind::Selection {
        show.push(check(
            "noop",
            &format!("Selection ({})", sc.selection.len()),
            true,
        ));
    }
    let by_genre = s
        .genres
        .iter()
        .map(|g| {
            check(
                &format!("scope-genre:{}", g.id),
                &format!("{} ({})", g.name, g.count),
                filtering && sc.genres.contains(&g.id),
            )
        })
        .collect();
    let by_museum = s
        .museums
        .iter()
        .filter(|mu| mu.enabled)
        .take(40)
        .map(|mu| {
            check(
                &format!("scope-museum:{}", mu.name),
                &format!("{} ({})", ellipsize(&mu.name, 44), mu.count),
                filtering && sc.museums.contains(&mu.name),
            )
        })
        .collect();
    show.extend([
        Node::Separator,
        submenu("By Genre", by_genre),
        submenu("By Museum", by_museum),
    ]);
    if s.rotation.empty {
        show.extend([
            Node::Separator,
            label("Nothing matches, so everything is shown"),
        ]);
    }
    show.extend([
        Node::Separator,
        check(
            "match-appearance",
            "Light Paintings by Day, Dark by Night",
            st.appearance.match_system,
        ),
        item("settings:rotation", "Playlists & Light/Dark…", true),
    ]);
    m.push(submenu(format!("Show: {scope_name}"), show));
    m.push(check(
        "pause-rotation",
        "Pause Rotation",
        st.rotation.paused,
    ));
    m.push(submenu("Change Painting", rot));
    m.push(Node::Separator);

    // Fit & motion
    let fit = [
        (FitMode::Auto, "Automatic"),
        (FitMode::Fit, "Show Entire Painting"),
        (FitMode::Fill, "Fill Screen"),
        (FitMode::Pan, "Fill & Pan"),
    ]
    .into_iter()
    .map(|(mode, text)| {
        check(
            &format!("fit:{}", enum_id(mode, "auto")),
            text,
            w.fit == mode,
        )
    })
    .collect();
    let speed = SPEEDS
        .iter()
        .map(|(v, text)| check(&format!("speed:{v}"), text, (w.pan_speed - v).abs() < 0.01))
        .collect();
    let motion = vec![
        check("pan-toggle", "Pan Wide & Tall Paintings", w.pan),
        check("pan-pause", "Pause Panning", w.pan_paused),
        submenu("Panning Speed", speed),
    ];
    let mut wall = vec![
        check(
            "matte:average",
            "Tinted to the Painting",
            w.matte == Matte::Average,
        ),
        check("matte:blur", "Blurred Painting", w.matte == Matte::Blur),
        Node::Separator,
    ];
    for (hex, text) in WALLS {
        let on = w.matte == Matte::Color && w.matte_color.eq_ignore_ascii_case(hex);
        wall.push(check(&format!("wall:{hex}"), text, on));
    }
    wall.push(Node::Separator);
    for (frame, text) in [
        (FrameStyle::None, "No Frame"),
        (FrameStyle::Black, "Black Frame"),
        (FrameStyle::White, "White Frame"),
        (FrameStyle::Gilded, "Gilded Frame"),
        (FrameStyle::Walnut, "Walnut Frame"),
    ] {
        wall.push(check(
            &format!("frame:{}", enum_id(frame, "none")),
            text,
            w.frame == frame,
        ));
    }
    m.push(submenu("Fit", fit));
    m.push(submenu("Panning", motion));
    m.push(submenu("Wall", wall));
    m.push(Node::Separator);

    // Info plate
    let mut style: Vec<Node> = s
        .builtin_styles
        .iter()
        .map(|b| check(&format!("style:{}", b.id), b.name, p.style == b.id))
        .collect();
    if !s.custom_styles.is_empty() {
        style.push(Node::Separator);
        for c in &s.custom_styles {
            style.push(check(&format!("style:{}", c.id), &c.name, p.style == c.id));
        }
    }
    style.extend([
        Node::Separator,
        item("styles-new", "New Custom Style…", true),
        item("styles-folder", "Open Styles Folder", true),
    ]);
    let mut pos: Vec<Node> = POSITIONS
        .iter()
        .map(|((x, y), text)| {
            let on = p.positions.is_empty()
                && (p.default_position.x - x).abs() < 0.01
                && (p.default_position.y - y).abs() < 0.01;
            check(&format!("pos:{x},{y}"), text, on)
        })
        .collect();
    pos.extend([Node::Separator, item("arrange", "Drag on Desktop…", live)]);
    let size = SIZES
        .iter()
        .map(|(v, text)| check(&format!("size:{v}"), text, (p.scale - v).abs() < 0.01))
        .collect();
    let f = &p.fields;
    let fields = vec![
        check(
            "field:artistBio",
            "Artist Nationality & Dates",
            f.artist_bio,
        ),
        check("field:medium", "Medium", f.medium),
        check("field:dimensions", "Dimensions", f.dimensions),
        check("field:description", "Description", f.description),
        check("field:museum", "Museum & Credit Line", f.museum),
        check("field:credit", "Image Credit", f.credit),
        check("field:license", "License", f.license),
        check("field:accession", "Accession Number", f.accession),
        check("field:qr", "QR Code to Museum Page", f.qr),
    ];
    m.push(submenu(
        "Info Plate",
        vec![
            check("plate-toggle", "Show Info Plate", p.visible),
            submenu("Style", style),
            submenu("Position", pos),
            submenu("Size", size),
            submenu("Details", fields),
        ],
    ));
    if s.arrange {
        m.push(item("arrange-done", "Done Arranging", true));
    }
    m.push(Node::Separator);

    // Library
    let lib_label = match (&s.library.root, &s.library.error) {
        (None, _) => "No folder chosen".to_string(),
        (Some(_), Some(e)) => ellipsize(e, 48),
        (Some(_), None) => {
            let mut t = format!("{} paintings", s.library.valid);
            if s.library.missing > 0 {
                t.push_str(&format!(" · {} missing", s.library.missing));
            }
            if s.library.invalid > 0 {
                t.push_str(&format!(" · {} invalid", s.library.invalid));
            }
            t
        }
    };
    let problems = s.library.missing + s.library.invalid + s.library.duplicates;
    let mut lib = vec![label(&lib_label)];
    if problems > 0 {
        lib.push(item(
            "settings:library",
            &format!("Review {problems} Issue(s)…"),
            true,
        ));
    }
    if s.downloads.active > 0 {
        lib.push(item(
            "settings:discover",
            &format!("Downloading {}…", s.downloads.active),
            true,
        ));
    }
    lib.extend([
        Node::Separator,
        item(
            "settings:discover",
            "Find Paintings to Download…",
            s.library.root.is_some(),
        ),
        item("settings:sources", "Museums…", has_lib),
        item("settings:browse", "Browse Paintings…", has_lib),
        item("library-choose", "Choose Art Folder…", true),
        item("library-rescan", "Rescan", s.library.root.is_some()),
        item("library-open", "Open Art Folder", s.library.root.is_some()),
    ]);
    m.push(submenu("Library", lib));

    let mut general = vec![
        check("live", "Live Desktop (Panning & Plate)", live),
        check("sync-system", "Also Set System Wallpaper", w.sync_system),
        item(
            "restore-wallpaper",
            "Restore Original Wallpaper",
            s.original_wallpapers > 0,
        ),
        Node::Separator,
    ];
    if s.platform == "macos" {
        general.insert(
            2,
            check("screen-saver", "Also Set Screen Saver", w.screen_saver),
        );
        general.push(check(
            "dock",
            "Keep in Dock When Closed",
            st.general.show_in_dock,
        ));
    }
    general.extend([
        check("login", "Launch at Login", st.general.launch_at_login),
        check(
            "enrich",
            "Look Up Descriptions Online",
            st.general.enrich_online,
        ),
    ]);
    m.push(submenu("Options", general));
    m.push(Node::Item {
        id: "settings".into(),
        text: "Settings…".into(),
        enabled: true,
        accelerator: Some("CmdOrCtrl+,"),
    });
    m.push(Node::Separator);
    m.push(item("quit", "Quit Galeria", true));
    m
}

fn patch(app: &AppHandle, v: serde_json::Value) {
    if let Err(e) = core::patch_settings(app, &v) {
        log::warn!("menu: {e}");
    }
}

fn current_painting(app: &AppHandle) -> Option<String> {
    let core = app.core();
    let i = core.lock();
    i.displays
        .first()
        .and_then(|d| i.persisted.assignments.get(&d.key))
        .map(|a| a.painting.clone())
        .filter(|id| i.library.get(id).is_some())
}

/// Carry out a menu item. Does file and network work, so call it off the UI thread.
pub fn on_menu(app: &AppHandle, id: &str) {
    let core = app.core();
    let settings = core.lock().settings.clone();
    let (head, arg) = id
        .split_once(':')
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((id, None));
    match (head, arg) {
        ("noop", _) => {}
        ("next", None) => core::advance(app, None, true),
        ("prev", None) => core::advance(app, None, false),
        ("next", Some(k)) => core::advance(app, Some(k.to_string()), true),
        ("prev", Some(k)) => core::advance(app, Some(k.to_string()), false),
        ("viewer", k) => {
            if let Some(shell) = app.shell() {
                shell.open_viewer(None, k.map(str::to_string));
            }
        }
        ("source", _) | ("wikipedia", _) => {
            if let Some(v) = current_painting(app).and_then(|id| core.painting_view(&id)) {
                let url = if head == "source" {
                    v.source_page
                } else {
                    v.wikipedia_url
                };
                if let Some(u) = url {
                    open::url(&u);
                }
            }
        }
        ("reveal", _) => {
            if let Some(src) = current_painting(app).and_then(|id| core.source_for(&id)) {
                open::reveal(&src.path);
            }
        }
        ("favorite", _) => {
            if let Some(id) = current_painting(app) {
                let fav = settings.favorites.contains(&id);
                core::set_favorite(app, &id, !fav);
            }
        }
        ("exclude", _) => {
            let ids: Vec<String> = {
                let i = core.lock();
                i.persisted
                    .assignments
                    .values()
                    .map(|a| a.painting.clone())
                    .collect()
            };
            if let Some(id) = current_painting(app).or_else(|| ids.first().cloned()) {
                core::set_excluded(app, &id, true);
            }
        }
        ("scope", Some(v)) => match v.split_once(':') {
            Some(("playlist", id)) => patch(
                app,
                serde_json::json!({ "rotation": { "scope": { "kind": "playlist", "playlist": id } } }),
            ),
            _ => patch(
                app,
                serde_json::json!({ "rotation": { "scope": { "kind": v } } }),
            ),
        },
        ("scope-genre", Some(v)) | ("scope-museum", Some(v)) => {
            // Ticking a genre or museum switches to (or edits) a filter; unticking the last one shows everything.
            let sc = &settings.rotation.scope;
            let (mut genres, mut museums) = if sc.kind == ScopeKind::Filter {
                (sc.genres.clone(), sc.museums.clone())
            } else {
                (vec![], vec![])
            };
            let list = if head == "scope-genre" {
                &mut genres
            } else {
                &mut museums
            };
            match list.iter().position(|x| x == v) {
                Some(i) => {
                    list.remove(i);
                }
                None => list.push(v.to_string()),
            }
            let kind = if genres.is_empty() && museums.is_empty() {
                "all"
            } else {
                "filter"
            };
            patch(
                app,
                serde_json::json!({ "rotation": { "scope": { "kind": kind, "genres": genres, "museums": museums } } }),
            );
        }
        ("match-appearance", _) => patch(
            app,
            serde_json::json!({ "appearance": { "matchSystem": !settings.appearance.match_system } }),
        ),
        ("pause-rotation", _) => patch(
            app,
            serde_json::json!({ "rotation": { "paused": !settings.rotation.paused } }),
        ),
        ("interval", Some(v)) => patch(
            app,
            serde_json::json!({ "rotation": { "intervalMinutes": v.parse::<u32>().unwrap_or(30) } }),
        ),
        ("order", Some(v)) => patch(app, serde_json::json!({ "rotation": { "order": v } })),
        ("same-all", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "sameOnAllDisplays": !settings.wallpaper.same_on_all_displays } }),
        ),
        ("match-orientation", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "matchOrientation": !settings.wallpaper.match_orientation } }),
        ),
        ("fit", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "fit": v } })),
        ("pan-toggle", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "pan": !settings.wallpaper.pan } }),
        ),
        ("pan-pause", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "panPaused": !settings.wallpaper.pan_paused } }),
        ),
        ("speed", Some(v)) => patch(
            app,
            serde_json::json!({ "wallpaper": { "panSpeed": v.parse::<f64>().unwrap_or(5.0), "panPaused": false } }),
        ),
        ("matte", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "matte": v } })),
        ("wall", Some(hex)) => patch(
            app,
            serde_json::json!({ "wallpaper": { "matte": "color", "matteColor": hex } }),
        ),
        ("frame", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "frame": v } })),
        ("plate-toggle", _) => patch(
            app,
            serde_json::json!({ "plate": { "visible": !settings.plate.visible } }),
        ),
        ("style", Some(v)) => patch(
            app,
            serde_json::json!({ "plate": { "style": v, "visible": true } }),
        ),
        ("pos", Some(v)) => {
            if let Some((x, y)) = v.split_once(',') {
                core::set_plate_preset(app, x.parse().unwrap_or(1.0), y.parse().unwrap_or(1.0));
            }
        }
        ("size", Some(v)) => patch(
            app,
            serde_json::json!({ "plate": { "scale": v.parse::<f64>().unwrap_or(1.0) } }),
        ),
        ("field", Some(f)) => {
            let cur = serde_json::to_value(&settings.plate.fields)
                .ok()
                .and_then(|v| v.get(f).and_then(|b| b.as_bool()));
            patch(
                app,
                serde_json::json!({ "plate": { "fields": { f: !cur.unwrap_or(false) } } }),
            );
        }
        ("arrange", _) => core::set_arrange(app, true),
        ("arrange-done", _) => core::set_arrange(app, false),
        ("styles-folder", _) => {
            let _ = std::fs::create_dir_all(&core.paths.styles);
            open::path(&core.paths.styles);
        }
        ("styles-new", _) => {
            if let Some(shell) = app.shell() {
                shell.open_settings(Some("plate".into()));
            }
        }
        ("library-choose", _) => {
            if let Some(shell) = app.shell() {
                shell.choose_library_folder();
            }
        }
        ("library-rescan", _) => {
            let a = app.clone();
            host::spawn_blocking(move || core::reload_library(&a));
        }
        ("library-open", _) => {
            if let Some(root) = &settings.library_path {
                open::path(root);
            }
        }
        ("live", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "live": !settings.wallpaper.live } }),
        ),
        ("screen-saver", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "screenSaver": !settings.wallpaper.screen_saver } }),
        ),
        ("sync-system", _) => patch(
            app,
            serde_json::json!({ "wallpaper": { "syncSystem": !settings.wallpaper.sync_system } }),
        ),
        ("restore-wallpaper", _) => {
            patch(
                app,
                serde_json::json!({ "wallpaper": { "syncSystem": false } }),
            );
            let a = app.clone();
            host::spawn_blocking(move || core::restore_original_wallpaper(&a));
        }
        ("dock", _) => patch(
            app,
            serde_json::json!({ "general": { "showInDock": !settings.general.show_in_dock } }),
        ),
        ("login", _) => patch(
            app,
            serde_json::json!({ "general": { "launchAtLogin": !settings.general.launch_at_login } }),
        ),
        ("enrich", _) => patch(
            app,
            serde_json::json!({ "general": { "enrichOnline": !settings.general.enrich_online } }),
        ),
        ("settings", section) => {
            if let Some(shell) = app.shell() {
                shell.open_settings(section.map(str::to_string));
            }
        }
        ("quit", _) => core::quit(app),
        _ => log::debug!("unhandled menu item {id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            match n {
                Node::Item { id, .. } | Node::Check { id, .. } => out.push(id.clone()),
                Node::Submenu { items, .. } => ids(items, out),
                Node::Separator => {}
            }
        }
    }

    #[test]
    fn menu_offers_every_control() {
        let dir = std::env::temp_dir().join(format!("galeria-menu-{}", fastrand::u64(..)));
        let paths = core::Paths::in_dir(&dir).unwrap();
        let app = AppHandle::new(core::Core::new(paths));
        let snap = app.core().snapshot();
        let menu = build(&snap);
        let mut all = Vec::new();
        ids(&menu, &mut all);
        for id in [
            "next",
            "prev",
            "viewer",
            "favorite",
            "exclude",
            "interval:30",
            "order:shuffle",
            "scope:all",
            "pause-rotation",
            "fit:auto",
            "fit:pan",
            "pan-toggle",
            "speed:5",
            "matte:average",
            "wall:#1d1c1a",
            "frame:gilded",
            "plate-toggle",
            "style:plastic",
            "style:glass",
            "pos:1,1",
            "size:1",
            "field:qr",
            "library-choose",
            "live",
            "sync-system",
            "login",
            "enrich",
            "settings",
            "quit",
        ] {
            assert!(all.iter().any(|x| x == id), "menu is missing {id}");
        }
        // Nothing to show yet, so the painting controls are greyed out.
        let next = menu
            .iter()
            .find(|n| matches!(n, Node::Item { id, .. } if id == "next"));
        assert!(matches!(next, Some(Node::Item { enabled: false, .. })));
        assert_eq!(tooltip(&snap), "Galeria");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn menu_items_change_settings() {
        let dir = std::env::temp_dir().join(format!("galeria-menu-{}", fastrand::u64(..)));
        let paths = core::Paths::in_dir(&dir).unwrap();
        let app = AppHandle::new(core::Core::new(paths));
        on_menu(&app, "style:brass");
        on_menu(&app, "fit:pan");
        on_menu(&app, "interval:60");
        on_menu(&app, "field:qr");
        on_menu(&app, "scope-genre:seascape");
        let s = app.core().lock().settings.clone();
        assert_eq!(s.plate.style, "brass");
        assert_eq!(s.wallpaper.fit, FitMode::Pan);
        assert_eq!(s.rotation.interval_minutes, 60);
        assert!(s.plate.fields.qr);
        assert_eq!(s.rotation.scope.kind, ScopeKind::Filter);
        assert_eq!(s.rotation.scope.genres, vec!["seascape".to_string()]);
        // Unticking the last genre goes back to showing everything.
        on_menu(&app, "scope-genre:seascape");
        assert_eq!(
            app.core().lock().settings.rotation.scope.kind,
            ScopeKind::All
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
