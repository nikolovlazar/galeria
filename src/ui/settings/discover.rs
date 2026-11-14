//! Discover: search a museum, see what may be downloaded and why, and download it.

use super::browse::{columns, tile, TileLook};
use super::widgets::{self, Kind, Tone};
use super::{SettingsUi, Tab};
use crate::actions::{self, Catalogue};
use crate::download::{self, Job, State as JobState};
use crate::host;
use crate::licensing::Decision;
use crate::sources::{Candidate, Page, Query};
use crate::ui::icons;
use crate::ui::theme::Look;
use rune::tok::Family;
use rune::{
    col, el, icon, row, text, Color, Cursor, Cx, El, ScrollSpec, SurfaceFit, TextEdit, TextStyle,
};
use rune_kit::Progress;
use serde_json::json;
use std::collections::BTreeSet;

pub struct State {
    /// The licensing rules, genres and local catalogue status.
    pub catalogue: Option<Catalogue>,
    pub source: String,
    /// Wikimedia Commons only: works held by this institution.
    pub museum: String,
    pub text: TextEdit,
    pub page: Option<Page>,
    pub items: Vec<Candidate>,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: BTreeSet<String>,
    /// The result whose details are open.
    pub focus: Option<String>,
    /// Downloading a source's catalogue.
    pub preparing: bool,
    /// Counts searches, so an answer to an old one is ignored.
    pub(super) request: u64,
    /// The first search has been started.
    pub(super) started: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            catalogue: None,
            source: "nga".into(),
            museum: String::new(),
            text: TextEdit::new(""),
            page: None,
            items: Vec::new(),
            loading: false,
            error: None,
            selected: BTreeSet::new(),
            focus: None,
            preparing: false,
            request: 0,
            started: false,
        }
    }
}

const SHORT: [(&str, &str); 6] = [
    ("nga", "National Gallery of Art"),
    ("cma", "Cleveland"),
    ("rijks", "Rijksmuseum"),
    ("met", "The Met"),
    ("aic", "Art Institute of Chicago"),
    ("commons", "Wikimedia Commons"),
];

fn active(j: &Job) -> bool {
    matches!(
        j.state,
        JobState::Queued | JobState::Checking | JobState::Downloading { .. } | JobState::Processing
    )
}

/// Download state of a candidate, if it is (or was) in the queue.
fn job_label(j: &Job) -> String {
    match &j.state {
        JobState::Queued => "Queued".into(),
        JobState::Checking => "Checking license…".into(),
        JobState::Downloading { received, total } => match total {
            Some(t) if *t > 0 => format!(
                "Downloading {}%",
                (*received as f64 / *t as f64 * 100.0).round()
            ),
            _ => format!("Downloading {}", widgets::bytes(*received)),
        },
        JobState::Processing => "Saving…".into(),
        JobState::Done { .. } => "Saved".into(),
        JobState::Failed { .. } => "Failed".into(),
        JobState::Skipped { .. } => "Skipped".into(),
    }
}

impl SettingsUi {
    fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    fn saved(&self, c: &Candidate) -> bool {
        c.in_library
            || self
                .job(&c.id)
                .is_some_and(|j| matches!(j.state, JobState::Done { .. }))
    }

    /// May be ticked for download: open, not already here, not already on its way.
    pub(super) fn selectable(&self, c: &Candidate) -> bool {
        c.decision.as_ref().is_some_and(|d| d.allowed)
            && !self.saved(c)
            && !self.job(&c.id).is_some_and(active)
    }

    /// The page was opened: load what it needs and run the first search.
    pub fn discover_opened(&mut self) {
        if self.discover.catalogue.is_none() {
            self.discover.catalogue = Some(actions::catalogue(self.cx.app.core()));
        }
        if !self.discover.started && self.snap.library.root.is_some() {
            self.search(None);
        }
    }

    /// Another page sends the user here: a source, and for Commons a collection.
    pub fn discover_from(&mut self, source: &str, museum: &str) {
        self.discover.source = source.to_string();
        self.discover.museum = museum.to_string();
        self.discover.started = false;
        self.go(Tab::Discover);
    }

