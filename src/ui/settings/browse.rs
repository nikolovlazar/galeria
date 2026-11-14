//! Browse: every painting in the library, to look through, choose from and group.

use super::now::{add_to_playlist, create_playlist, details, set_tone};
use super::widgets::{self, Kind};
use super::SettingsUi;
use crate::actions;
use crate::core::{LibraryView, PaintingView};
use crate::tone::Tone;
use crate::ui::icons;
use crate::ui::theme::{rgb, Look};
use rune::{
    col, el, icon, row, text, Color, Cursor, Cx, El, GrowthSpec, ScrollSpec, SurfaceFit, TextEdit,
    TextStyle, Tx,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

pub struct State {
    pub query: TextEdit,
    pub filter: String,
    pub museum: String,
    pub genre: String,
    pub playlist: String,
    pub sort: String,
    /// The painting whose details are open beside the grid.
    pub selected: Option<String>,
    /// Picking several paintings at once (for a playlist, a one-off rotation, light/dark…).
    pub picking: bool,
    pub picked: BTreeSet<String>,
    /// Asking to confirm batch deletion of `picked`.
    pub confirm_delete: bool,
    /// The name of a playlist being made from the picked paintings.
    pub new_playlist: Option<TextEdit>,
}

impl Default for State {
    fn default() -> Self {
        State {
            query: TextEdit::new(""),
            filter: "all".into(),
            museum: String::new(),
            genre: String::new(),
            playlist: String::new(),
            sort: "manifest".into(),
            selected: None,
            picking: false,
            picked: BTreeSet::new(),
            confirm_delete: false,
            new_playlist: None,
        }
    }
}

const FILTERS: [(&str, &str); 9] = [
    ("all", "All"),
    ("rotation", "In rotation"),
    ("favorites", "Favorites"),
    ("excluded", "Hidden"),
    ("light", "Light"),
    ("dark", "Dark"),
    ("landscape", "Landscape"),
    ("portrait", "Portrait"),
    ("panorama", "Panoramas (pan)"),
];

const SORTS: [(&str, &str); 4] = [
    ("manifest", "Manifest order"),
    ("title", "Title"),
    ("artist", "Artist"),
    ("brightness", "Dark to light"),
];

/// The paintings that pass the filters, in the chosen order (as indices into the library).
pub fn filtered(
    st: &State,
    lib: &LibraryView,
    playlists: &[crate::settings::Playlist],
) -> Vec<usize> {
    let needle = st.query.text.trim().to_lowercase();
    let in_list: Option<&Vec<String>> = (!st.playlist.is_empty())
        .then(|| {
            playlists
                .iter()
                .find(|p| p.id == st.playlist)
                .map(|p| &p.items)
        })
        .flatten();
    let has = |v: &Option<String>| {
        v.as_ref()
            .is_some_and(|v| v.to_lowercase().contains(&needle))
    };
    let mut out: Vec<usize> = lib
        .paintings
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            if !st.museum.is_empty() && p.museum.as_deref() != Some(st.museum.as_str()) {
                return false;
            }
            if !st.genre.is_empty() {
                let matches = if st.genre == crate::genres::OTHER {
                    p.genres.is_empty()
                } else {
                    p.genres.contains(&st.genre)
                };
                if !matches {
                    return false;
                }
            }
            if !st.playlist.is_empty() && !in_list.is_some_and(|l| l.contains(&p.id)) {
                return false;
            }
            let keep = match st.filter.as_str() {
                "rotation" => p.in_rotation,
                "favorites" => p.favorite,
                "excluded" => p.excluded,
                "landscape" => p.orientation == "landscape",
                "portrait" => p.orientation == "portrait",
                "panorama" => p.aspect > 2.0 || p.aspect < 0.5,
                "light" => p.tone == Some(Tone::Light),
                "dark" => p.tone == Some(Tone::Dark),
                _ => true,
            };
            if !keep {
                return false;
            }
            needle.is_empty()
                || p.title.to_lowercase().contains(&needle)
                || has(&p.original_title)
                || has(&p.artist)
                || has(&p.date)
                || has(&p.museum)
                || has(&p.medium)
                || p.tags.iter().any(|t| t.to_lowercase().contains(&needle))
        })
        .map(|(i, _)| i)
        .collect();
    let ps = &lib.paintings;
    match st.sort.as_str() {
        "title" => out.sort_by(|a, b| {
            ps[*a]
                .title
                .to_lowercase()
                .cmp(&ps[*b].title.to_lowercase())
        }),
        "artist" => out.sort_by(|a, b| {
            let key = |i: usize| {
                ps[i]
                    .artist
                    .as_ref()
                    .map(|a| a.to_lowercase())
                    .unwrap_or_else(|| "~".into())
            };
            key(*a).cmp(&key(*b))
        }),
        "brightness" => out.sort_by(|a, b| {
            ps[*a]
                .brightness
                .unwrap_or(999.0)
                .total_cmp(&ps[*b].brightness.unwrap_or(999.0))
        }),
        _ => {}
    }
    out
}

