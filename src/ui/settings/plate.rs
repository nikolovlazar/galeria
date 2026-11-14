//! Info Plate: its style, what it says and where it sits.

use super::now::preview;
use super::widgets::{self, Kind, Tone};
use super::{scroll_page, SettingsUi};
use crate::actions;
use crate::core;
use crate::settings::PlatePosition;
use crate::ui::theme::Look;
use rune::{col, el, row, text, Color, Cursor, Cx, El, TextEdit, TextStyle};
use serde_json::json;

#[derive(Default)]
pub struct State {
    pub new_style: TextEdit,
}

const ANCHORS: [(f64, f64, &str, &str); 9] = [
    (0.0, 0.0, "↖", "Top left"),
    (0.5, 0.0, "↑", "Top centre"),
    (1.0, 0.0, "↗", "Top right"),
    (0.0, 0.5, "←", "Left"),
    (0.5, 0.5, "•", "Centre"),
    (1.0, 0.5, "→", "Right"),
    (0.0, 1.0, "↙", "Bottom left"),
    (0.5, 1.0, "↓", "Bottom centre"),
    (1.0, 1.0, "↘", "Bottom right"),
];

const FIELDS: [(&str, &str); 11] = [
    ("artist", "Artist"),
    ("artistBio", "Nationality and life dates"),
    ("date", "Date"),
    ("medium", "Medium"),
    ("dimensions", "Dimensions"),
    ("description", "Description"),
    ("museum", "Museum and credit line"),
    ("credit", "Image credit"),
    ("license", "License"),
    ("accession", "Accession number"),
    ("qr", "QR code to the museum page"),
];

fn percent(v: f64) -> String {
    format!("{}%", (v * 100.0).round())
}

fn points(v: f64) -> String {
    format!("{} pt", v.round())
}

fn plate(s: &SettingsUi, patch: serde_json::Value) {
    s.cx.patch(json!({ "plate": patch }));
}