    /// Search the chosen source for what is typed. `cursor` continues the last search.
    pub fn search(&mut self, cursor: Option<String>) {
        let d = &mut self.discover;
        d.started = true;
        d.request += 1;
        d.loading = true;
        d.error = None;
        if cursor.is_none() {
            d.items.clear();
            d.page = None;
            d.selected.clear();
            d.focus = None;
        }
        let dl = &self.snap.settings.download;
        let query = Query {
            source: d.source.clone(),
            text: d.text.text.clone(),
            cursor: cursor.clone(),
            museum: (d.source == "commons" && !d.museum.is_empty()).then(|| d.museum.clone()),
            landscape_only: dl.landscape_only,
            min_long_edge: dl.min_long_edge,
        };
        let (token, more) = (d.request, cursor.is_some());
        if self.immediate {
            // Headless renders stay off the network unless asked.
            if std::env::var_os("GALERIA_UI_NETWORK").is_some() {
                let result = host::block_on(actions::discover_search(&self.cx.app, &query));
                self.search_done(token, more, result);
            } else {
                self.search_done(token, more, Ok(Page::default()));
            }
            return;
        }
        let Some(handle) = self.handle.clone() else {
            return;
        };
        let app = self.cx.app.clone();
        host::spawn(async move {
            let result = actions::discover_search(&app, &query).await;
            handle.update(move |ui: &mut SettingsUi, _| ui.search_done(token, more, result));
        });
    }

    fn search_done(&mut self, token: u64, more: bool, result: Result<Page, String>) {
        let d = &mut self.discover;
        if token != d.request {
            return;
        }
        d.loading = false;
        match result {
            Ok(mut page) => {
                let found = std::mem::take(&mut page.items);
                if more {
                    for c in found {
                        if !d.items.iter().any(|x| x.id == c.id) {
                            d.items.push(c);
                        }
                    }
                } else {
                    d.items = found;
                }
                d.page = Some(page);
            }
            Err(e) => d.error = Some(e),
        }
        if !more {
            self.onboarding_results();
        }
    }

    /// Fetch (or refresh) the open dataset the source is searched from, then search again.
    fn prepare_catalog(&mut self) {
        let Some(handle) = self.handle.clone() else {
            return;
        };
        self.discover.preparing = true;
        self.discover.error = None;
        self.progress = None;
        let (app, source) = (self.cx.app.clone(), self.discover.source.clone());
        host::spawn(async move {
            let result = actions::discover_prepare(&app, &source).await;
            let catalogue = actions::catalogue(app.core());
            handle.update(move |ui: &mut SettingsUi, _| {
                ui.discover.preparing = false;
                ui.progress = None;
                ui.discover.catalogue = Some(catalogue);
                match result {
                    Ok(_) => ui.search(None),
                    Err(e) => ui.discover.error = Some(e),
                }
            });
        });
    }

    pub(super) fn download(&mut self, ids: Vec<String>) {
        let requests: Vec<download::Request> = self
            .discover
            .items
            .iter()
            .filter(|c| ids.contains(&c.id) && self.selectable(c))
            .map(|c| download::Request {
                source: c.source.clone(),
                locator: c.locator.clone(),
                id: c.id.clone(),
                title: c.title.clone(),
                artist: c.artist.clone(),
                thumb_url: c.thumb_url.clone(),
            })
            .collect();
        if requests.is_empty() {
            return;
        }
        for r in &requests {
            self.discover.selected.remove(&r.id);
        }
        self.cx.run(move |app| {
            actions::download_start(app, requests);
        });
    }
}