/// What pressing a tile's tick does.
pub type TickPress = std::rc::Rc<dyn Fn(&mut SettingsUi)>;

/// What a tile shows over its picture and how it reacts.
pub struct TileLook {
    pub title: String,
    pub sub: String,
    /// Dimmed: hidden by the user, switched off, or not downloadable.
    pub faded: bool,
    /// The one whose details are open.
    pub selected: bool,
    /// `Some(on)` while several are being picked.
    pub tick: Option<bool>,
    /// Pressing the tick itself, apart from the rest of the tile. Without it the tile's own press does it.
    pub on_tick: Option<TickPress>,
    pub badge: Option<(String, widgets::Tone)>,
    pub star: bool,
    /// Bottom-left and bottom-right labels over the picture.
    pub corner: Option<(String, widgets::Tone)>,
    pub size: Option<String>,
    pub tone: Option<Tone>,
}

/// One painting in a grid: its picture, then title and a line under it.
pub fn tile(l: &Look, key: String, picture: &rune::Pixels, width: f32, t: TileLook) -> El {
    let image_h = (width * 0.7).round();
    let white = Color::WHITE;
    let over = |e: El| e.h(18.0).px(7.0).center().radius(9.0);
    let mut image = el()
        .w(width)
        .h(image_h)
        .radius(l.kit.radius.md)
        .clip_rounded(l.kit.radius.md)
        .fill(l.well)
        .child(
            el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .pixels_fit(picture, SurfaceFit::Fill)
                .a11y_hidden(),
        );
    if let Some((label, tone)) = &t.badge {
        let (ink, wash) = match tone {
            widgets::Tone::Bad => (white, rgb(0xb43a2a)),
            widgets::Tone::Gold => (l.on_gold, l.gold),
            _ => (white, Color::BLACK.alpha(0.62)),
        };
        image = image.child(
            over(row()).absolute(8.0, 8.0).fill(wash).child(
                text(label.clone())
                    .font(TextStyle::new(10.5, 600.0).lh(13.0))
                    .color(ink),
            ),
        );
    }
    if t.star {
        image = image.child(
            el().pin(
                None,
                Some(7.0),
                Some(if t.tick.is_some() { 36.0 } else { 8.0 }),
                None,
            )
            .child(icon(&icons::STAR, 15.0).color(rgb(0xf2c84b))),
        );
    }
    if let Some(on) = t.tick {
        let mut tick = row()
            .pin(None, Some(8.0), Some(8.0), None)
            .size(22.0, 22.0)
            .center()
            .radius(11.0)
            .fill(if on { l.gold } else { Color::BLACK.alpha(0.35) })
            .border(1.5, if on { l.gold } else { white.alpha(0.9) })
            .children(on.then(|| icon(&icons::CHECK, 13.0).color(l.on_gold)));
        if let Some(f) = t.on_tick.clone() {
            tick = tick
                .role("checkbox")
                .checked(on)
                .label("Select")
                .cursor(Cursor::Pointer)
                .on_press(move |s: &mut SettingsUi| f(s));
        }
        image = image.child(tick);
    }
    if let Some((label, tone)) = &t.corner {
        let ink = if *tone == widgets::Tone::Good {
            rgb(0xc9f0b8)
        } else {
            rgb(0xffc9c0)
        };
        image = image.child(
            over(row())
                .pin(Some(8.0), None, None, Some(8.0))
                .fill(Color::BLACK.alpha(0.62))
                .child(
                    text(label.clone())
                        .font(TextStyle::new(10.5, 600.0).lh(13.0))
                        .color(ink),
                ),
        );
    }
    if let Some(size) = &t.size {
        image = image.child(
            over(row())
                .pin(None, None, Some(8.0), Some(8.0))
                .fill(Color::BLACK.alpha(0.62))
                .child(
                    text(size.clone())
                        .font(TextStyle::new(10.5, 500.0).lh(13.0).features("'tnum' 1"))
                        .color(white),
                ),
        );
    }
    if let Some(tone) = t.tone {
        // A small sun or moon: which appearance the painting is shown in.
        let light = tone == Tone::Light;
        image = image.child(
            el().pin(None, None, Some(8.0), Some(8.0))
                .size(10.0, 10.0)
                .radius(5.0)
                .fill(if light { rgb(0xf6e7b4) } else { rgb(0x2a2d3a) })
                .border(
                    1.0,
                    if light {
                        Color::BLACK.alpha(0.25)
                    } else {
                        white.alpha(0.5)
                    },
                )
                .help(if light {
                    "Counts as light"
                } else {
                    "Counts as dark"
                }),
        );
    }
    let marked = t.selected || t.tick == Some(true);
    col()
        .key(key)
        .role("button")
        .label(t.title.clone())
        .focusable(true)
        .w(width + 8.0)
        .padding(4.0)
        .gap(6.0)
        .radius(l.kit.radius.md + 4.0)
        .fill(if marked {
            l.gold_wash
        } else {
            Color::TRANSPARENT
        })
        .border(
            1.0,
            if marked {
                l.gold_border
            } else {
                Color::TRANSPARENT
            },
        )
        .hover(l.kit.c.hover)
        .cursor(Cursor::Pointer)
        .child(if t.faded { image.dim(0.45) } else { image })
        .child(
            col()
                .w(width)
                .px(2.0)
                .child(
                    text(t.title)
                        .font(TextStyle::new(12.5, 550.0).lh(17.0))
                        .color(l.text)
                        .truncate()
                        .a11y_hidden(),
                )
                .child(
                    text(t.sub)
                        .font(TextStyle::new(11.5, 400.0).lh(16.0))
                        .color(l.muted)
                        .truncate(),
                ),
        )
}

