//! Menu bar (tray) item: the main way to control Galeria.

use crate::core::{self, Core, Snapshot};
use crate::settings::{FitMode, FrameStyle, Matte, Order};
use crate::wall;
use tauri::image::Image;
use tauri::menu::{CheckMenuItemBuilder, IsMenuItem, Menu, MenuBuilder, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

pub const TRAY_ID: &str = "galeria";

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
const SPEEDS: &[(f64, &str)] = &[(2.0, "Glacial"), (5.0, "Very Slow"), (10.0, "Slow"), (20.0, "Gentle"), (40.0, "Brisk")];
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
const SIZES: &[(f64, &str)] = &[(0.8, "Small"), (1.0, "Medium"), (1.2, "Large"), (1.45, "Extra Large")];

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
                d.painting.as_ref().map(|p| (&p.id, &p.title, &p.artist, &p.date, p.excluded, p.favorite, &p.source_page)),
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
    })
    .to_string()
}

pub fn create(app: &AppHandle) {
    let snap = app.state::<Core>().snapshot();
    let menu = match build(app, &snap) {
        Ok(m) => m,
        Err(e) => {
            log::error!("tray menu: {e}");
            return;
        }
    };
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png")).expect("tray icon");
    let res = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Galeria")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
        .build(app);
    if let Err(e) = res {
        log::error!("tray icon: {e}");
    }
}

pub fn update(app: &AppHandle, snap: &Snapshot) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    match build(app, snap) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::error!("tray menu: {e}"),
    }
    let tip = snap
        .displays
        .first()
        .and_then(|d| d.painting.as_ref())
        .map(|p| match &p.artist {
            Some(a) => format!("{} — {}", p.title, a),
            None => p.title.clone(),
        })
        .unwrap_or_else(|| "Galeria".into());
    let _ = tray.set_tooltip(Some(tip));
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>().trim_end())
    }
}

