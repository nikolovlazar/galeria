//! Wallpaper: how paintings meet the screen.

use super::widgets::{self, Kind};
use super::{scroll_page, SettingsUi};
use crate::actions;
use crate::settings::{FitMode, FrameStyle, Matte};
use crate::ui::theme::{hex_or, Look};
use rune::{el, row, Color, Cursor, El};
use serde_json::json;

const SPEEDS: [(f64, &str); 5] = [
    (2.0, "Glacial"),
    (5.0, "Very slow"),
    (10.0, "Slow"),
    (20.0, "Gentle"),
    (40.0, "Brisk"),
];
pub const WALLS: [(&str, &str); 7] = [
    ("#1d1c1a", "Charcoal"),
    ("#46443f", "Gallery Grey"),
    ("#5a1e22", "Museum Red"),
    ("#1e2b3a", "Prussian Blue"),
    ("#36402f", "Sage"),
    ("#e6e3dc", "Gallery White"),
    ("#000000", "Black"),
];

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

fn seconds(v: f64) -> String {
    format!("{v} s")
}

fn speed(v: f64) -> String {
    format!("{v} pt/s")
}

fn wallpaper(s: &SettingsUi, patch: serde_json::Value) {
    s.cx.patch(json!({ "wallpaper": patch }));
}