/// How many tiles fit across `room`, and how wide each one's picture is.
pub fn columns(room: f32, gap: f32) -> (usize, f32) {
    const MIN: f32 = 150.0;
    let n = (((room + gap) / (MIN + 8.0 + gap)).floor() as usize).max(1);
    let outer = (room - gap * (n as f32 - 1.0)) / n as f32;
    (n, (outer - 8.0).floor().max(60.0))
}

fn options(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(id, name)| (id.to_string(), name.to_string()))
        .collect()
}

fn stop_picking(s: &mut SettingsUi) {
    s.browse.picking = false;
    s.browse.picked.clear();
    s.browse.confirm_delete = false;
    s.browse.new_playlist = None;
}

fn bulk_bar(s: &SettingsUi, l: &Look, layer: &mut Vec<El>, shown: &Rc<Vec<String>>) -> El {
    let snap = &s.snap;
    let n = s.browse.picked.len();
    let any = n > 0;
    let all = shown.clone();
    let picked = |s: &SettingsUi| s.browse.picked.iter().cloned().collect::<Vec<String>>();
    let mut bar = row()
        .key("bulk")
        .w_full()
        .min_h(44.0)
        .px(10.0)
        .py(8.0)
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .radius(l.kit.radius.lg)
        .fill(l.gold_wash)
        .border(l.hair, l.gold_border)
        .child(text(format!("{n} selected")).font(widgets::strong()).color(l.text))
        .child(widgets::button(l, format!("All {}", shown.len()), Kind::Quiet, move |s, _| {
            s.browse.picked = all.iter().cloned().collect()
        }))
        .child(widgets::button_if(l, "None", Kind::Quiet, any, |s, _| {
            s.browse.picked.clear();
            s.browse.confirm_delete = false;
        }))
        .child(el().grow(1.0))
        .child(
            widgets::button_if(l, "Show Only These", Kind::Primary, any, move |s, _| {
                s.cx.patch(json!({ "rotation": { "scope": { "kind": "selection", "selection": picked(s) } } }));
                stop_picking(s);
            })
            .help("Rotate through just these paintings"),
        );
    match &s.browse.new_playlist {
        None => {
            let mut lists: Vec<(String, String)> = vec![(String::new(), "Add to playlist…".into())];
            lists.extend(
                snap.settings
                    .playlists
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone())),
            );
            lists.push(("+".into(), "New playlist…".into()));
            bar = bar.child(widgets::picker(
                l,
                s,
                layer,
                "add-to-playlist",
                &lists,
                "",
                132.0,
                move |s, id, _| {
                    if id == "+" {
                        let mut edit = TextEdit::new("");
                        edit.focused = true;
                        s.browse.new_playlist = Some(edit);
                    } else if !id.is_empty() {
                        add_to_playlist(s, id, &picked(s));
                    }
                },
            ));
        }
        Some(edit) => {
            let create = move |s: &mut SettingsUi| {
                let Some(edit) = s.browse.new_playlist.take() else {
                    return;
                };
                let name = edit.text.trim().to_string();
                let ids = picked(s);
                if !name.is_empty() && !ids.is_empty() {
                    create_playlist(s, &name, ids);
                }
            };
            bar = bar
                .child(widgets::field(
                    l,
                    edit,
                    "Playlist name",
                    Some(170.0),
                    |s: &mut SettingsUi| s.browse.new_playlist.as_mut().expect("naming a playlist"),
                    move |s, _| create(s),
                ))
                .child(widgets::button_if(
                    l,
                    "Create",
                    Kind::Normal,
                    any && !edit.text.trim().is_empty(),
                    move |s, _| create(s),
                ));
        }
    }
    let tones = options(&[
        ("", "Light / dark…"),
        ("light", "Treat as light"),
        ("dark", "Treat as dark"),
        ("both", "Fine in both"),
        ("auto", "Decide by brightness"),
    ]);
    let bar = bar
        .child(widgets::button_if(
            l,
            "★ Favorite",
            Kind::Normal,
            any,
            move |s, _| {
                let mut all = s.snap.settings.favorites.clone();
                all.extend(
                    picked(s)
                        .into_iter()
                        .filter(|id| !s.snap.settings.favorites.contains(id)),
                );
                s.cx.patch(json!({ "favorites": all }));
            },
        ))
        .child(widgets::picker(
            l,
            s,
            layer,
            "bulk-tone",
            &tones,
            "",
            114.0,
            move |s, id, _| {
                if !id.is_empty() {
                    set_tone(s, &picked(s), (id != "auto").then_some(id));
                }
            },
        ))
        .child(widgets::button_if(
            l,
            "Never Show",
            Kind::Normal,
            any,
            move |s, _| {
                let mut all = s.snap.settings.excluded.clone();
                all.extend(
                    picked(s)
                        .into_iter()
                        .filter(|id| !s.snap.settings.excluded.contains(id)),
                );
                s.cx.patch(json!({ "excluded": all }));
            },
        ))
        .child(
            widgets::button_if(l, "Delete…", Kind::Danger, any, |s, _| {
                s.browse.confirm_delete = true;
            })
            .key("bulk-delete"),
        );
    if !s.browse.confirm_delete || !any {
        return bar;
    }
    let what = widgets::plural(n, "painting", "paintings");
    let button_label = format!("Delete {}", widgets::plural(n, "Painting", "Paintings"));
    col()
        .w_full()
        .gap(8.0)
        .child(bar)
        .child(
            row()
                .key("bulk-confirm")
                .w_full()
                .px(12.0)
                .py(10.0)
                .gap(12.0)
                .center_y()
                .flex_wrap()
                .radius(l.kit.radius.lg)
                .fill(l.danger_wash)
                .border(l.hair, l.danger.alpha(0.45))
                .child(
                    text(format!(
                        "Delete {what} from your art folder? The image files and their manifest entries will be permanently removed."
                    ))
                    .font(widgets::small())
                    .color(l.text)
                    .wrap()
                    .grow(1.0)
                    .min_w(200.0),
                )
                .child(
                    widgets::button(l, button_label, Kind::Danger, move |s, _| {
                        let ids = picked(s);
                        if s.browse
                            .selected
                            .as_ref()
                            .is_some_and(|sel| ids.contains(sel))
                        {
                            s.browse.selected = None;
                        }
                        stop_picking(s);
                        s.cx.run(move |app| {
                            if let Err(e) = actions::delete_paintings(app, &ids) {
                                log::warn!("delete paintings: {e}");
                            }
                        });
                    })
                    .key("bulk-confirm-yes"),
                )
                .child(
                    widgets::button(l, "Cancel", Kind::Normal, |s, _| {
                        s.browse.confirm_delete = false;
                    })
                    .key("bulk-confirm-no"),
                ),
        )
}

