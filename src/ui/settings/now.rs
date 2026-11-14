//! Now Showing: what hangs on each display, and everything known about it.

use super::widgets::{self, Kind, S};
use super::{scroll_page, SettingsUi};
use crate::actions;
use crate::core::{self, DisplayView, PaintingView, Snapshot};
use crate::settings::{PlateSettings, Playlist};
use crate::ui::plate;
use crate::ui::theme::{rgb, Look};
use rune::{col, el, row, text, El, SurfaceFit, TextStyle, Tx};
use serde_json::json;

// ---------------------------------------------------------------------------
// Playlists & per-painting choices (all stored in settings)

pub fn save_playlists(s: &S, playlists: Vec<Playlist>) {
    s.cx.patch(json!({ "playlists": playlists }));
}

pub fn add_to_playlist(s: &S, id: &str, ids: &[String]) {
    let lists = s
        .snap
        .settings
        .playlists
        .iter()
        .cloned()
        .map(|mut p| {
            if p.id == id {
                for i in ids {
                    if !p.items.contains(i) {
                        p.items.push(i.clone());
                    }
                }
            }
            p
        })
        .collect();
    save_playlists(s, lists);
}

pub fn remove_from_playlist(s: &S, id: &str, ids: &[String]) {
    let lists = s
        .snap
        .settings
        .playlists
        .iter()
        .cloned()
        .map(|mut p| {
            if p.id == id {
                p.items.retain(|i| !ids.contains(i));
            }
            p
        })
        .collect();
    save_playlists(s, lists);
}

pub fn create_playlist(s: &S, name: &str, ids: Vec<String>) {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut lists = s.snap.settings.playlists.clone();
    lists.push(Playlist {
        id: format!("pl-{stamp:x}"),
        name: name.trim().to_string(),
        items: ids,
    });
    save_playlists(s, lists);
}

/// `None` clears the manual choice, so brightness decides again.
pub fn set_tone(s: &S, ids: &[String], value: Option<&str>) {
    let overrides: serde_json::Map<String, serde_json::Value> =
        ids.iter().map(|id| (id.clone(), json!(value))).collect();
    s.cx.patch(json!({ "appearance": { "overrides": overrides } }));
}

pub fn genre_name<'a>(snap: &'a Snapshot, id: &'a str) -> &'a str {
    snap.genres
        .iter()
        .find(|g| g.id == id)
        .map(|g| g.name.as_str())
        .unwrap_or(id)
}

fn time_until(at: Option<u64>) -> Option<String> {
    let secs = at? as f64 - crate::util::now() as f64;
    if secs <= 60.0 {
        return Some("in under a minute".into());
    }
    let m = (secs / 60.0).round() as u64;
    Some(if m < 60 {
        format!("in {m} min")
    } else {
        format!("in {} h {} min", m / 60, m % 60)
    })
}

// ---------------------------------------------------------------------------

/// A thumbnail with the live plate over it, as on the desktop.
// Each argument is one independent thing the picture depends on; a struct would only rename them.
#[allow(clippy::too_many_arguments)]
pub fn preview(
    s: &S,
    l: &Look,
    p: &PaintingView,
    ps: &PlateSettings,
    width: f32,
    aspect: f32,
    zoom: f32,
    position: crate::settings::PlatePosition,
) -> El {
    let height = (width / aspect.max(0.2)).round();
    let style = plate::resolve(&ps.style, &s.snap.custom_styles);
    let thumb = s.cx.art.thumb(&p.id, p.bytes, 640);
    let m = 12.0;
    el().size(width, height)
        .radius(l.kit.radius.md)
        .clip_rounded(l.kit.radius.md)
        .fill(l.well)
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels_fit(&thumb, SurfaceFit::Fill)
                .a11y_hidden(),
        )
        .child(
            plate::place(
                plate::render(p, ps, &style, &s.cx.art, zoom),
                position,
                [m, m, width - 2.0 * m, height - 2.0 * m],
            )
            .inert(),
        )
}

