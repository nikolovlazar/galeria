//! Rotation: which paintings take part, playlists, light and dark, and timing.

use super::now::{create_playlist, save_playlists};
use super::widgets::{self, Kind};
use super::{scroll_page, SettingsUi, Tab};
use crate::core;
use crate::settings::{Order, ScopeKind};
use crate::ui::theme::Look;
use rune::{col, el, row, text, El, TextEdit};
use serde_json::json;

#[derive(Default)]
pub struct State {
    pub new_playlist: TextEdit,
    /// The playlist being renamed and its name so far.
    pub renaming: Option<(String, TextEdit)>,
}

const INTERVALS: [(u32, &str); 9] = [
    (0, "Never"),
    (1, "Every minute"),
    (5, "Every 5 minutes"),
    (15, "Every 15 minutes"),
    (30, "Every 30 minutes"),
    (60, "Every hour"),
    (180, "Every 3 hours"),
    (720, "Every 12 hours"),
    (1440, "Every day"),
];

fn whole(v: f64) -> String {
    format!("{}", v.round())
}

fn toggled(list: &[String], v: &str) -> Vec<String> {
    if list.iter().any(|x| x == v) {
        list.iter().filter(|x| *x != v).cloned().collect()
    } else {
        list.iter()
            .cloned()
            .chain(std::iter::once(v.to_string()))
            .collect()
    }
}

/// Brightness of every painting in the library, with the light/dark dividing line.
fn tone_chart(s: &SettingsUi, l: &Look) -> El {
    const BINS: usize = 32;
    const HEIGHT: f32 = 84.0;
    let mut bins = [0usize; BINS];
    if let Some(lib) = &s.library {
        for p in lib
            .paintings
            .iter()
            .filter(|p| !p.excluded && p.museum_enabled)
        {
            if let Some(b) = p.brightness {
                bins[((b / 256.0 * BINS as f64) as usize).min(BINS - 1)] += 1;
            }
        }
    }
    let max = bins.iter().copied().max().unwrap_or(0).max(1) as f32;
    let t = &s.snap.tone;
    let mut bars = row().w_full().h(HEIGHT).gap(2.0).items_end();
    for (i, n) in bins.iter().enumerate() {
        let light = (i as f64 + 0.5) / BINS as f64 * 256.0 >= t.threshold;
        bars = bars.child(
            el().grow(1.0)
                .h((*n as f32 / max * HEIGHT).max(if *n > 0 { 2.0 } else { 0.0 }))
                .corners([2.0, 2.0, 0.0, 0.0])
                .fill(if light { l.gold } else { l.text.alpha(0.32) })
                .help(widgets::plural(*n, "painting", "paintings")),
        );
    }
    let legend = row()
        .w_full()
        .justify_between()
        .child(
            text(format!("● {} dark", t.dark))
                .font(widgets::small())
                .color(l.text.alpha(0.6)),
        )
        .child(
            text(format!("dividing line {}", t.threshold.round()))
                .font(widgets::small())
                .color(l.muted),
        )
        .child(
            text(format!("{} light ●", t.light))
                .font(widgets::small())
                .color(l.gold),
        );
    let mut c = col()
        .key("tone-chart")
        .w_full()
        .gap(8.0)
        .child(
            el().w_full().h(HEIGHT).child(bars).child(
                // The dividing line, where it falls on the 0–255 scale.
                row()
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .inert()
                    .a11y_hidden()
                    .child(el().grow((t.threshold / 256.0) as f32))
                    .child(el().w(1.5).h_full().fill(l.text.alpha(0.75)))
                    .child(el().grow(1.0 - (t.threshold / 256.0) as f32)),
            ),
        )
        .child(legend);
    if t.unknown > 0 {
        c = c.child(widgets::hint(
            l,
            format!(
                "{} not measured yet. Galeria is working through them.",
                t.unknown
            ),
        ));
    }
    widgets::block(c)
}