fn painting_tile(s: &SettingsUi, l: &Look, p: &PaintingView, width: f32, on_desktop: bool) -> El {
    let st = &s.browse;
    let id = p.id.clone();
    let open = id.clone();
    let picture = s.cx.art.thumb(&p.id, p.bytes, 420);
    tile(
        l,
        format!("tile-{}", p.id),
        &picture,
        width,
        TileLook {
            title: p.title.clone(),
            sub: [p.artist.clone(), p.date.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", "),
            faded: p.excluded || !p.museum_enabled,
            selected: !st.picking && st.selected.as_deref() == Some(p.id.as_str()),
            tick: st.picking.then(|| st.picked.contains(&p.id)),
            on_tick: None,
            badge: on_desktop.then(|| ("On desktop".to_string(), widgets::Tone::Gold)),
            star: p.favorite,
            corner: None,
            size: None,
            tone: p.tone,
        },
    )
    .on_press(move |s: &mut SettingsUi| {
        let b = &mut s.browse;
        if b.picking {
            if !b.picked.remove(&id) {
                b.picked.insert(id.clone());
            }
        } else {
            b.selected = (b.selected.as_deref() != Some(id.as_str())).then(|| id.clone());
        }
    })
    .on_double_press(move |s: &mut SettingsUi| {
        if !s.browse.picking {
            let id = open.clone();
            s.cx.run(move |app| {
                if let Some(shell) = app.shell() {
                    shell.open_viewer(Some(id), None);
                }
            });
        }
    })
}