/// Everything known about a painting, and what can be done with it.
/// `display` is set on Now Showing, where the painting is already on a desktop.
pub fn details(s: &S, l: &Look, p: &PaintingView, display: Option<&str>, compact: bool) -> El {
    let snap = &s.snap;
    let multi = snap.displays.len() > 1 && !snap.settings.wallpaper.same_on_all_displays;
    let join = |a: &Option<String>, b: Option<String>, sep: &str| {
        [a.clone(), b]
            .into_iter()
            .flatten()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(sep)
    };
    let brightness = match p.brightness {
        Some(b) => format!(
            "{} of 255{}",
            b.round(),
            match p.tone {
                Some(crate::tone::Tone::Light) => " · counts as light",
                Some(crate::tone::Tone::Dark) => " · counts as dark",
                None => " · shown in light and dark",
            }
        ),
        None => "Not measured yet".into(),
    };
    let facts = [
        (
            "Artist",
            join(
                &p.artist,
                p.artist_bio.as_ref().map(|b| format!("({b})")),
                " ",
            ),
        ),
        ("Date", p.date.clone().unwrap_or_default()),
        ("Medium", p.medium.clone().unwrap_or_default()),
        ("Dimensions", p.dimensions.clone().unwrap_or_default()),
        ("Culture", p.culture.clone().unwrap_or_default()),
        ("Collection", join(&p.museum, p.credit_line.clone(), " · ")),
        ("Accession", p.accession.clone().unwrap_or_default()),
        ("Image credit", p.credit.clone().unwrap_or_default()),
        ("License", p.license.clone().unwrap_or_default()),
        (
            "Genres",
            p.genres
                .iter()
                .map(|g| genre_name(snap, g))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        (
            "Scan",
            format!(
                "{}×{} px · {:.1} MP · {}",
                p.width,
                p.height,
                p.width as f64 * p.height as f64 / 1e6,
                widgets::bytes(p.bytes)
            ),
        ),
        ("Brightness", brightness),
        ("File", p.file.clone()),
    ];

    let mut c = col().key("details").grow(1.0).min_w(0.0).gap(12.0).child(
        col()
            .gap(2.0)
            .child(
                text(p.title.clone())
                    .role("heading")
                    .level(3)
                    .font(TextStyle::new(19.0, 650.0).lh(24.0).tracking(-0.2))
                    .color(l.text)
                    .wrap()
                    .w_full()
                    .selectable(true),
            )
            .children(p.original_title.as_ref().map(|t| {
                text(t.clone())
                    .font(widgets::body().italic())
                    .color(l.muted)
                    .wrap()
                    .w_full()
            })),
    );
    c = c.child(widgets::facts(l, &facts));
    if let Some(d) = &p.description {
        let source = p
            .description_source
            .as_ref()
            .map(|src| {
                let lang = p
                    .description_lang
                    .as_deref()
                    .filter(|x| *x != "en")
                    .map(|x| format!(" ({x})"))
                    .unwrap_or_default();
                format!(" — {src}{lang}")
            })
            .unwrap_or_default();
        c = c.child(
            text(format!("{d}{source}"))
                .font(widgets::body().lh(20.0))
                .color(l.text.alpha(0.86))
                .wrap()
                .w_full()
                .selectable(true),
        );
    }
    if let Some(d) = p.did_you_know.as_ref().filter(|_| !compact) {
        c = c.child(
            text(format!("Did you know? {d}"))
                .font(widgets::body().lh(20.0))
                .color(l.text.alpha(0.86))
                .wrap()
                .w_full(),
        );
    }
    if !p.museum_enabled {
        c = c.child(widgets::notice(
            l,
            "Its museum is switched off in Museums, so it is left out of the rotation.",
            false,
        ));
    }

    // Light & Dark: how this painting counts when the desktop follows the system's appearance.
    let tone = match p.tone_override {
        None => 0,
        Some(crate::tone::ToneOverride::Light) => 1,
        Some(crate::tone::ToneOverride::Dark) => 2,
        Some(crate::tone::ToneOverride::Both) => 3,
    };
    let id = p.id.clone();
    c = c.child(
        row()
            .w_full()
            .gap(12.0)
            .center_y()
            .child(
                text("Light & Dark")
                    .w(92.0)
                    .font(widgets::small())
                    .color(l.muted),
            )
            .child(widgets::segmented(
                l,
                &["Auto", "Light", "Dark", "Both"],
                Some(tone),
                move |s, i| {
                    set_tone(
                        s,
                        std::slice::from_ref(&id),
                        [None, Some("light"), Some("dark"), Some("both")][i],
                    )
                },
            )),
    );
    if !snap.settings.playlists.is_empty() {
        let chips = snap
            .settings
            .playlists
            .iter()
            .map(|pl| {
                let on = pl.items.contains(&p.id);
                let (list, painting) = (pl.id.clone(), p.id.clone());
                widgets::choice(
                    l,
                    format!("pl-{}", pl.id),
                    pl.name.clone(),
                    None,
                    on,
                    move |s, _| {
                        if on {
                            remove_from_playlist(s, &list, std::slice::from_ref(&painting));
                        } else {
                            add_to_playlist(s, &list, std::slice::from_ref(&painting));
                        }
                    },
                )
            })
            .collect();
        c = c.child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(
                    text("Playlists")
                        .w(92.0)
                        .pt(6.0)
                        .font(widgets::small())
                        .color(l.muted),
                )
                .child(el().grow(1.0).min_w(0.0).child(widgets::wrap(chips))),
        );
    }

    let mut actions: Vec<El> = Vec::new();
    let id = || p.id.clone();
    if display.is_none() {
        let pid = id();
        actions.push(widgets::button(
            l,
            "Show on Desktop",
            Kind::Primary,
            move |s, _| {
                let pid = pid.clone();
                s.cx.run(move |app| {
                    let _ = core::show(app, &pid, None);
                });
            },
        ));
        if multi {
            for d in &snap.displays {
                let (pid, key) = (id(), d.info.key.clone());
                actions.push(widgets::button(
                    l,
                    format!("On {}", d.info.name),
                    Kind::Normal,
                    move |s, _| {
                        let (pid, key) = (pid.clone(), key.clone());
                        s.cx.run(move |app| {
                            let _ = core::show(app, &pid, Some(key));
                        });
                    },
                ));
            }
        }
    }
    let (pid, on) = (id(), display.map(str::to_string));
    actions.push(widgets::button(
        l,
        "Open Viewer",
        Kind::Normal,
        move |s, _| {
            let (pid, on) = (pid.clone(), on.clone());
            s.cx.run(move |app| {
                if let Some(shell) = app.shell() {
                    shell.open_viewer(Some(pid), on);
                }
            });
        },
    ));
    for (label, url) in [
        ("Museum Page", &p.source_page),
        ("Wikipedia", &p.wikipedia_url),
    ] {
        if let Some(url) = url.clone() {
            actions.push(widgets::button(l, label, Kind::Normal, move |_, _| {
                let _ = actions::open_url(&url);
            }));
        }
    }
    let pid = id();
    actions.push(widgets::button(
        l,
        "Show File",
        Kind::Normal,
        move |s, _| {
            let pid = pid.clone();
            s.cx.run(move |app| {
                let _ = actions::reveal_painting(app, &pid);
            });
        },
    ));
    let (pid, favorite) = (id(), p.favorite);
    actions.push(widgets::button(
        l,
        if favorite {
            "★ Favorite"
        } else {
            "☆ Favorite"
        },
        Kind::Normal,
        move |s, _| {
            let pid = pid.clone();
            s.cx.run(move |app| core::set_favorite(app, &pid, !favorite));
        },
    ));
    let (pid, excluded) = (id(), p.excluded);
    actions.push(widgets::button(
        l,
        if excluded {
            "Allow Again"
        } else {
            "Never Show"
        },
        Kind::Normal,
        move |s, _| {
            let pid = pid.clone();
            s.cx.run(move |app| core::set_excluded(app, &pid, !excluded));
        },
    ));
    let pid = id();
    let confirming = s.confirm_delete.as_deref() == Some(pid.as_str());
    actions.push(widgets::button(
        l,
        "Delete…",
        Kind::Danger,
        move |s, _| {
            s.confirm_delete = Some(pid.clone());
        },
    ));
    let pid = id();
    actions.push(
        widgets::button(l, "Refresh Info", Kind::Quiet, move |s, _| {
            let pid = pid.clone();
            s.cx.run(move |app| app.core().refresh_enrichment(Some(&pid)));
        })
        .help("Fetch the description and artist details again"),
    );
    let mut c = c.child(el().w_full().pt(2.0).child(widgets::wrap(actions)));
    if confirming {
        let pid = id();
        c = c.child(
            col()
                .key("confirm-delete")
                .w_full()
                .padding(12.0)
                .gap(8.0)
                .radius(l.kit.radius.md)
                .fill(l.danger_wash)
                .border(l.hair, l.danger.alpha(0.45))
                .child(
                    text(format!(
                        "Delete “{}” from your art folder? The image file and its manifest entry will be permanently removed.",
                        p.title
                    ))
                    .font(widgets::small())
                    .color(l.text)
                    .wrap()
                    .w_full(),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            widgets::button(l, "Delete Painting", Kind::Danger, move |s, _| {
                                s.confirm_delete = None;
                                if s.browse.selected.as_deref() == Some(pid.as_str()) {
                                    s.browse.selected = None;
                                }
                                let pid = pid.clone();
                                s.cx.run(move |app| {
                                    if let Err(e) = actions::delete_paintings(app, &[pid]) {
                                        log::warn!("delete painting: {e}");
                                    }
                                });
                            })
                            .key("confirm-delete-yes"),
                        )
                        .child(
                            widgets::button(l, "Cancel", Kind::Normal, |s, _| {
                                s.confirm_delete = None;
                            })
                            .key("confirm-delete-no"),
                        ),
                ),
        );
    }
    c
}