fn build(app: &AppHandle, s: &Snapshot) -> tauri::Result<Menu<Wry>> {
    let st = &s.settings;
    let w = &st.wallpaper;
    let p = &st.plate;
    let live = w.live && s.live_supported;
    let has_lib = s.library.valid > 0;
    let multi = s.displays.len() > 1 && !w.same_on_all_displays;
    let item = |id: &str, text: &str, enabled: bool| MenuItemBuilder::with_id(id, text).enabled(enabled).build(app);
    let check = |id: &str, text: &str, on: bool| CheckMenuItemBuilder::with_id(id, text).checked(on).build(app);

    let mut m = MenuBuilder::new(app);

    // Now showing
    let mut current_id: Option<String> = None;
    for d in &s.displays {
        if multi {
            m = m.item(&item("noop", &d.info.name.to_uppercase(), false)?);
        }
        match &d.painting {
            Some(pv) => {
                current_id.get_or_insert(pv.id.clone());
                m = m.item(&item("noop", &ellipsize(&pv.title, 52), false)?);
                let by = [pv.artist.clone(), pv.date.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ");
                if !by.is_empty() {
                    m = m.item(&item("noop", &ellipsize(&by, 52), false)?);
                }
                if multi {
                    let sub = SubmenuBuilder::new(app, "This Display")
                        .item(&item(&format!("next:{}", d.info.key), "Next Painting", true)?)
                        .item(&item(&format!("prev:{}", d.info.key), "Previous Painting", d.history_len > 0)?)
                        .item(&item(&format!("viewer:{}", d.info.key), "Open in Viewer", true)?)
                        .build()?;
                    m = m.item(&sub);
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
                m = m.item(&item("noop", &ellipsize(msg, 52), false)?);
            }
        }
        if multi {
            m = m.separator();
        }
    }
    if !multi {
        m = m.separator();
    }

    let any_history = s.displays.iter().any(|d| d.history_len > 0);
    m = m
        .item(&item("next", "Next Painting", has_lib)?)
        .item(&item("prev", "Previous Painting", any_history)?)
        .item(&item("viewer", "Open in Viewer…", has_lib)?);

    let current = current_id.clone().unwrap_or_default();
    let cur_view = s.displays.iter().find_map(|d| d.painting.as_ref());
    let about = SubmenuBuilder::new(app, "This Painting")
        .item(&item("source", "Open Museum Page", cur_view.is_some_and(|p| p.source_page.is_some()))?)
        .item(&item("wikipedia", "Open on Wikipedia", cur_view.is_some_and(|p| p.wikipedia_url.is_some()))?)
        .item(&item("reveal", "Show File", cur_view.is_some())?)
        .separator()
        .item(&check("favorite", "Favorite", cur_view.is_some_and(|p| p.favorite))?)
        .item(&item("exclude", "Never Show Again", !current.is_empty())?)
        .build()?;
    m = m.item(&about).separator();

    // Rotation
    let mut rot = SubmenuBuilder::new(app, "Change Painting");
    for (min, label) in INTERVALS {
        rot = rot.item(&check(&format!("interval:{min}"), label, st.rotation.interval_minutes == *min)?);
    }
    rot = rot
        .separator()
        .item(&check("order:shuffle", "Shuffle", st.rotation.order == Order::Shuffle)?)
        .item(&check("order:sequential", "In Manifest Order", st.rotation.order == Order::Sequential)?)
        .separator()
        .item(&check("same-all", "Same Painting on All Displays", w.same_on_all_displays)?)
        .item(&check("match-orientation", "Match Display Orientation", w.match_orientation)?);
    m = m
        .item(&check("pause-rotation", "Pause Rotation", st.rotation.paused)?)
        .item(&rot.build()?)
        .separator();

    // Fit & motion
    let mut fit = SubmenuBuilder::new(app, "Fit");
    for (mode, label) in [
        (FitMode::Auto, "Automatic"),
        (FitMode::Fit, "Show Entire Painting"),
        (FitMode::Fill, "Fill Screen"),
        (FitMode::Pan, "Fill & Pan"),
    ] {
        let id = format!("fit:{}", serde_json::to_value(mode).unwrap().as_str().unwrap_or("auto"));
        fit = fit.item(&check(&id, label, w.fit == mode)?);
    }
    let mut speed = SubmenuBuilder::new(app, "Panning Speed");
    for (v, label) in SPEEDS {
        speed = speed.item(&check(&format!("speed:{v}"), label, (w.pan_speed - v).abs() < 0.01)?);
    }
    let motion = SubmenuBuilder::new(app, "Panning")
        .item(&check("pan-toggle", "Pan Wide & Tall Paintings", w.pan)?)
        .item(&check("pan-pause", "Pause Panning", w.pan_paused)?)
        .item(&speed.build()?)
        .build()?;
    let mut wall_menu = SubmenuBuilder::new(app, "Wall")
        .item(&check("matte:average", "Tinted to the Painting", w.matte == Matte::Average)?)
        .item(&check("matte:blur", "Blurred Painting", w.matte == Matte::Blur)?)
        .separator();
    for (hex, label) in WALLS {
        let on = w.matte == Matte::Color && w.matte_color.eq_ignore_ascii_case(hex);
        wall_menu = wall_menu.item(&check(&format!("wall:{hex}"), label, on)?);
    }
    wall_menu = wall_menu.separator();
    for (frame, label) in [
        (FrameStyle::None, "No Frame"),
        (FrameStyle::Black, "Black Frame"),
        (FrameStyle::White, "White Frame"),
        (FrameStyle::Gilded, "Gilded Frame"),
        (FrameStyle::Walnut, "Walnut Frame"),
    ] {
        let id = format!("frame:{}", serde_json::to_value(frame).unwrap().as_str().unwrap_or("none"));
        wall_menu = wall_menu.item(&check(&id, label, w.frame == frame)?);
    }
    m = m.item(&fit.build()?).item(&motion).item(&wall_menu.build()?).separator();

    // Info plate
    let mut style = SubmenuBuilder::new(app, "Style");
    for b in &s.builtin_styles {
        style = style.item(&check(&format!("style:{}", b.id), b.name, p.style == b.id)?);
    }
    if !s.custom_styles.is_empty() {
        style = style.separator();
        for c in &s.custom_styles {
            style = style.item(&check(&format!("style:{}", c.id), &c.name, p.style == c.id)?);
        }
    }
    style = style.separator().item(&item("styles-new", "New Custom Style…", true)?).item(&item(
        "styles-folder",
        "Open Styles Folder",
        true,
    )?);
    let mut pos = SubmenuBuilder::new(app, "Position");
    for ((x, y), label) in POSITIONS {
        let on = p.positions.is_empty() && (p.default_position.x - x).abs() < 0.01 && (p.default_position.y - y).abs() < 0.01;
        pos = pos.item(&check(&format!("pos:{x},{y}"), label, on)?);
    }
    pos = pos.separator().item(&item("arrange", "Drag on Desktop…", live)?);
    let mut size = SubmenuBuilder::new(app, "Size");
    for (v, label) in SIZES {
        size = size.item(&check(&format!("size:{v}"), label, (p.scale - v).abs() < 0.01)?);
    }
    let f = &p.fields;
    let fields = SubmenuBuilder::new(app, "Details")
        .item(&check("field:artistBio", "Artist Nationality & Dates", f.artist_bio)?)
        .item(&check("field:medium", "Medium", f.medium)?)
        .item(&check("field:dimensions", "Dimensions", f.dimensions)?)
        .item(&check("field:description", "Description", f.description)?)
        .item(&check("field:museum", "Museum & Credit Line", f.museum)?)
        .item(&check("field:credit", "Image Credit", f.credit)?)
        .item(&check("field:license", "License", f.license)?)
        .item(&check("field:accession", "Accession Number", f.accession)?)
        .item(&check("field:qr", "QR Code to Museum Page", f.qr)?)
        .build()?;
    let plate: Submenu<Wry> = SubmenuBuilder::new(app, "Info Plate")
        .item(&check("plate-toggle", "Show Info Plate", p.visible)?)
        .item(&style.build()?)
        .item(&pos.build()?)
        .item(&size.build()?)
        .item(&fields)
        .build()?;
    m = m.item(&plate);
    if s.arrange {
        m = m.item(&item("arrange-done", "Done Arranging", true)?);
    }
    m = m.separator();

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
    let mut lib = SubmenuBuilder::new(app, "Library").item(&item("noop", &lib_label, false)?);
    if problems > 0 {
        lib = lib.item(&item("settings:library", &format!("Review {problems} Issue(s)…"), true)?);
    }
    lib = lib
        .separator()
        .item(&item("settings:browse", "Browse Paintings…", has_lib)?)
        .item(&item("library-choose", "Choose Art Folder…", true)?)
        .item(&item("library-rescan", "Rescan", s.library.root.is_some())?)
        .item(&item("library-open", "Open Art Folder", s.library.root.is_some())?);
    m = m.item(&lib.build()?);

    let general = SubmenuBuilder::new(app, "Options")
        .item(&check("live", "Live Desktop (Panning & Plate)", live)?)
        .item(&check("sync-system", "Also Set System Wallpaper", w.sync_system)?)
        .item(&item("restore-wallpaper", "Restore Original Wallpaper", s.original_wallpapers > 0)?)
        .separator()
        .item(&check("dock", "Show in Dock", st.general.show_in_dock)?)
        .item(&check("login", "Launch at Login", st.general.launch_at_login)?)
        .item(&check("enrich", "Look Up Descriptions Online", st.general.enrich_online)?)
        .build()?;
    let quit = item("quit", "Quit Galeria", true)?;
    let settings = MenuItemBuilder::with_id("settings", "Settings…").accelerator("CmdOrCtrl+,").build(app)?;
    let tail: [&dyn IsMenuItem<Wry>; 2] = [&general, &settings];
    m = m.items(&tail).separator().item(&quit);
    m.build()
}

fn patch(app: &AppHandle, v: serde_json::Value) {
    if let Err(e) = core::patch_settings(app, &v) {
        log::warn!("menu: {e}");
    }
}

fn current_painting(app: &AppHandle) -> Option<String> {
    let core = app.state::<Core>();
    let i = core.lock();
    i.displays
        .first()
        .and_then(|d| i.persisted.assignments.get(&d.key))
        .map(|a| a.painting.clone())
        .filter(|id| i.library.get(id).is_some())
}

pub fn on_menu(app: &AppHandle, id: &str) {
    let core = app.state::<Core>();
    let settings = core.lock().settings.clone();
    let (head, arg) = id.split_once(':').map(|(a, b)| (a, Some(b))).unwrap_or((id, None));
    match (head, arg) {
        ("noop", _) => {}
        ("next", None) => core::advance(app, None, true),
        ("prev", None) => core::advance(app, None, false),
        ("next", Some(k)) => core::advance(app, Some(k.to_string()), true),
        ("prev", Some(k)) => core::advance(app, Some(k.to_string()), false),
        ("viewer", k) => wall::spawn_open_viewer(app, None, k.map(str::to_string)),
        ("source", _) | ("wikipedia", _) => {
            if let Some(v) = current_painting(app).and_then(|id| core.painting_view(&id)) {
                let url = if head == "source" { v.source_page } else { v.wikipedia_url };
                if let Some(u) = url {
                    let _ = app.opener().open_url(u, None::<&str>);
                }
            }
        }
        ("reveal", _) => {
            if let Some(src) = current_painting(app).and_then(|id| core.source_for(&id)) {
                let _ = app.opener().reveal_item_in_dir(src.path);
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
                i.persisted.assignments.values().map(|a| a.painting.clone()).collect()
            };
            if let Some(id) = current_painting(app).or_else(|| ids.first().cloned()) {
                core::set_excluded(app, &id, true);
            }
        }
        ("pause-rotation", _) => patch(app, serde_json::json!({ "rotation": { "paused": !settings.rotation.paused } })),
        ("interval", Some(v)) => patch(app, serde_json::json!({ "rotation": { "intervalMinutes": v.parse::<u32>().unwrap_or(30) } })),
        ("order", Some(v)) => patch(app, serde_json::json!({ "rotation": { "order": v } })),
        ("same-all", _) => patch(app, serde_json::json!({ "wallpaper": { "sameOnAllDisplays": !settings.wallpaper.same_on_all_displays } })),
        ("match-orientation", _) => patch(app, serde_json::json!({ "wallpaper": { "matchOrientation": !settings.wallpaper.match_orientation } })),
        ("fit", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "fit": v } })),
        ("pan-toggle", _) => patch(app, serde_json::json!({ "wallpaper": { "pan": !settings.wallpaper.pan } })),
        ("pan-pause", _) => patch(app, serde_json::json!({ "wallpaper": { "panPaused": !settings.wallpaper.pan_paused } })),
        ("speed", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "panSpeed": v.parse::<f64>().unwrap_or(5.0), "panPaused": false } })),
        ("matte", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "matte": v } })),
        ("wall", Some(hex)) => patch(app, serde_json::json!({ "wallpaper": { "matte": "color", "matteColor": hex } })),
        ("frame", Some(v)) => patch(app, serde_json::json!({ "wallpaper": { "frame": v } })),
        ("plate-toggle", _) => patch(app, serde_json::json!({ "plate": { "visible": !settings.plate.visible } })),
        ("style", Some(v)) => patch(app, serde_json::json!({ "plate": { "style": v, "visible": true } })),
        ("pos", Some(v)) => {
            if let Some((x, y)) = v.split_once(',') {
                core::set_plate_preset(app, x.parse().unwrap_or(1.0), y.parse().unwrap_or(1.0));
            }
        }
        ("size", Some(v)) => patch(app, serde_json::json!({ "plate": { "scale": v.parse::<f64>().unwrap_or(1.0) } })),
        ("field", Some(f)) => {
            let cur = serde_json::to_value(&settings.plate.fields).ok().and_then(|v| v.get(f).and_then(|b| b.as_bool()));
            patch(app, serde_json::json!({ "plate": { "fields": { f: !cur.unwrap_or(false) } } }));
        }
        ("arrange", _) => wall::set_arrange(app, true),
        ("arrange-done", _) => wall::set_arrange(app, false),
        ("styles-folder", _) => {
            let _ = app.opener().open_path(core.paths.styles.to_string_lossy(), None::<&str>);
        }
        ("styles-new", _) => wall::spawn_open_settings(app, Some("plate".into())),
        ("library-choose", _) => {
            let a = app.clone();
            app.dialog().file().set_title("Choose the folder that contains manifest.json").pick_folder(move |folder| {
                if let Some(path) = folder.and_then(|f| f.into_path().ok()) {
                    patch(&a, serde_json::json!({ "libraryPath": path }));
                }
            });
        }
        ("library-rescan", _) => {
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || core::reload_library(&a));
        }
        ("library-open", _) => {
            if let Some(root) = &settings.library_path {
                let _ = app.opener().open_path(root.to_string_lossy(), None::<&str>);
            }
        }
        ("live", _) => patch(app, serde_json::json!({ "wallpaper": { "live": !settings.wallpaper.live } })),
        ("sync-system", _) => patch(app, serde_json::json!({ "wallpaper": { "syncSystem": !settings.wallpaper.sync_system } })),
        ("restore-wallpaper", _) => {
            patch(app, serde_json::json!({ "wallpaper": { "syncSystem": false } }));
            let a = app.clone();
            tauri::async_runtime::spawn_blocking(move || core::restore_original_wallpaper(&a));
        }
        ("dock", _) => patch(app, serde_json::json!({ "general": { "showInDock": !settings.general.show_in_dock } })),
        ("login", _) => patch(app, serde_json::json!({ "general": { "launchAtLogin": !settings.general.launch_at_login } })),
        ("enrich", _) => patch(app, serde_json::json!({ "general": { "enrichOnline": !settings.general.enrich_online } })),
        ("settings", section) => wall::spawn_open_settings(app, section.map(str::to_string)),
        ("quit", _) => crate::quit(app),
        _ => log::debug!("unhandled menu item {id}"),
    }
}