pub fn page(s: &SettingsUi, l: &Look, layer: &mut Vec<El>, cx: &mut Cx<SettingsUi>) -> El {
    let snap = &s.snap;
    let ps = &snap.settings.plate;

    // Style: every style drawn live over the current painting ----------------------
    let room = cx
        .layout_size("style-gallery")
        .map(|[w, _]| w)
        .unwrap_or(772.0)
        .max(300.0);
    let gap = 14.0;
    let columns = if room >= 560.0 { 2.0 } else { 1.0 };
    let card_w = ((room - gap * (columns - 1.0)) / columns).floor();
    let styles = snap
        .builtin_styles
        .iter()
        .map(|b| (b.id.to_string(), b.name.to_string(), false, None))
        .chain(
            snap.custom_styles
                .iter()
                .map(|c| (c.id.clone(), c.name.clone(), true, c.error.clone())),
        );
    let sample = s.sample();
    let mut cards = Vec::new();
    for (id, name, custom, error) in styles {
        let on = ps.style == id;
        let mut shown = ps.clone();
        shown.style = id.clone();
        let picture = match sample {
            Some(p) => preview(
                s,
                l,
                p,
                &shown,
                card_w - 16.0,
                1.6,
                0.5,
                PlatePosition { x: 0.5, y: 1.0 },
            ),
            None => el()
                .size(card_w - 16.0, ((card_w - 16.0) / 1.6).round())
                .radius(l.kit.radius.md)
                .fill(l.well),
        };
        let mut caption = row().w_full().px(4.0).gap(8.0).center_y().child(
            text(name.clone())
                .font(TextStyle::new(13.0, if on { 600.0 } else { 450.0 }).lh(18.0))
                .color(if on { l.gold } else { l.text })
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
        if let Some(e) = error {
            caption = caption.child(widgets::chip(l, "Can't be read", Tone::Bad).help(e));
        }
        if custom {
            let file = id.clone();
            caption = caption.child(widgets::link(l, "Edit", move |s, _| {
                let file = file.clone();
                s.cx.run(move |app| {
                    let _ = actions::open_style_file(app, &file);
                });
            }));
        }
        let pick = id.clone();
        cards.push(
            col()
                .key(format!("style-{id}"))
                .role("radio")
                .label(name)
                .checked(on)
                .focusable(true)
                .w(card_w)
                .padding(8.0)
                .gap(8.0)
                .radius(l.kit.radius.lg)
                .fill(if on { l.gold_wash } else { Color::TRANSPARENT })
                .border(
                    1.0,
                    if on {
                        l.gold_border
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .hover(l.kit.c.hover)
                .cursor(Cursor::Pointer)
                .on_press(move |s: &mut SettingsUi| {
                    plate(s, json!({ "style": pick, "visible": true }))
                })
                .child(picture)
                .child(caption),
        );
    }
    let create = |s: &mut SettingsUi| {
        let name = s.plate.new_style.text.trim().to_string();
        if name.is_empty() {
            return;
        }
        s.plate.new_style = TextEdit::new("");
        s.cx.run(move |app| {
            if let Err(e) = actions::create_custom_style(app, &name) {
                log::warn!("custom style: {e}");
            }
        });
    };
    let style = vec![
        widgets::block(el().key("style-gallery").w_full().child(row().w_full().flex_wrap().gaps(gap, gap).children(cards))),
        widgets::setting(
            l,
            "new-style",
            "New custom style",
            Some("Creates a documented file in the styles folder and opens it. Saving the file updates the desktop at once.".into()),
            Some(
                row()
                    .gap(8.0)
                    .child(widgets::field(
                        l,
                        &s.plate.new_style,
                        "Style name",
                        Some(180.0),
                        |s: &mut SettingsUi| &mut s.plate.new_style,
                        move |s, _| create(s),
                    ))
                    .child(widgets::button_if(
                        l,
                        "Create",
                        Kind::Normal,
                        !s.plate.new_style.text.trim().is_empty(),
                        move |s, _| create(s),
                    )),
            ),
        ),
    ];

    // Plate -------------------------------------------------------------------------
    let (visible, foreign) = (ps.visible, ps.foreign_descriptions);
    let lines: Vec<(String, String)> = [
        (3, "3 lines"),
        (4, "4 lines"),
        (6, "6 lines"),
        (8, "8 lines"),
        (12, "12 lines"),
        (0, "Everything"),
    ]
    .iter()
    .map(|(n, name)| (n.to_string(), name.to_string()))
    .collect();
    let fades: Vec<(String, String)> = [
        (0, "Never"),
        (10, "10 seconds"),
        (30, "30 seconds"),
        (60, "1 minute"),
        (300, "5 minutes"),
    ]
    .iter()
    .map(|(n, name)| (n.to_string(), name.to_string()))
    .collect();
    let looks = vec![
        widgets::setting(
            l,
            "visible",
            "Show the info plate",
            None,
            Some(widgets::toggle(l, visible, move |s| plate(s, json!({ "visible": !visible })))),
        ),
        widgets::setting(
            l,
            "size",
            "Size",
            None,
            Some(widgets::slider(l, s, "plate-scale", ps.scale, (0.6, 1.8, 0.05), percent, |s, v| {
                plate(s, json!({ "scale": v }))
            })),
        ),
        widgets::setting(
            l,
            "width",
            "Width",
            None,
            Some(widgets::slider(l, s, "plate-width", ps.width, (240.0, 640.0, 10.0), points, |s, v| {
                plate(s, json!({ "width": v }))
            })),
        ),
        widgets::setting(
            l,
            "lines",
            "Description length",
            None,
            Some(widgets::picker(l, s, layer, "description-lines", &lines, &ps.description_lines.to_string(), 150.0, |s, id, _| {
                plate(s, json!({ "descriptionLines": id.parse::<u32>().unwrap_or(6) }))
            })),
        ),
        widgets::setting(
            l,
            "foreign",
            "Descriptions in other languages",
            Some("Some museum texts only exist in the original language (the Rijksmuseum often writes in Dutch).".into()),
            Some(widgets::toggle(l, foreign, move |s| plate(s, json!({ "foreignDescriptions": !foreign })))),
        ),
        widgets::setting(
            l,
            "fade",
            "Fade out after",
            Some("The plate reappears with every new painting.".into()),
            Some(widgets::picker(l, s, layer, "auto-hide", &fades, &ps.auto_hide_seconds.to_string(), 150.0, |s, id, _| {
                plate(s, json!({ "autoHideSeconds": id.parse::<u32>().unwrap_or(0) }))
            })),
        ),
    ];

    // Position ------------------------------------------------------------------------
    let dragged = !ps.positions.is_empty();
    let mut grid = col().gap(4.0);
    for line in ANCHORS.chunks(3) {
        let mut r = row().gap(4.0);
        for (x, y, arrow, name) in line {
            let (x, y) = (*x, *y);
            let on = !dragged
                && (ps.default_position.x - x).abs() < 0.01
                && (ps.default_position.y - y).abs() < 0.01;
            r = r.child(
                row()
                    .key(format!("anchor-{x}-{y}"))
                    .role("radio")
                    .label(*name)
                    .checked(on)
                    .focusable(true)
                    .size(30.0, 26.0)
                    .center()
                    .radius(6.0)
                    .fill(if on { l.gold } else { l.text.alpha(0.07) })
                    .hover(l.kit.c.hover)
                    .cursor(Cursor::Pointer)
                    .help(*name)
                    .on_press(move |s: &mut SettingsUi| {
                        s.cx.run(move |app| core::set_plate_position(app, "wallpaper", None, x, y))
                    })
                    .child(
                        text(*arrow)
                            .font(TextStyle::new(13.0, 500.0))
                            .color(if on { l.on_gold } else { l.muted })
                            .a11y_hidden(),
                    ),
            );
        }
        grid = grid.child(r);
    }
    let live = snap.settings.wallpaper.live && snap.live_supported;
    let arranging = snap.arrange;
    let position = vec![
        widgets::setting(
            l,
            "anchor",
            "Anchor",
            dragged.then(|| {
                "Some displays use a position you dragged; picking an anchor resets them."
                    .to_string()
            }),
            Some(grid),
        ),
        widgets::disabled(
            widgets::setting(
                l,
                "arrange",
                "Drag on the desktop",
                Some("Lifts the wallpaper above your icons until you press Done (or Esc).".into()),
                Some(widgets::button(
                    l,
                    if arranging {
                        "Done Arranging"
                    } else {
                        "Arrange…"
                    },
                    if arranging {
                        Kind::Primary
                    } else {
                        Kind::Normal
                    },
                    move |s, _| s.cx.run(move |app| core::set_arrange(app, !arranging)),
                )),
            ),
            !live,
        ),
        widgets::setting(
            l,
            "margin",
            "Distance from screen edges",
            None,
            Some(widgets::slider(
                l,
                s,
                "plate-margin",
                ps.margin,
                (0.0, 160.0, 4.0),
                points,
                |s, v| plate(s, json!({ "margin": v })),
            )),
        ),
    ];

    // Details ---------------------------------------------------------------------------
    let shown = serde_json::to_value(&ps.fields).unwrap_or_default();
    let checks: Vec<El> = FIELDS
        .iter()
        .map(|(field, name)| {
            let on = shown.get(*field).and_then(|v| v.as_bool()).unwrap_or(false);
            el().w(250.0)
                .child(widgets::checkbox(l, on, name, move |s| {
                    plate(s, json!({ "fields": { *field: !on } }))
                }))
        })
        .collect();
    let details =
        vec![widgets::block(col().w_full().child(
            row().w_full().flex_wrap().gaps(10.0, 12.0).children(checks),
        ))];

    scroll_page(
        "plate",
        vec![
            widgets::section(
                l,
                "style",
                "Style",
                Some("Every style is drawn live over the current painting.".into()),
                vec![widgets::button(
                    l,
                    "Open Styles Folder",
                    Kind::Normal,
                    |s, _| s.cx.run(actions::open_styles_folder),
                )],
                style,
            ),
            widgets::section(l, "plate", "Plate", None, vec![], looks),
            widgets::section(
                l,
                "position",
                "Position",
                Some("Pick a spot, or drag the plate directly on the desktop.".into()),
                vec![],
                position,
            ),
            widgets::section(l, "details", "Details", None, vec![], details),
        ],
    )
}