pub fn page(s: &SettingsUi, l: &Look, _: &mut Vec<El>) -> El {
    let snap = &s.snap;
    let w = &snap.settings.wallpaper;
    let live_ok = snap.live_supported;
    let live = w.live && live_ok;

    // Desktop ---------------------------------------------------------------
    let mut desktop = vec![widgets::disabled(
        widgets::setting(
            l,
            "live",
            "Live desktop",
            Some(if live_ok {
                "Draws the painting, its info plate and slow panning behind your desktop icons."
                    .into()
            } else {
                "Not available in this desktop session (Wayland). Galeria sets the system wallpaper instead.".into()
            }),
            Some(widgets::toggle(l, live, move |s| {
                wallpaper(s, json!({ "live": !live }))
            })),
        ),
        !live_ok,
    )];
    let sync = w.sync_system;
    desktop.push(widgets::setting(
        l,
        "sync",
        "Also set the system wallpaper",
        Some("Keeps Mission Control, Spaces and the lock screen in step. Your original wallpaper is remembered.".into()),
        Some(widgets::toggle(l, sync, move |s| wallpaper(s, json!({ "syncSystem": !sync })))),
    ));
    if crate::platform::screen_saver_supported() {
        let saver = w.screen_saver;
        desktop.push(widgets::setting(
            l,
            "saver",
            "Also set the screen saver",
            Some("Plays your art folder as a slideshow in the Photos screen saver. Turning this on changes your screen saver choice.".into()),
            Some(widgets::toggle(l, saver, move |s| wallpaper(s, json!({ "screenSaver": !saver })))),
        ));
    }
    if snap.original_wallpapers > 0 {
        desktop.push(widgets::setting(
            l,
            "restore",
            "Original wallpaper",
            Some("Turns off syncing and puts your previous wallpaper back.".into()),
            Some(widgets::button(l, "Restore", Kind::Normal, |s, _| {
                s.cx.run(actions::restore_original_wallpaper)
            })),
        ));
    }

    // Fit -------------------------------------------------------------------
    let modes = [FitMode::Auto, FitMode::Fit, FitMode::Fill, FitMode::Pan];
    let mut fit = vec![widgets::setting(
        l,
        "mode",
        "Mode",
        None,
        Some(widgets::segmented(
            l,
            &["Automatic", "Entire painting", "Fill screen", "Fill & pan"],
            modes.iter().position(|m| *m == w.fit),
            move |s, i| wallpaper(s, json!({ "fit": modes[i] })),
        )),
    )];
    if w.fit == FitMode::Auto {
        fit.push(widgets::setting(
            l,
            "crop",
            "Fill when the crop is under",
            Some("Paintings close to the screen's shape fill it edge to edge; anything more unusual pans (or hangs on the wall).".into()),
            Some(widgets::slider(l, s, "crop-tolerance", w.crop_tolerance, (0.0, 0.3, 0.01), percent, |s, v| {
                wallpaper(s, json!({ "cropTolerance": v }))
            })),
        ));
    }
    let pan = w.pan;
    fit.push(widgets::setting(
        l,
        "pan",
        "Pan wide and tall paintings",
        Some("Fill the screen and drift edge to edge, so you see the whole work over time.".into()),
        Some(widgets::toggle(l, pan, move |s| {
            wallpaper(s, json!({ "pan": !pan }))
        })),
    ));
    // How long one sweep of a typical panorama takes at this speed, on this display.
    let sweep = snap
        .displays
        .first()
        .map(|d| (d.info.width * 2.9 - d.info.width) / w.pan_speed.max(0.1) / 60.0)
        .unwrap_or(0.0);
    let sweep = if sweep < 1.0 {
        "<1".to_string()
    } else {
        format!("{}", sweep.round())
    };
    fit.push(widgets::setting(
        l,
        "speed",
        "Panning speed",
        Some(format!(
            "A 2.9:1 panorama on this display takes about {sweep} min per sweep."
        )),
        Some(widgets::segmented(
            l,
            &SPEEDS.map(|(_, name)| name),
            SPEEDS
                .iter()
                .position(|(v, _)| (*v - w.pan_speed).abs() < 0.01),
            |s, i| wallpaper(s, json!({ "panSpeed": SPEEDS[i].0, "panPaused": false })),
        )),
    ));
    fit.push(widgets::setting(
        l,
        "custom-speed",
        "Custom speed",
        None,
        Some(widgets::slider(
            l,
            s,
            "pan-speed",
            w.pan_speed,
            (0.5, 60.0, 0.5),
            speed,
            |s, v| wallpaper(s, json!({ "panSpeed": v })),
        )),
    ));
    fit.push(widgets::setting(
        l,
        "rest",
        "Rest at each edge",
        None,
        Some(widgets::slider(
            l,
            s,
            "pan-pause",
            w.pan_pause,
            (0.0, 180.0, 5.0),
            seconds,
            |s, v| wallpaper(s, json!({ "panPause": v })),
        )),
    ));
    let paused = w.pan_paused;
    fit.push(widgets::setting(
        l,
        "pause",
        "Pause panning",
        None,
        Some(widgets::toggle(l, paused, move |s| {
            wallpaper(s, json!({ "panPaused": !paused }))
        })),
    ));

    // Wall ------------------------------------------------------------------
    let mattes = [Matte::Average, Matte::Blur, Matte::Color];
    let mut swatches = row().gap(8.0).center_y();
    for (hex, name) in WALLS {
        let on = w.matte == Matte::Color && w.matte_color.eq_ignore_ascii_case(hex);
        let c = hex_or(hex, Color::BLACK);
        swatches = swatches.child(
            el().key(format!("wall-{hex}"))
                .role("radio")
                .label(name)
                .checked(on)
                .focusable(true)
                .size(24.0, 24.0)
                .radius(12.0)
                .fill(c)
                .border(
                    if on { 2.0 } else { l.hair },
                    if on { l.gold } else { l.text.alpha(0.22) },
                )
                .cursor(Cursor::Pointer)
                .help(name)
                .on_press(move |s: &mut SettingsUi| {
                    wallpaper(s, json!({ "matte": "color", "matteColor": hex }))
                }),
        );
    }
    let frames = [
        FrameStyle::None,
        FrameStyle::Black,
        FrameStyle::White,
        FrameStyle::Gilded,
        FrameStyle::Walnut,
    ];
    let (shadow, spotlight) = (w.shadow, w.spotlight);
    let wall = vec![
        widgets::setting(
            l,
            "matte",
            "Wall",
            None,
            Some(widgets::segmented(
                l,
                &["Tinted to painting", "Blurred painting", "Colour"],
                mattes.iter().position(|m| *m == w.matte),
                move |s, i| wallpaper(s, json!({ "matte": mattes[i] })),
            )),
        ),
        widgets::setting(l, "wall-colour", "Wall colour", None, Some(swatches)),
        widgets::setting(
            l,
            "frame",
            "Frame",
            None,
            Some(widgets::segmented(
                l,
                &["None", "Black", "White", "Gilded", "Walnut"],
                frames.iter().position(|f| *f == w.frame),
                move |s, i| wallpaper(s, json!({ "frame": frames[i] })),
            )),
        ),
        widgets::setting(
            l,
            "margin",
            "Space around the painting",
            None,
            Some(widgets::slider(
                l,
                s,
                "fit-margin",
                w.fit_margin,
                (0.0, 0.2, 0.01),
                percent,
                |s, v| wallpaper(s, json!({ "fitMargin": v })),
            )),
        ),
        widgets::setting(
            l,
            "shadow",
            "Shadow on the wall",
            None,
            Some(widgets::toggle(l, shadow, move |s| {
                wallpaper(s, json!({ "shadow": !shadow }))
            })),
        ),
        widgets::setting(
            l,
            "spotlight",
            "Gallery spotlight",
            None,
            Some(widgets::toggle(l, spotlight, move |s| {
                wallpaper(s, json!({ "spotlight": !spotlight }))
            })),
        ),
        widgets::setting(
            l,
            "crossfade",
            "Crossfade",
            None,
            Some(widgets::slider(
                l,
                s,
                "transition",
                w.transition_seconds,
                (0.0, 6.0, 0.25),
                seconds,
                |s, v| wallpaper(s, json!({ "transitionSeconds": v })),
            )),
        ),
    ];

    // Displays ----------------------------------------------------------------
    let (same, orient) = (w.same_on_all_displays, w.match_orientation);
    let mut displays = vec![
        widgets::setting(
            l,
            "same",
            "Same painting on every display",
            None,
            Some(widgets::toggle(l, same, move |s| {
                wallpaper(s, json!({ "sameOnAllDisplays": !same }))
            })),
        ),
        widgets::setting(
            l,
            "orientation",
            "Match display orientation",
            Some(
                "Only landscape works on landscape screens and portrait works on portrait screens."
                    .into(),
            ),
            Some(widgets::toggle(l, orient, move |s| {
                wallpaper(s, json!({ "matchOrientation": !orient }))
            })),
        ),
    ];
    for d in &snap.displays {
        displays.push(widgets::setting(
            l,
            &format!("display-{}", d.info.label),
            d.info.name.clone(),
            Some(format!(
                "{:.0}×{:.0} pt @{}x{}",
                d.info.width,
                d.info.height,
                d.info.scale,
                if d.info.primary {
                    " · main display"
                } else {
                    ""
                }
            )),
            None,
        ));
    }

    scroll_page(
        "wallpaper",
        vec![
            widgets::section(
                l,
                "desktop",
                "Desktop",
                Some("How paintings meet your screen.".into()),
                vec![],
                desktop,
            ),
            widgets::section(
                l,
                "fit",
                "Fit",
                Some("Galeria compares each painting's aspect ratio with the display's.".into()),
                vec![],
                fit,
            ),
            widgets::section(
                l,
                "wall",
                "Wall",
                Some("What surrounds a painting that is shown in full.".into()),
                vec![],
                wall,
            ),
            widgets::section(l, "displays", "Displays", None, vec![], displays),
        ],
    )
}