fn display_card(s: &S, l: &Look, d: &DisplayView, multi: bool) -> El {
    let snap = &s.snap;
    let (w, h) = (d.info.width, d.info.height);
    let mut sub = format!("{w:.0}×{h:.0} pt · {:.2}:1", w / h.max(1.0));
    if let Some(next) = time_until(d.next_change_at) {
        sub.push_str(&format!(" · next painting {next}"));
    }
    if snap.settings.rotation.paused {
        sub.push_str(" · rotation paused");
    }
    let key = d.info.key.clone();
    let (prev_key, next_key) = (key.clone(), key.clone());
    let head = row()
        .w_full()
        .items_end()
        .gap(12.0)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(
                    text(if multi {
                        d.info.name.clone()
                    } else {
                        "On your desktop".to_string()
                    })
                    .role("heading")
                    .level(2)
                    .font(widgets::title(l))
                    .color(l.text),
                )
                .child(widgets::muted(l, sub)),
        )
        .child(
            row()
                .gap(8.0)
                .child(widgets::button_if(
                    l,
                    "← Previous",
                    Kind::Normal,
                    d.history_len > 0,
                    move |s: &mut S, _: &mut Tx| {
                        let key = prev_key.clone();
                        s.cx.run(move |app| core::advance(app, Some(key), false));
                    },
                ))
                .child(widgets::button(l, "Next →", Kind::Normal, move |s, _| {
                    let key = next_key.clone();
                    s.cx.run(move |app| core::advance(app, Some(key), true));
                })),
        );
    let body = match &d.painting {
        Some(p) => {
            let ps = &snap.settings.plate;
            let position = ps
                .positions
                .get(&key)
                .copied()
                .unwrap_or(ps.default_position);
            row()
                .w_full()
                .gap(24.0)
                .items_start()
                .child(
                    preview(s, l, p, ps, 340.0, (w / h.max(1.0)) as f32, 0.42, position)
                        .key("preview"),
                )
                .child(details(s, l, p, Some(&key), false))
        }
        None => widgets::card(l).child(widgets::block(widgets::muted(
            l,
            snap.library
                .error
                .clone()
                .unwrap_or_else(|| "No painting available.".into()),
        ))),
    };
    col()
        .key(format!("display-{}", d.info.label))
        .w_full()
        .gap(14.0)
        .child(head)
        .child(body)
}

pub fn page(s: &SettingsUi, l: &Look, _: &mut Vec<El>) -> El {
    let snap = &s.snap;
    if snap.library.root.is_none() {
        return col()
            .key("now")
            .w_full()
            .h_full()
            .center()
            .gap(10.0)
            .px(40.0)
            .child(
                el().size(72.0, 54.0)
                    .radius(6.0)
                    .border(3.0, l.gold)
                    .fill(rgb(0x000000).alpha(0.12))
                    .mb(8.0)
                    .a11y_hidden(),
            )
            .child(text("Choose your art folder").font(TextStyle::new(20.0, 650.0)).color(l.text))
            .child(
                el().max_w(460.0).child(
                    text("Galeria reads a folder containing a manifest.json and your painting files. Nothing is copied or bundled.")
                        .font(widgets::body())
                        .color(l.muted)
                        .wrap()
                        .align(rune::Align::Center),
                ),
            )
            .child(el().pt(8.0).child(widgets::button(l, "Choose Folder…", Kind::Primary, |s, tx| s.choose_folder(tx))));
    }
    let multi = snap.displays.len() > 1;
    let cards = snap
        .displays
        .iter()
        .map(|d| display_card(s, l, d, multi))
        .collect();
    scroll_page("now", cards)
}