/// The verdict on one record, with the evidence it rests on.
fn license_box(l: &Look, d: &Decision, notes: &[String]) -> El {
    let (tone, wash, edge) = if d.allowed {
        (Tone::Good, l.ok_wash, l.ok.alpha(0.35))
    } else {
        (Tone::Bad, l.danger_wash, l.danger.alpha(0.35))
    };
    let mut c = col()
        .key("license")
        .w_full()
        .padding(14.0)
        .gap(8.0)
        .radius(l.kit.radius.lg)
        .fill(wash)
        .border(l.hair, edge)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(widgets::chip(l, d.license.clone(), tone))
                .child(
                    text(if d.allowed {
                        "Free to download"
                    } else {
                        "Not downloadable"
                    })
                    .font(widgets::strong())
                    .color(l.text),
                ),
        )
        .child(widgets::para(l, d.reason.clone()));
    for o in &d.obligations {
        c = c.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(text("•").font(widgets::body()).color(l.muted))
                .child(widgets::para(l, o.clone()).grow(1.0).min_w(0.0)),
        );
    }
    for n in notes {
        c = c.child(widgets::muted(l, n.clone()));
    }
    c = c.child(
        el().w_full()
            .px(10.0)
            .py(7.0)
            .radius(l.kit.radius.sm)
            .fill(l.text.alpha(0.06))
            .child(
                text(d.evidence.clone())
                    .font(
                        TextStyle::new(11.0, 400.0)
                            .family(Family::SystemMono)
                            .lh(16.0),
                    )
                    .color(l.text.alpha(0.8))
                    .wrap()
                    .w_full()
                    .selectable(true)
                    .help("The fields of the museum's own record this decision rests on"),
            ),
    );
    let mut links = row().gap(14.0);
    if let Some(url) = d.license_url.clone() {
        links = links.child(widgets::link(l, "License text ↗", move |_, _| {
            let _ = actions::open_url(&url);
        }));
    }
    if !d.policy_url.is_empty() {
        let url = d.policy_url.clone();
        let name = if d.program.is_empty() {
            "Policy".to_string()
        } else {
            d.program.clone()
        };
        links = links.child(widgets::link(l, format!("{name} ↗"), move |_, _| {
            let _ = actions::open_url(&url);
        }));
    }
    c.child(links)
}

fn candidate_detail(s: &SettingsUi, l: &Look, c: &Candidate) -> El {
    let job = s.job(&c.id);
    let saved = s.saved(c);
    let join = |a: &Option<String>, b: Option<String>, sep: &str| {
        [a.clone(), b]
            .into_iter()
            .flatten()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(sep)
    };
    let file = match (c.width, c.height) {
        (Some(w), Some(h)) => format!("{w}×{h} px · {:.1} MP", w as f64 * h as f64 / 1e6),
        _ => "Size is known once downloaded".into(),
    };
    let file = match c.download_bytes {
        Some(b) => format!("{file} · {} download", widgets::bytes(b)),
        None => file,
    };
    let facts = [
        (
            "Artist",
            join(
                &c.artist,
                c.artist_bio.as_ref().map(|b| format!("({b})")),
                " ",
            ),
        ),
        ("Date", c.date.clone().unwrap_or_default()),
        ("Medium", c.medium.clone().unwrap_or_default()),
        ("Dimensions", c.dimensions.clone().unwrap_or_default()),
        (
            "Collection",
            join(&Some(c.museum.clone()), c.credit_line.clone(), " · "),
        ),
        ("Accession", c.accession.clone().unwrap_or_default()),
        ("File", file),
    ];
    let mut body = col().key("candidate").w_full().gap(12.0);
    if let Some(url) = c.preview_url.as_ref().or(c.thumb_url.as_ref()) {
        let picture = s.cx.art.remote(url, 900);
        body = body.child(
            el().w_full()
                .child(
                    el().w_full()
                        .aspect_ratio((c.aspect.unwrap_or(1.4) as f32).clamp(0.5, 2.4))
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
                        .on_press(|s: &mut SettingsUi| s.discover.focus = None)
                        .child(icon(&icons::CLOSE, 12.0).color(Color::WHITE)),
                ),
        );
    }
    body = body.child(
        col()
            .gap(2.0)
            .child(
                text(c.title.clone())
                    .font(TextStyle::new(19.0, 650.0).lh(24.0).tracking(-0.2))
                    .color(l.text)
                    .wrap()
                    .w_full()
                    .selectable(true),
            )
            .children(c.title_original.as_ref().map(|t| {
                text(t.clone())
                    .font(widgets::body().italic())
                    .color(l.muted)
                    .wrap()
                    .w_full()
            })),
    );
    body = body.child(widgets::facts(l, &facts));
    if let Some(d) = &c.description {
        body = body.child(
            text(d.clone())
                .font(widgets::body().lh(20.0))
                .color(l.text.alpha(0.86))
                .wrap()
                .w_full()
                .selectable(true),
        );
    }
    if let Some(d) = &c.decision {
        body = body.child(license_box(l, d, &c.notes));
    }
    let label = if saved {
        "In Your Library".to_string()
    } else {
        match job {
            Some(j) if !matches!(j.state, JobState::Failed { .. } | JobState::Skipped { .. }) => {
                job_label(j)
            }
            _ => "Download".to_string(),
        }
    };
    let id = c.id.clone();
    let mut actions_row = row().gap(8.0).child(widgets::button_if(
        l,
        label,
        Kind::Primary,
        s.selectable(c),
        move |s, _| s.download(vec![id.clone()]),
    ));
    if let Some(url) = c.page_url.clone() {
        actions_row = actions_row.child(widgets::button(
            l,
            "Museum Page ↗",
            Kind::Normal,
            move |_, _| {
                let _ = actions::open_url(&url);
            },
        ));
    }
    body = body.child(actions_row);
    if let Some(Job {
        state: JobState::Failed { error },
        ..
    }) = job
    {
        body = body.child(widgets::notice(l, error.clone(), true));
    }
    body
}