pub fn page(s: &SettingsUi, l: &Look, layer: &mut Vec<El>) -> El {
    let snap = &s.snap;
    let r = &snap.settings.rotation;
    let sc = &r.scope;
    let ap = &snap.settings.appearance;
    let scope = |s: &SettingsUi, patch: serde_json::Value| {
        s.cx.patch(json!({ "rotation": { "scope": patch } }))
    };
    let rotation =
        |s: &SettingsUi, patch: serde_json::Value| s.cx.patch(json!({ "rotation": patch }));
    let appearance =
        |s: &SettingsUi, patch: serde_json::Value| s.cx.patch(json!({ "appearance": patch }));
    let theme = match snap.theme {
        Some("dark") => Some("Dark Mode"),
        Some("light") => Some("Light Mode"),
        _ => None,
    };

    // What to show --------------------------------------------------------------
    let mut kinds: Vec<(String, String)> = vec![
        ("all".into(), format!("Everything ({})", snap.library.valid)),
        (
            "favorites".into(),
            format!("Favorites ({})", snap.settings.favorites.len()),
        ),
    ];
    if !snap.settings.playlists.is_empty() {
        kinds.push(("playlist".into(), "A playlist".into()));
    }
    kinds.push(("filter".into(), "Chosen museums and genres".into()));
    if !sc.selection.is_empty() {
        kinds.push((
            "selection".into(),
            format!("My selection ({})", sc.selection.len()),
        ));
    }
    let kind_id = serde_json::to_value(sc.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let first_playlist = snap.settings.playlists.first().map(|p| p.id.clone());
    let has_playlist = sc.playlist.is_some();
    let mut what = vec![widgets::setting(
        l,
        "paintings",
        "Paintings",
        Some("Hidden paintings and museums that are switched off never appear, whatever you choose here.".into()),
        Some(widgets::picker(l, s, layer, "scope-kind", &kinds, &kind_id, 250.0, move |s, kind, _| {
            if kind == "playlist" && !has_playlist {
                scope(s, json!({ "kind": kind, "playlist": first_playlist }));
            } else {
                scope(s, json!({ "kind": kind }));
            }
        })),
    )];
    match sc.kind {
        ScopeKind::Playlist => {
            let lists: Vec<(String, String)> = snap
                .settings
                .playlists
                .iter()
                .map(|p| (p.id.clone(), format!("{} ({})", p.name, p.items.len())))
                .collect();
            what.push(widgets::setting(
                l,
                "playlist",
                "Playlist",
                None,
                Some(widgets::picker(
                    l,
                    s,
                    layer,
                    "scope-playlist",
                    &lists,
                    sc.playlist.as_deref().unwrap_or(""),
                    250.0,
                    move |s, id, _| scope(s, json!({ "playlist": id })),
                )),
            ));
        }
        ScopeKind::Filter => {
            let genres = snap
                .genres
                .iter()
                .map(|g| {
                    let (id, list) = (g.id.clone(), sc.genres.clone());
                    widgets::choice(
                        l,
                        format!("genre-{}", g.id),
                        g.name.clone(),
                        Some(g.count),
                        sc.genres.contains(&g.id),
                        move |s, _| scope(s, json!({ "genres": toggled(&list, &id) })),
                    )
                })
                .collect();
            let museums = snap
                .museums
                .iter()
                .filter(|m| m.enabled)
                .map(|m| {
                    let (name, list) = (m.name.clone(), sc.museums.clone());
                    widgets::choice(
                        l,
                        format!("museum-{}", m.name),
                        m.name.clone(),
                        Some(m.count),
                        sc.museums.contains(&m.name),
                        move |s, _| scope(s, json!({ "museums": toggled(&list, &name) })),
                    )
                })
                .collect();
            what.push(widgets::block(
                col()
                    .w_full()
                    .gap(10.0)
                    .child(widgets::hint(
                        l,
                        if sc.genres.is_empty() {
                            "Genres (any)"
                        } else {
                            "Genres"
                        },
                    ))
                    .child(widgets::wrap(genres))
                    .child(el().h(2.0))
                    .child(widgets::hint(
                        l,
                        if sc.museums.is_empty() {
                            "Museums (any)"
                        } else {
                            "Museums"
                        },
                    ))
                    .child(widgets::wrap(museums)),
            ));
        }
        ScopeKind::Selection => what.push(widgets::setting(
            l,
            "selection",
            format!(
                "{} you picked",
                widgets::plural(sc.selection.len(), "painting", "paintings")
            ),
            Some("Pick a different set in Browse: Select, then Show Only These.".into()),
            Some(widgets::button(l, "Open Browse", Kind::Normal, |s, _| {
                s.go(Tab::Browse)
            })),
        )),
        ScopeKind::Favorites if snap.settings.favorites.is_empty() => what.push(widgets::setting(
            l,
            "no-favorites",
            "No favorites yet",
            Some("Star paintings in Browse or from the menu bar.".into()),
            Some(widgets::button(l, "Open Browse", Kind::Normal, |s, _| {
                s.go(Tab::Browse)
            })),
        )),
        _ => {}
    }
    let what_says = if snap.rotation.empty {
        "Nothing matches this choice, so every painting is shown for now.".to_string()
    } else {
        format!(
            "{} part{}.",
            if snap.rotation.count == 1 {
                "1 painting takes".to_string()
            } else {
                format!("{} paintings take", snap.rotation.count)
            },
            match theme {
                Some(mode) if ap.match_system =>
                    format!(", {} of them suited to {mode}", snap.rotation.matching),
                _ => String::new(),
            }
        )
    };

    // Playlists -------------------------------------------------------------------
    let mut playlists = Vec::new();
    for pl in &snap.settings.playlists {
        let showing =
            sc.kind == ScopeKind::Playlist && sc.playlist.as_deref() == Some(pl.id.as_str());
        let count = widgets::plural(pl.items.len(), "painting", "paintings");
        if let Some((_, edit)) = s.rotation.renaming.as_ref().filter(|(id, _)| *id == pl.id) {
            let commit = |s: &mut SettingsUi| {
                let Some((id, edit)) = s.rotation.renaming.take() else {
                    return;
                };
                let name = edit.text.trim().to_string();
                if name.is_empty() {
                    return;
                }
                let lists = s
                    .snap
                    .settings
                    .playlists
                    .iter()
                    .cloned()
                    .map(|mut p| {
                        if p.id == id {
                            p.name = name.clone();
                        }
                        p
                    })
                    .collect();
                save_playlists(s, lists);
            };
            playlists.push(
                row()
                    .key(format!("playlist-{}", pl.id))
                    .w_full()
                    .min_h(46.0)
                    .px(16.0)
                    .py(9.0)
                    .gap(8.0)
                    .center_y()
                    .child(el().grow(1.0).min_w(0.0).child(widgets::field(
                        l,
                        edit,
                        "Playlist name",
                        None,
                        |s: &mut SettingsUi| &mut s.rotation.renaming.as_mut().expect("renaming").1,
                        move |s, _| commit(s),
                    )))
                    .child(widgets::button(l, "Save", Kind::Primary, move |s, _| {
                        commit(s)
                    }))
                    .child(widgets::button(l, "Cancel", Kind::Quiet, |s, _| {
                        s.rotation.renaming = None
                    })),
            );
            continue;
        }
        let (show_id, rename_id, delete_id, name) =
            (pl.id.clone(), pl.id.clone(), pl.id.clone(), pl.name.clone());
        playlists.push(widgets::setting(
            l,
            &format!("playlist-{}", pl.id),
            pl.name.clone(),
            Some(count),
            Some(
                row()
                    .gap(8.0)
                    .child(widgets::button_if(
                        l,
                        if showing { "Showing" } else { "Show" },
                        if showing { Kind::Primary } else { Kind::Normal },
                        !pl.items.is_empty(),
                        move |s, _| scope(s, json!({ "kind": "playlist", "playlist": show_id })),
                    ))
                    .child(widgets::button(l, "Rename", Kind::Normal, move |s, _| {
                        let mut edit = TextEdit::new(name.clone());
                        edit.focused = true;
                        s.rotation.renaming = Some((rename_id.clone(), edit));
                    }))
                    .child(widgets::button(l, "Delete", Kind::Danger, move |s, _| {
                        let lists = s
                            .snap
                            .settings
                            .playlists
                            .iter()
                            .filter(|p| p.id != delete_id)
                            .cloned()
                            .collect();
                        save_playlists(s, lists);
                    })),
            ),
        ));
    }
    let add = |s: &mut SettingsUi| {
        let name = s.rotation.new_playlist.text.trim().to_string();
        if !name.is_empty() {
            create_playlist(s, &name, Vec::new());
            s.rotation.new_playlist = TextEdit::new("");
        }
    };
    playlists.push(
        row()
            .key("new-playlist")
            .w_full()
            .min_h(46.0)
            .px(16.0)
            .py(9.0)
            .gap(8.0)
            .center_y()
            .child(el().grow(1.0).min_w(0.0).child(widgets::field(
                l,
                &s.rotation.new_playlist,
                "New playlist name",
                None,
                |s: &mut SettingsUi| &mut s.rotation.new_playlist,
                move |s, _| add(s),
            )))
            .child(widgets::button_if(
                l,
                "Add Playlist",
                Kind::Normal,
                !s.rotation.new_playlist.text.trim().is_empty(),
                move |s, _| add(s),
            )),
    );

    // Light & dark ------------------------------------------------------------------
    let (matching, immediately) = (ap.match_system, ap.switch_immediately);
    let custom = ap.threshold.is_some();
    let current_line = snap.tone.threshold.round();
    let mut line = row().gap(12.0).center_y().child(widgets::segmented(
        l,
        &["Automatic", "Custom"],
        Some(custom as usize),
        move |s, i| {
            appearance(
                s,
                json!({ "threshold": if i == 0 { None } else { Some(current_line) } }),
            )
        },
    ));
    if let Some(t) = ap.threshold {
        line = line.child(widgets::slider(
            l,
            s,
            "tone-threshold",
            t,
            (30.0, 220.0, 1.0),
            whole,
            move |s, v| appearance(s, json!({ "threshold": v })),
        ));
    }
    let overrides = ap.overrides.len();
    let light_dark = vec![
        widgets::setting(
            l,
            "match",
            "Match the system appearance",
            Some(format!(
                "Brighter paintings in Light Mode, darker ones in Dark Mode.{}",
                theme.map(|m| format!(" Your system is in {m} now.")).unwrap_or_default()
            )),
            Some(widgets::toggle(l, matching, move |s| appearance(s, json!({ "matchSystem": !matching })))),
        ),
        tone_chart(s, l),
        widgets::setting(
            l,
            "line",
            "Dividing line",
            Some(if custom {
                "Paintings at or above this brightness count as light.".to_string()
            } else {
                format!(
                    "Automatic: the middle of your library ({} of 255), so light and dark each get about half.",
                    snap.tone.auto_threshold.round()
                )
            }),
            Some(line),
        ),
        widgets::disabled(
            widgets::setting(
                l,
                "immediately",
                "Change as soon as the appearance switches",
                Some("Otherwise the next scheduled change picks a fitting painting.".into()),
                Some(widgets::toggle(l, immediately, move |s| {
                    appearance(s, json!({ "switchImmediately": !immediately }))
                })),
            ),
            !matching,
        ),
        widgets::block(widgets::hint(
            l,
            format!(
                "Disagree with a verdict? Open a painting in Browse and set it to Light, Dark or Both.{}",
                if overrides > 0 { format!(" You have set {overrides} by hand.") } else { String::new() }
            ),
        )),
    ];

    // Timing -----------------------------------------------------------------------
    let intervals: Vec<(String, String)> = INTERVALS
        .iter()
        .map(|(m, name)| (m.to_string(), name.to_string()))
        .collect();
    let paused = r.paused;
    let orders = [Order::Shuffle, Order::Sequential];
    let timing = vec![
        widgets::setting(
            l,
            "interval",
            "Change painting",
            None,
            Some(widgets::picker(
                l,
                s,
                layer,
                "interval",
                &intervals,
                &r.interval_minutes.to_string(),
                190.0,
                move |s, id, _| {
                    rotation(
                        s,
                        json!({ "intervalMinutes": id.parse::<u32>().unwrap_or(30) }),
                    )
                },
            )),
        ),
        widgets::setting(
            l,
            "order",
            "Order",
            Some("Shuffle goes through every painting in the rotation before repeating.".into()),
            Some(widgets::segmented(
                l,
                &["Shuffle", "Manifest order"],
                orders.iter().position(|o| *o == r.order),
                move |s, i| rotation(s, json!({ "order": orders[i] })),
            )),
        ),
        widgets::setting(
            l,
            "pause",
            "Pause rotation",
            None,
            Some(widgets::toggle(l, paused, move |s| {
                rotation(s, json!({ "paused": !paused }))
            })),
        ),
        widgets::setting(
            l,
            "now",
            "Change now",
            None,
            Some(widgets::button(l, "Next Painting", Kind::Normal, |s, _| {
                s.cx.run(|app| core::advance(app, None, true))
            })),
        ),
    ];

    scroll_page(
        "rotation",
        vec![
            widgets::section(l, "what", "What to Show", Some(what_says), vec![], what),
            widgets::section(
                l,
                "playlists",
                "Playlists",
                Some("Named sets of paintings. Fill them in Browse: Select, then Add to Playlist.".into()),
                vec![],
                playlists,
            ),
            widgets::section(
                l,
                "light-dark",
                "Light & Dark",
                Some("Galeria measures how bright each painting is and can follow your system's appearance.".into()),
                vec![],
                light_dark,
            ),
            widgets::section(
                l,
                "timing",
                "Timing",
                Some("When and in what order paintings change.".into()),
                vec![],
                timing,
            ),
        ],
    )
}