pub fn page(s: &SettingsUi, l: &Look, layer: &mut Vec<El>, cx: &mut Cx<SettingsUi>) -> El {
    let snap = &s.snap;
    let st = &s.browse;
    let Some(lib) = s.library.clone() else {
        return col()
            .key("browse")
            .w_full()
            .h_full()
            .center()
            .child(widgets::muted(l, "Loading the library…").w(200.0));
    };
    let list = Rc::new(filtered(st, &lib, &snap.settings.playlists));
    let shown_ids: Rc<Vec<String>> =
        Rc::new(list.iter().map(|i| lib.paintings[*i].id.clone()).collect());

    // The bar across the top: search, filters, order.
    let mut museums: Vec<String> = lib
        .paintings
        .iter()
        .filter_map(|p| p.museum.clone())
        .collect();
    museums.sort();
    museums.dedup();
    let mut genre_options = vec![(String::new(), "All genres".to_string())];
    genre_options.extend(
        snap.genres
            .iter()
            .map(|g| (g.id.clone(), format!("{} ({})", g.name, g.count))),
    );
    let mut museum_options = vec![(String::new(), "All museums".to_string())];
    museum_options.extend(museums.into_iter().map(|m| (m.clone(), m)));
    let mut bar = row()
        .key("toolbar")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(
            el().grow(1.0).min_w(180.0).child(
                rune_kit::TextInput::new(&st.query)
                    .placeholder(format!("Search {} paintings…", lib.paintings.len()))
                    .label("Search paintings")
                    .leading_icon(&icons::SEARCH)
                    .clear_button()
                    .on_action(|s: &mut SettingsUi, _| s.browse.query = TextEdit::new(""))
                    .on_edit(|s: &mut SettingsUi| &mut s.browse.query)
                    .build(&l.kit)
                    .w_full(),
            ),
        )
        .child(widgets::picker(
            l,
            s,
            layer,
            "filter",
            &options(&FILTERS),
            &st.filter,
            124.0,
            |s, id, _: &mut Tx| s.browse.filter = id.into(),
        ))
        .child(widgets::picker(
            l,
            s,
            layer,
            "genre",
            &genre_options,
            &st.genre,
            136.0,
            |s, id, _| s.browse.genre = id.into(),
        ))
        .child(widgets::picker(
            l,
            s,
            layer,
            "museum",
            &museum_options,
            &st.museum,
            150.0,
            |s, id, _| s.browse.museum = id.into(),
        ));
    if !snap.settings.playlists.is_empty() {
        let mut lists = vec![(String::new(), "All playlists".to_string())];
        lists.extend(
            snap.settings
                .playlists
                .iter()
                .map(|p| (p.id.clone(), p.name.clone())),
        );
        bar = bar.child(widgets::picker(
            l,
            s,
            layer,
            "playlist",
            &lists,
            &st.playlist,
            130.0,
            |s, id, _| s.browse.playlist = id.into(),
        ));
    }
    let picking = st.picking;
    bar = bar
        .child(widgets::picker(
            l,
            s,
            layer,
            "sort",
            &options(&SORTS),
            &st.sort,
            136.0,
            |s, id, _| s.browse.sort = id.into(),
        ))
        .child(
            text(list.len().to_string())
                .font(widgets::body().features("'tnum' 1"))
                .color(l.muted)
                .px(4.0),
        )
        .child(widgets::button(
            l,
            if picking { "Done" } else { "Select" },
            if picking { Kind::Primary } else { Kind::Normal },
            move |s, _| {
                if picking {
                    stop_picking(s);
                } else {
                    s.browse.picking = true;
                }
            },
        ));

    // The grid: only the rows in view are built, so only their pictures are decoded.
    let gap = 10.0;
    let room = cx
        .layout_size("browse-grid")
        .map(|[w, _]| w)
        .unwrap_or(820.0);
    let (per_row, width) = columns(room, gap);
    let row_h = (width * 0.7).round() + 8.0 + 6.0 + 33.0 + gap;
    let rows = list.len().div_ceil(per_row);
    let on_desktop: Rc<Vec<String>> = Rc::new(
        snap.displays
            .iter()
            .filter_map(|d| d.painting.as_ref().map(|p| p.id.clone()))
            .collect(),
    );
    let (look, library, indices) = (Rc::new(l.clone()), Arc::clone(&lib), list.clone());
    let grid = col()
        .key("browse-grid")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .key("rows")
                .w_full()
                .growth(GrowthSpec::default())
                .virtual_rows(
                    rows,
                    row_h,
                    move |_: &SettingsUi, i| format!("row-{per_row}-{i}").into(),
                    move |s: &SettingsUi, i| {
                        let mut line = row().w_full().h(row_h).gap(gap).items_start();
                        for index in indices.iter().skip(i * per_row).take(per_row) {
                            let p = &library.paintings[*index];
                            line = line.child(painting_tile(
                                s,
                                &look,
                                p,
                                width,
                                on_desktop.contains(&p.id),
                            ));
                        }
                        line
                    },
                ),
        );
    let mut main = col()
        .key("browse-main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .gap(12.0)
        .child(bar);
    if picking {
        main = main.child(bulk_bar(s, l, layer, &shown_ids));
    }
    main = if list.is_empty() {
        main.child(
            el().w_full()
                .grow(1.0)
                .center()
                .child(widgets::muted(l, "Nothing matches these filters.").w(220.0)),
        )
    } else {
        main.child(grid)
    };

    let mut page = row()
        .key("browse")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(20.0)
        .pt(2.0)
        .pb(14.0)
        .gap(18.0)
        .child(main);
    let open = st
        .selected
        .as_ref()
        .filter(|_| !picking)
        .and_then(|id| lib.paintings.iter().find(|p| &p.id == id));
    if let Some(p) = open {
        let picture = s.cx.art.thumb(&p.id, p.bytes, 640);
        page = page.child(
            col()
                .key("detail")
                .w(330.0)
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(
                    col()
                        .w_full()
                        .gap(14.0)
                        .pb(10.0)
                        .child(
                            el().w_full()
                                .child(
                                    el().w_full()
                                        .aspect_ratio((p.aspect as f32).clamp(0.5, 2.4))
                                        .radius(l.kit.radius.lg)
                                        .clip_rounded(l.kit.radius.lg)
                                        .fill(l.well)
                                        .pixels_fit(&picture, SurfaceFit::Fill),
                                )
                                .child(
                                    row()
                                        .key("close")
                                        .role("button")
                                        .label("Close details")
                                        .focusable(true)
                                        .pin(None, Some(8.0), Some(8.0), None)
                                        .size(26.0, 26.0)
                                        .center()
                                        .radius(13.0)
                                        .fill(Color::BLACK.alpha(0.55))
                                        .hover(Color::WHITE.alpha(0.15))
                                        .cursor(Cursor::Pointer)
                                        .on_press(|s: &mut SettingsUi| s.browse.selected = None)
                                        .child(icon(&icons::CLOSE, 12.0).color(Color::WHITE)),
                                ),
                        )
                        .child(details(s, l, p, None, true)),
                ),
        );
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> LibraryView {
        let p = |id: &str, title: &str, artist: &str, museum: &str, brightness: f64| {
            let mut p = PaintingView::sample();
            p.id = id.into();
            p.title = title.into();
            p.artist = Some(artist.into());
            p.museum = Some(museum.into());
            p.brightness = Some(brightness);
            p.tone = Some(if brightness >= 110.0 {
                Tone::Light
            } else {
                Tone::Dark
            });
            p
        };
        let mut wide = p("c", "Water Lilies", "Claude Monet", "Orangerie", 140.0);
        wide.aspect = 3.0;
        wide.genres = vec!["landscape".into()];
        let mut night = p("b", "The Night Watch", "Rembrandt", "Rijksmuseum", 40.0);
        night.favorite = true;
        LibraryView {
            paintings: vec![
                p("a", "Wheat Field", "Vincent van Gogh", "The Met", 180.0),
                night,
                wide,
            ],
            missing: vec![],
            invalid: vec![],
            duplicates: vec![],
            unindexed: vec![],
        }
    }

    fn ids(st: &State, lib: &LibraryView) -> Vec<String> {
        filtered(st, lib, &[])
            .into_iter()
            .map(|i| lib.paintings[i].id.clone())
            .collect()
    }

    #[test]
    fn search_filters_and_order() {
        let lib = library();
        let mut st = State::default();
        assert_eq!(ids(&st, &lib), ["a", "b", "c"], "manifest order");
        st.query = TextEdit::new("MONET");
        assert_eq!(
            ids(&st, &lib),
            ["c"],
            "search ignores case and reads artists"
        );
        st.query = TextEdit::new("");
        st.filter = "favorites".into();
        assert_eq!(ids(&st, &lib), ["b"]);
        st.filter = "panorama".into();
        assert_eq!(ids(&st, &lib), ["c"]);
        st.filter = "dark".into();
        assert_eq!(ids(&st, &lib), ["b"]);
        st.filter = "all".into();
        st.museum = "The Met".into();
        assert_eq!(ids(&st, &lib), ["a"]);
        st.museum.clear();
        st.genre = "landscape".into();
        assert_eq!(ids(&st, &lib), ["c"]);
        st.genre = crate::genres::OTHER.into();
        assert_eq!(ids(&st, &lib), ["a", "b"], "paintings with no genre");
        st.genre.clear();
        st.sort = "brightness".into();
        assert_eq!(ids(&st, &lib), ["b", "c", "a"], "dark to light");
        st.sort = "artist".into();
        assert_eq!(ids(&st, &lib), ["c", "b", "a"]);
        st.sort = "title".into();
        assert_eq!(ids(&st, &lib), ["b", "c", "a"]);
    }

    #[test]
    fn playlists_narrow_the_grid() {
        let lib = library();
        let lists = [crate::settings::Playlist {
            id: "pl-1".into(),
            name: "Evenings".into(),
            items: vec!["b".into(), "c".into()],
        }];
        let st = State {
            playlist: "pl-1".into(),
            ..State::default()
        };
        let got: Vec<&str> = filtered(&st, &lib, &lists)
            .into_iter()
            .map(|i| lib.paintings[i].id.as_str())
            .collect();
        assert_eq!(got, ["b", "c"]);
    }

    #[test]
    fn tiles_fill_the_room() {
        let (n, w) = columns(820.0, 10.0);
        assert_eq!(n, 4);
        // Four tiles and three gaps use the whole width (each tile is its picture plus 8).
        assert!((4.0 * (w + 8.0) + 30.0 - 820.0).abs() < 4.0, "{w}");
        assert_eq!(columns(560.0, 10.0).0, 3, "beside the details");
        assert_eq!(columns(100.0, 10.0).0, 1, "never fewer than one");
    }
}