fn downloads(s: &SettingsUi, l: &Look) -> El {
    let running = s.jobs.iter().filter(|j| active(j)).count();
    let finished = s.jobs.len() - running;
    let mut head = row().w_full().gap(12.0).center_y().child(
        text("Downloads")
            .font(TextStyle::new(15.0, 650.0))
            .color(l.text)
            .grow(1.0),
    );
    if running > 0 {
        head = head.child(widgets::link(l, "Cancel all", |s, _| {
            s.cx.run(|app| actions::download_cancel(app, None))
        }));
    }
    if finished > 0 {
        head = head.child(widgets::link(l, "Clear finished", |s, _| {
            s.cx.run(actions::download_clear)
        }));
    }
    let mut list = col().key("downloads").w_full().gap(8.0).child(head);
    for j in &s.jobs {
        let says = match &j.state {
            JobState::Failed { error } => error.clone(),
            JobState::Skipped { reason } => reason.clone(),
            JobState::Done {
                width,
                height,
                bytes,
                license,
                ..
            } => {
                format!(
                    "Saved · {width}×{height} · {} · {license}",
                    widgets::bytes(*bytes)
                )
            }
            _ => job_label(j),
        };
        let bad = matches!(j.state, JobState::Failed { .. });
        let mut words = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(2.0)
            .child(
                text(j.title.clone())
                    .font(widgets::strong())
                    .color(l.text)
                    .truncate(),
            )
            .child(
                text(says)
                    .font(widgets::small())
                    .color(if bad { l.danger } else { l.muted })
                    .wrap()
                    .w_full(),
            );
        if let JobState::Downloading { received, total } = &j.state {
            let value = total
                .filter(|t| *t > 0)
                .map(|t| (*received as f32 / t as f32).clamp(0.0, 1.0));
            words = words.child(
                el().w_full()
                    .pt(2.0)
                    .child(Progress::new(value).build(&l.kit)),
            );
        }
        let mut line = row()
            .key(format!("job-{}", j.id))
            .w_full()
            .gap(10.0)
            .items_start();
        line = match &j.thumb_url {
            Some(url) => line.child(
                el().size(44.0, 34.0)
                    .radius(4.0)
                    .clip_rounded(4.0)
                    .fill(l.well)
                    .pixels_fit(&s.cx.art.remote(url, 420), SurfaceFit::Fill),
            ),
            None => line.child(el().size(44.0, 34.0).radius(4.0).fill(l.well)),
        };
        line = line.child(words);
        if matches!(
            j.state,
            JobState::Queued | JobState::Downloading { .. } | JobState::Checking
        ) {
            let id = j.id.clone();
            line = line.child(
                row()
                    .role("button")
                    .label("Cancel")
                    .focusable(true)
                    .size(22.0, 22.0)
                    .center()
                    .radius(11.0)
                    .hover(l.kit.c.hover)
                    .cursor(Cursor::Pointer)
                    .help("Cancel")
                    .on_press(move |s: &mut SettingsUi| {
                        let id = id.clone();
                        s.cx.run(move |app| actions::download_cancel(app, Some(&id)));
                    })
                    .child(icon(&icons::CLOSE, 11.0).color(l.muted)),
            );
        }
        list = list.child(line);
    }
    widgets::card(l).child(widgets::block(list))
}

/// A tile for every result: its picture, licence and size, with a tick to choose it.
/// With `details`, pressing a tile also opens its details beside the grid.
pub(super) fn result_tiles(s: &SettingsUi, l: &Look, width: f32, details: bool) -> Vec<El> {
    let d = &s.discover;
    let mut tiles = Vec::new();
    for c in &d.items {
        let ok = c.decision.as_ref().is_some_and(|x| x.allowed);
        let job = s.job(&c.id);
        let badge = if s.saved(c) {
            Some(("In library".to_string(), Tone::Plain))
        } else {
            job.map(|j| {
                (
                    job_label(j),
                    if matches!(j.state, JobState::Failed { .. }) {
                        Tone::Bad
                    } else {
                        Tone::Gold
                    },
                )
            })
        };
        let picture = match &c.thumb_url {
            Some(url) => s.cx.art.remote(url, 420),
            None => rune::Pixels::new(),
        };
        let id = c.id.clone();
        let sub = [c.artist.clone(), c.date.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        tiles.push(
            tile(
                l,
                format!("cand-{}", c.id),
                &picture,
                width,
                TileLook {
                    title: c.title.clone(),
                    sub: if sub.is_empty() {
                        c.museum.clone()
                    } else {
                        sub
                    },
                    faded: !ok,
                    selected: details && d.focus.as_deref() == Some(c.id.as_str()),
                    tick: s.selectable(c).then(|| d.selected.contains(&c.id)),
                    on_tick: Some(std::rc::Rc::new({
                        let id = c.id.clone();
                        move |s: &mut SettingsUi| {
                            // Only the selection: the details stay as they are.
                            if !s.discover.selected.remove(&id) {
                                s.discover.selected.insert(id.clone());
                            }
                        }
                    })),
                    badge,
                    star: false,
                    corner: Some(match &c.decision {
                        Some(x) if x.allowed => (x.license.clone(), Tone::Good),
                        _ => ("Not open".to_string(), Tone::Bad),
                    }),
                    size: c.width.zip(c.height).map(|(w, h)| format!("{w}×{h}")),
                    tone: None,
                },
            )
            .on_press(move |s: &mut SettingsUi| {
                if details {
                    s.discover.focus = Some(id.clone());
                }
                let can = s
                    .discover
                    .items
                    .iter()
                    .find(|c| c.id == id)
                    .is_some_and(|c| s.selectable(c));
                if can && !s.discover.selected.remove(&id) {
                    s.discover.selected.insert(id.clone());
                }
            }),
        );
    }
    tiles
}

pub fn page(s: &SettingsUi, l: &Look, layer: &mut Vec<El>, cx: &mut Cx<SettingsUi>) -> El {
    let snap = &s.snap;
    let d = &s.discover;
    if snap.library.root.is_none() {
        return col()
            .key("discover")
            .w_full()
            .h_full()
            .center()
            .gap(8.0)
            .px(40.0)
            .child(text("Choose your art folder first").font(TextStyle::new(20.0, 650.0)).color(l.text))
            .child(
                el().max_w(460.0).child(
                    text("Downloads are saved next to your other paintings, so Galeria needs to know where they live.")
                        .font(widgets::body())
                        .color(l.muted)
                        .wrap()
                        .align(rune::Align::Center),
                ),
            )
            .child(el().pt(8.0).child(widgets::button(l, "Choose Folder…", Kind::Primary, |s, tx| s.choose_folder(tx))));
    }
    let dl = &snap.settings.download;
    let rules = d.catalogue.as_ref().map(|c| c.rules);
    let rule = rules.and_then(|r| r.sources.iter().find(|x| x.id == d.source));
    let catalog = d
        .catalogue
        .as_ref()
        .and_then(|c| c.catalogs.iter().find(|x| x.source == d.source));

    // Which museum.
    let sources = rules
        .map(|r| {
            r.sources
                .iter()
                .map(|x| (x.id.clone(), x.name.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tabs = sources
        .iter()
        .map(|(id, name)| {
            let short = SHORT
                .iter()
                .find(|(x, _)| x == id)
                .map(|(_, n)| n.to_string())
                .unwrap_or_else(|| name.clone());
            let pick = id.clone();
            widgets::choice(
                l,
                format!("source-{id}"),
                short,
                None,
                d.source == *id,
                move |s, _| {
                    if s.discover.source != pick {
                        s.discover.source = pick.clone();
                        s.search(None);
                    }
                },
            )
        })
        .collect();
    let mut top = col().w_full().gap(10.0).child(widgets::wrap(tabs));

    // What its programme allows.
    if let (Some(rule), Some(rules)) = (rule, rules) {
        let grants = match &rule.grants {
            Some(id) => rules
                .licenses
                .iter()
                .find(|x| &x.id == id)
                .map(|x| x.label.clone())
                .unwrap_or_else(|| id.clone()),
            None if rule.id == "commons" => "License checked per file".into(),
            None => "Rights checked per record".into(),
        };
        let first_sentence = rule
            .summary
            .split_inclusive(". ")
            .next()
            .unwrap_or(&rule.summary)
            .trim()
            .to_string();
        let policy = rule.policy_url.clone();
        let program = if rule.program.to_lowercase().ends_with("policy") {
            rule.program.clone()
        } else {
            format!("{} policy", rule.program)
        };
        top = top.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(widgets::chip(l, grants, Tone::Good))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(2.0)
                        .child(widgets::hint(l, first_sentence).help(rule.summary.clone()))
                        .child(
                            row()
                                .gap(14.0)
                                .child(widgets::link(l, format!("{program} ↗"), move |_, _| {
                                    let _ = actions::open_url(&policy);
                                }))
                                .child(widgets::link(l, "How Galeria decides", |s, _| {
                                    s.go(Tab::Sources)
                                })),
                        ),
                ),
        );
    }

    // What to look for.
    let placeholder = if d.source == "commons" {
        "Search Wikimedia Commons: an artist, a title, a place…".to_string()
    } else {
        format!(
            "Search {}: an artist, a title, a subject…",
            rule.map(|r| r.name.as_str()).unwrap_or("the museum")
        )
    };
    let mut bar = row()
        .key("search")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(
            el().grow(1.0).min_w(220.0).child(
                rune_kit::TextInput::new(&d.text)
                    .placeholder(placeholder)
                    .label("Search the museum")
                    .leading_icon(&icons::SEARCH)
                    .on_edit(|s: &mut SettingsUi| &mut s.discover.text)
                    .on_submit(|s: &mut SettingsUi, _| s.search(None))
                    .build(&l.kit)
                    .w_full(),
            ),
        )
        .child(widgets::button(l, "Search", Kind::Primary, |s, _| {
            s.search(None)
        }));
    if d.source == "commons" {
        let mut collections = vec![(String::new(), "Any collection".to_string())];
        collections.extend(
            snap.museums
                .iter()
                .filter(|m| !m.policy.direct && m.name != "Collection not recorded")
                .map(|m| (m.name.clone(), m.name.clone())),
        );
        bar = bar.child(widgets::picker(
            l,
            s,
            layer,
            "collection",
            &collections,
            &d.museum,
            200.0,
            |s, id, _| {
                s.discover.museum = id.into();
                s.search(None);
            },
        ));
    }
    let landscape = dl.landscape_only;
    let sizes: Vec<(String, String)> = [
        (0, "Any size"),
        (2500, "2,500 px +"),
        (3000, "3,000 px +"),
        (4000, "4,000 px +"),
        (5000, "5,000 px +"),
    ]
    .iter()
    .map(|(n, name)| (n.to_string(), name.to_string()))
    .collect();
    bar = bar
        .child(
            widgets::checkbox(l, landscape, "Landscape format", move |s| {
                s.cx.patch(json!({ "download": { "landscapeOnly": !landscape } }));
            })
            .help("Hide works that are taller than they are wide"),
        )
        .child(widgets::picker(
            l,
            s,
            layer,
            "min-size",
            &sizes,
            &dl.min_long_edge.to_string(),
            130.0,
            |s, id, _| {
                s.cx.patch(
                    json!({ "download": { "minLongEdge": id.parse::<u32>().unwrap_or(3000) } }),
                );
            },
        ));
    top = top.child(bar);
    if let Some(c) = &d.catalogue {
        let typed = d.text.text.trim().to_lowercase();
        let chips: Vec<El> = c
            .genres
            .iter()
            .map(|g| {
                let words = g.search.clone();
                widgets::choice(
                    l,
                    format!("genre-{}", g.id),
                    g.name.clone(),
                    None,
                    typed == g.search.to_lowercase(),
                    move |s, _| {
                        s.discover.text = TextEdit::new(words.clone());
                        s.search(None);
                    },
                )
            })
            .collect();
        top = top.child(
            el().w_full().child(
                row()
                    .w_full()
                    .gap(8.0)
                    .scroll_x()
                    .child(row().gap(8.0).children(chips)),
            ),
        );
    }

    // What was found.
    let needs_catalog = d.page.as_ref().is_some_and(|p| p.needs_catalog);
    let results: El = if needs_catalog {
        let mut card = col()
            .w_full()
            .gap(10.0)
            .child(text("Get the National Gallery's catalogue").font(TextStyle::new(15.0, 650.0)).color(l.text))
            .child(widgets::muted(
                l,
                format!(
                    "The National Gallery of Art has no search service. It publishes its whole catalogue as open data instead, including which images are open access. Galeria downloads it once ({}), keeps a small index of the open-access paintings and searches that on this computer.",
                    catalog.map(|c| c.size_hint).unwrap_or("about 35 MB")
                ),
            ));
        card = match (&s.progress, d.preparing) {
            (Some(p), true) => card.child(
                col()
                    .w_full()
                    .gap(6.0)
                    .child(
                        Progress::new(Some(
                            (p.received as f32 / p.total.max(1) as f32).clamp(0.0, 1.0),
                        ))
                        .build(&l.kit),
                    )
                    .child(widgets::hint(
                        l,
                        if p.total > 1 {
                            format!("{} · {}", p.stage, widgets::bytes(p.received))
                        } else {
                            p.stage.clone()
                        },
                    )),
            ),
            (_, preparing) => card.child(row().child(widgets::button_if(
                l,
                if preparing {
                    "Starting…"
                } else {
                    "Download Catalogue"
                },
                Kind::Primary,
                !preparing,
                |s, _| s.prepare_catalog(),
            ))),
        };
        el().w_full()
            .grow(1.0)
            .child(widgets::card(l).child(widgets::block(card)))
    } else {
        let status = if d.loading && d.items.is_empty() {
            "Searching…".to_string()
        } else if d.error.is_some() {
            String::new()
        } else if let Some(p) = &d.page {
            [
                Some(format!("{} shown", d.items.len())),
                p.total
                    .map(|t| format!("{} at the source", widgets::thousands(t))),
                (p.hidden > 0).then(|| {
                    format!(
                        "{} hidden by the format and size filters",
                        widgets::thousands(p.hidden as u64)
                    )
                }),
                catalog.filter(|c| c.ready).map(|c| {
                    format!(
                        "catalogue of {} open-access paintings",
                        widgets::thousands(c.count as u64)
                    )
                }),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
        } else {
            String::new()
        };
        let mut line = row().w_full().gap(12.0).center_y().child(
            text(status)
                .font(widgets::small())
                .color(l.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
        if catalog.is_some_and(|c| c.ready) {
            let label = match (&s.progress, d.preparing) {
                (Some(p), true) => format!("{}…", p.stage),
                (None, true) => "Refreshing…".to_string(),
                _ => "Refresh catalogue".to_string(),
            };
            line = line.child(widgets::link(l, label, |s, _| {
                if !s.discover.preparing {
                    s.prepare_catalog();
                }
            }));
        }
        let gap = 10.0;
        let room = cx
            .layout_size("discover-grid")
            .map(|[w, _]| w)
            .unwrap_or(560.0);
        let (per_row, width) = columns(room, gap);
        let tiles = result_tiles(s, l, width, true);
        let _ = per_row;
        let mut grid = col()
            .w_full()
            .gap(12.0)
            .child(row().w_full().flex_wrap().gaps(gap, gap).children(tiles));
        if !d.loading && d.page.is_some() && d.items.is_empty() && d.error.is_none() {
            let hidden = d.page.as_ref().map(|p| p.hidden).unwrap_or(0);
            grid = grid.child(widgets::block(widgets::muted(
                l,
                format!(
                    "Nothing matched{}. Try another word, or relax the filters.",
                    if hidden > 0 {
                        format!(
                            ": {} results were hidden by the format and size filters",
                            widgets::thousands(hidden as u64)
                        )
                    } else {
                        String::new()
                    }
                ),
            )));
        }
        if let Some(next) = d.page.as_ref().and_then(|p| p.next.clone()) {
            let loading = d.loading;
            grid = grid.child(
                row()
                    .w_full()
                    .justify_center()
                    .py(6.0)
                    .child(widgets::button_if(
                        l,
                        if loading { "Loading…" } else { "Load More" },
                        Kind::Normal,
                        !loading,
                        move |s, _| s.search(Some(next.clone())),
                    )),
            );
        }
        let mut found = col().w_full().grow(1.0).min_h(0.0).gap(8.0).child(line);
        if let Some(e) = &d.error {
            found = found.child(widgets::notice(l, e.clone(), true));
        }
        if let Some(n) = d.page.as_ref().and_then(|p| p.notice.clone()) {
            found = found.child(widgets::notice(l, n, false));
        }
        found.child(
            col()
                .key("discover-grid")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(grid),
        )
    };

    // What to do with it.
    let chosen: Vec<String> = d
        .items
        .iter()
        .filter(|c| d.selected.contains(&c.id) && s.selectable(c))
        .map(|c| c.id.clone())
        .collect();
    let running = s.jobs.iter().filter(|j| active(j)).count();
    let mut says = if chosen.is_empty() {
        "Click paintings to select them".to_string()
    } else {
        format!("{} selected", chosen.len())
    };
    if running > 0 {
        says.push_str(&format!(" · downloading {running}"));
    }
    let mut footer = row()
        .key("actions")
        .w_full()
        .gap(8.0)
        .center_y()
        .pt(10.0)
        .child(
            text(says)
                .font(widgets::body())
                .color(l.muted)
                .grow(1.0)
                .min_w(0.0)
                .truncate(),
        );
    if d.items.iter().any(|c| s.selectable(c)) {
        footer = footer.child(widgets::button(l, "Select All", Kind::Quiet, |s, _| {
            let all: BTreeSet<String> = s
                .discover
                .items
                .iter()
                .filter(|c| s.selectable(c))
                .map(|c| c.id.clone())
                .collect();
            s.discover.selected = all;
        }));
    }
    if !chosen.is_empty() {
        footer = footer.child(widgets::button(l, "Clear", Kind::Quiet, |s, _| {
            s.discover.selected.clear()
        }));
    }
    let n = chosen.len();
    footer = footer.child(widgets::button_if(
        l,
        if n > 1 {
            format!("Download {n} Paintings")
        } else {
            "Download".to_string()
        },
        Kind::Primary,
        n > 0,
        move |s, _| s.download(chosen.clone()),
    ));

    let main = col()
        .key("discover-main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .min_h(0.0)
        .gap(12.0)
        .child(top)
        .child(results)
        .child(el().w_full().h(l.hair).fill(l.separator))
        .child(footer);
    let mut page = row()
        .key("discover")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(20.0)
        .pt(2.0)
        .pb(14.0)
        .gap(18.0)
        .child(main);
    let focus = d
        .focus
        .as_ref()
        .and_then(|id| d.items.iter().find(|c| &c.id == id));
    if focus.is_some() || !s.jobs.is_empty() {
        let mut aside = col().w_full().gap(14.0).pb(10.0);
        if !s.jobs.is_empty() {
            aside = aside.child(downloads(s, l));
        }
        if let Some(c) = focus {
            aside = aside.child(candidate_detail(s, l, c));
        }
        page = page.child(
            col()
                .key("detail")
                .w(340.0)
                .h_full()
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(aside),
        );
    }
    page
}
