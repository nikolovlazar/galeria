//! The first run: what Galeria is, where the paintings will live, and the first
//! few on the wall. It takes over the settings window until it is finished or
//! skipped; after that the window is the app.
//!
//! 1. Welcome      where the paintings come from, and why they are free to use
//! 2. Desktop      what Galeria does with them on this computer
//! 3. Folder       choose (or create) the art folder; an empty manifest is started
//! 4. Paintings    pick a museum and a theme, download a first set
//! 5. Hanging      wait for the first one, then open the app

use super::browse::columns;
use super::discover::result_tiles;
use super::now::preview;
use super::widgets::{self, Kind, Tone};
use super::{app_icon, SettingsUi, Tab};
use crate::actions;
use crate::download::{Request, State as JobState};
use crate::sources::{Candidate, Query};
use crate::ui::icons;
use crate::ui::theme::{rgb, rgba, Look};
use rune::{
    col, el, icon, row, text, Align, Color, Cx, El, Family, FileDialogOptions, Icon, ScrollSpec,
    Shadow, TextEdit, TextStyle, Tx,
};
use rune_kit::Progress;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f32::consts::PI;
use std::path::PathBuf;

/// How many paintings are ticked for someone who just wants a good start.
const FIRST_SET: usize = 12;

/// Museums whose collections can be searched straight away. (The National
/// Gallery of Art needs its catalogue downloaded first, and Wikimedia Commons
/// wants a search of your own; both are in Discover.)
const MUSEUMS: [(&str, &str, &str); 4] = [
    ("cma", "Cleveland Museum of Art", "Cleveland"),
    ("rijks", "Rijksmuseum", "Amsterdam"),
    ("met", "The Metropolitan Museum of Art", "New York"),
    ("aic", "Art Institute of Chicago", "Chicago"),
];

const SERIF: &str = "Iowan Old Style, Palatino, Baskerville, Georgia";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Desktop,
    Folder,
    Paintings,
    Hanging,
}

impl Step {
    const ALL: [Step; 5] = [
        Step::Welcome,
        Step::Desktop,
        Step::Folder,
        Step::Paintings,
        Step::Hanging,
    ];
}

/// A museum and a theme: one page of results. (`"cma"`, `"landscape"`.)
pub type Shelf = (String, String);

pub struct State {
    pub step: Step,
    /// The folder picked on the Folder step. It becomes the art folder on Continue.
    pub folder: Option<PathBuf>,
    /// What Continue will find there, in a sentence.
    pub folder_note: String,
    pub error: Option<String>,
    /// Setting the folder up.
    pub working: bool,

    // What to download. It outlives the grid, which only ever shows one shelf:
    /// The theme on show (a genre id); the museum is `discover.source`.
    pub theme: String,
    /// Paintings ticked one by one, on shelves that are not taken whole.
    pub picked: BTreeMap<String, Request>,
    /// Shelves taken whole: every painting found for that theme at that museum.
    pub whole: BTreeSet<Shelf>,
    /// Paintings unticked on a shelf that is taken whole.
    pub unpicked: BTreeSet<String>,
    /// What each shelf held when it was last looked at.
    pub results: HashMap<Shelf, Vec<Candidate>>,
    /// The first dozen have been ticked once; after that the choice is the user's.
    pub started: bool,

    /// How many paintings were sent for.
    pub asked: usize,
    /// Whole shelves still being read from the museums after Download.
    pub gathering: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            step: Step::Welcome,
            folder: None,
            folder_note: String::new(),
            error: None,
            working: false,
            theme: String::new(),
            picked: BTreeMap::new(),
            whole: BTreeSet::new(),
            unpicked: BTreeSet::new(),
            results: HashMap::new(),
            started: false,
            asked: 0,
            gathering: 0,
        }
    }
}

/// How a museum or a theme stands: nothing of it chosen, some, or all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chosen {
    None,
    Some,
    All,
}

fn request_of(c: &Candidate) -> Request {
    Request {
        source: c.source.clone(),
        locator: c.locator.clone(),
        id: c.id.clone(),
        title: c.title.clone(),
        artist: c.artist.clone(),
        thumb_url: c.thumb_url.clone(),
    }
}

fn themes() -> &'static [crate::genres::Genre] {
    crate::genres::all()
}

/// `~/Pictures/Galeria`: where Galeria suggests keeping the paintings.
pub fn recommended_folder() -> Option<PathBuf> {
    Some(rune::dirs::home()?.join("Pictures").join("Galeria"))
}

/// The folder's name as a person would say it: `~/Pictures/Galeria`.
fn friendly(path: &std::path::Path) -> String {
    match rune::dirs::home().and_then(|h| path.strip_prefix(h).ok().map(|p| p.to_path_buf())) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// What is in `folder` now, so Continue holds no surprise.
fn describe(folder: &std::path::Path) -> String {
    if folder.join(crate::library::MANIFEST).is_file() {
        return "This folder already has a manifest.json. Galeria will show the paintings it lists and add your downloads to it.".into();
    }
    let empty = std::fs::read_dir(folder)
        .map(|mut d| d.next().is_none())
        .unwrap_or(true);
    if empty {
        "An empty folder. Galeria starts a small index in it (manifest.json) and saves your downloads next to it.".into()
    } else {
        "Galeria adds a small index (manifest.json) to this folder and saves your downloads here. Nothing already in it is changed.".into()
    }
}

impl SettingsUi {
    fn onboard(&mut self) -> Option<&mut State> {
        self.onboarding.as_mut()
    }

    fn onboard_go(&mut self, step: Step) {
        if let Some(o) = self.onboard() {
            o.step = step;
            o.error = None;
        }
    }

    /// Create `~/Pictures/Galeria` and pick it.
    fn onboard_use_recommended(&mut self) {
        let Some(path) = recommended_folder() else {
            if let Some(o) = self.onboard() {
                o.error = Some(
                    "Galeria could not find your home folder. Choose a folder instead.".into(),
                );
            }
            return;
        };
        let made = std::fs::create_dir_all(&path);
        let Some(o) = self.onboard() else { return };
        match made {
            Ok(()) => {
                o.folder_note = describe(&path);
                o.folder = Some(path);
                o.error = None;
            }
            Err(e) => o.error = Some(format!("Could not create {}: {e}", friendly(&path))),
        }
    }

    fn onboard_choose_folder(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Choose a folder for your paintings".into(),
            directory: rune::dirs::home().map(|h| h.join("Pictures")),
            ..Default::default()
        };
        tx.open_folder(options, |s: &mut SettingsUi, result, _| {
            let Some(o) = s.onboard() else { return };
            match result {
                Ok(Some(path)) => {
                    o.folder_note = describe(&path);
                    o.folder = Some(path);
                    o.error = None;
                }
                Ok(None) => {}
                Err(e) => o.error = Some(format!("The folder picker could not open: {e}")),
            }
        });
    }

    /// Continue from the Folder step: the folder becomes the art folder, with an
    /// empty manifest if it had none, and the first search starts.
    fn onboard_commit_folder(&mut self) {
        let Some(path) = self.onboarding.as_ref().and_then(|o| o.folder.clone()) else {
            return;
        };
        let handle = self.handle.clone().filter(|_| !self.immediate);
        let Some(handle) = handle else {
            // Tests and headless renders: on the spot.
            let result = actions::setup_library(&self.cx.app, path);
            self.onboard_folder_ready(result);
            return;
        };
        if let Some(o) = self.onboard() {
            o.working = true;
            o.error = None;
        }
        self.cx.run(move |app| {
            let result = actions::setup_library(app, path);
            handle.update(move |ui: &mut SettingsUi, _| ui.onboard_folder_ready(result));
        });
    }

    fn onboard_folder_ready(&mut self, result: Result<(), String>) {
        let Some(o) = self.onboard() else { return };
        o.working = false;
        match result {
            Ok(()) => self.onboard_paintings(),
            Err(e) => o.error = Some(e),
        }
    }

    /// Open the Paintings step on a museum that answers at once, looking for landscapes.
    fn onboard_paintings(&mut self) {
        self.onboard_go(Step::Paintings);
        if self.discover.catalogue.is_none() {
            self.discover.catalogue = Some(actions::catalogue(self.cx.app.core()));
        }
        let museum = MUSEUMS
            .iter()
            .map(|(id, _, _)| *id)
            .find(|id| *id == self.discover.source)
            .unwrap_or(MUSEUMS[0].0);
        let theme = self
            .onboarding
            .as_ref()
            .map(|o| o.theme.clone())
            .filter(|t| themes().iter().any(|g| &g.id == t))
            .or_else(|| {
                themes()
                    .iter()
                    .find(|g| g.id == "landscape")
                    .or(themes().first())
                    .map(|g| g.id.clone())
            })
            .unwrap_or_default();
        self.onboard_show(museum, &theme);
    }

    /// The shelf on show.
    pub(super) fn shelf(&self) -> Shelf {
        (
            self.discover.source.clone(),
            self.onboarding
                .as_ref()
                .map(|o| o.theme.clone())
                .unwrap_or_default(),
        )
    }

    /// Carry the ticks on the grid into what will be downloaded. The grid only
    /// shows one shelf, so this runs before it shows another.
    fn onboard_absorb(&mut self) {
        let shelf = self.shelf();
        let ticks: Vec<(Candidate, bool)> = self
            .discover
            .items
            .iter()
            .filter(|c| self.selectable(c))
            .map(|c| (c.clone(), self.discover.selected.contains(&c.id)))
            .collect();
        let Some(o) = self.onboard() else { return };
        let whole = o.whole.contains(&shelf);
        for (c, ticked) in ticks {
            if whole {
                o.picked.remove(&c.id);
                if ticked {
                    o.unpicked.remove(&c.id);
                } else {
                    o.unpicked.insert(c.id.clone());
                }
            } else if ticked {
                o.picked.insert(c.id.clone(), request_of(&c));
            } else {
                o.picked.remove(&c.id);
            }
        }
    }

    /// Tick the grid the way the shelf on show stands.
    fn onboard_retick(&mut self) {
        let shelf = self.shelf();
        let Some(o) = &self.onboarding else { return };
        let whole = o.whole.contains(&shelf);
        let ticked = self
            .discover
            .items
            .iter()
            .filter(|c| self.selectable(c))
            .filter(|c| {
                if whole {
                    !o.unpicked.contains(&c.id)
                } else {
                    o.picked.contains_key(&c.id)
                }
            })
            .map(|c| c.id.clone())
            .collect();
        self.discover.selected = ticked;
    }

    /// Show a museum's paintings on a theme. What was ticked elsewhere is kept.
    pub(super) fn onboard_show(&mut self, museum: &str, theme: &str) {
        self.onboard_absorb();
        let Some(genre) = themes().iter().find(|g| g.id == theme) else {
            return;
        };
        self.discover.source = museum.to_string();
        self.discover.text = TextEdit::new(genre.search.clone());
        self.discover.focus = None;
        let shelf: Shelf = (museum.to_string(), theme.to_string());
        let Some(o) = self.onboard() else { return };
        o.theme = theme.to_string();
        match o.results.get(&shelf).cloned() {
            // Seen before: no need to ask the museum again.
            Some(items) => {
                self.discover.request += 1; // An answer still on its way is for another shelf.
                self.discover.loading = false;
                self.discover.error = None;
                self.discover.items = items;
                self.onboard_retick();
            }
            None => self.search(None),
        }
    }

    /// A search came back: on the Paintings step, remember the shelf and tick it
    /// the way it stands (the very first one starts with a dozen ticked).
    pub(super) fn onboarding_results(&mut self) {
        if !self
            .onboarding
            .as_ref()
            .is_some_and(|o| o.step == Step::Paintings)
        {
            return;
        }
        let shelf = self.shelf();
        let failed = self.discover.error.is_some();
        let items = self.discover.items.clone();
        let first: Vec<String> = items
            .iter()
            .filter(|c| self.selectable(c))
            .take(FIRST_SET)
            .map(|c| c.id.clone())
            .collect();
        let Some(o) = self.onboard() else { return };
        if !failed {
            o.results.insert(shelf, items);
        }
        let fresh = !o.started && o.picked.is_empty() && o.whole.is_empty();
        if fresh && !first.is_empty() {
            o.started = true;
            self.discover.selected = first.into_iter().collect();
        } else {
            self.onboard_retick();
        }
    }

    /// Take a whole shelf, or put it back.
    pub(super) fn onboard_toggle_theme(&mut self, theme: &str) {
        self.onboard_absorb();
        let shelf: Shelf = (self.discover.source.clone(), theme.to_string());
        let Some(o) = self.onboard() else { return };
        o.started = true;
        let on = !o.whole.remove(&shelf);
        if on {
            o.whole.insert(shelf.clone());
        }
        // Either way the shelf starts clean: all of it, or none of it.
        for c in o.results.get(&shelf).cloned().unwrap_or_default() {
            o.unpicked.remove(&c.id);
            o.picked.remove(&c.id);
        }
        self.onboard_retick();
    }

    /// Take every theme at a museum, or none of them.
    pub(super) fn onboard_toggle_museum(&mut self, museum: &str) {
        self.onboard_absorb();
        let all = self.onboard_museum(museum) == Chosen::All;
        let Some(o) = self.onboard() else { return };
        o.started = true;
        for g in themes() {
            let shelf: Shelf = (museum.to_string(), g.id.clone());
            if all {
                o.whole.remove(&shelf);
            } else {
                o.whole.insert(shelf.clone());
            }
            for c in o.results.get(&shelf).cloned().unwrap_or_default() {
                o.unpicked.remove(&c.id);
            }
        }
        // Single paintings from this museum go with the rest.
        o.picked.retain(|_, r| r.source != museum);
        self.onboard_retick();
    }

    /// The ids chosen on `shelf`, or `None` when it is taken whole and has not been read yet.
    fn onboard_shelf_ids(&self, shelf: &Shelf) -> Option<Vec<String>> {
        let o = self.onboarding.as_ref()?;
        if *shelf == self.shelf() && !(self.discover.loading && self.discover.items.is_empty()) {
            // The shelf on show: the grid's own ticks are the latest word.
            return Some(
                self.discover
                    .items
                    .iter()
                    .filter(|c| self.discover.selected.contains(&c.id) && self.selectable(c))
                    .map(|c| c.id.clone())
                    .collect(),
            );
        }
        let whole = o.whole.contains(shelf);
        match o.results.get(shelf) {
            Some(items) => Some(
                items
                    .iter()
                    .filter(|c| self.selectable(c))
                    .filter(|c| {
                        if whole {
                            !o.unpicked.contains(&c.id)
                        } else {
                            o.picked.contains_key(&c.id)
                        }
                    })
                    .map(|c| c.id.clone())
                    .collect(),
            ),
            None if whole => None,
            None => Some(Vec::new()),
        }
    }

    /// How a theme stands at the museum on show.
    pub(super) fn onboard_theme(&self, theme: &str) -> Chosen {
        let shelf: Shelf = (self.discover.source.clone(), theme.to_string());
        let Some(o) = &self.onboarding else {
            return Chosen::None;
        };
        let whole = o.whole.contains(&shelf);
        match self.onboard_shelf_ids(&shelf) {
            None => Chosen::All,
            Some(ids) if ids.is_empty() => Chosen::None,
            Some(ids) => {
                let all = o.results.get(&shelf).is_some_and(|items| {
                    items.iter().filter(|c| self.selectable(c)).count() == ids.len()
                });
                if whole && all {
                    Chosen::All
                } else {
                    Chosen::Some
                }
            }
        }
    }

    /// How a museum stands: every theme taken, something chosen, or nothing.
    pub(super) fn onboard_museum(&self, museum: &str) -> Chosen {
        let Some(o) = &self.onboarding else {
            return Chosen::None;
        };
        let shelves = || {
            themes()
                .iter()
                .map(move |g| -> Shelf { (museum.to_string(), g.id.clone()) })
        };
        if shelves().all(|s| o.whole.contains(&s)) {
            return Chosen::All;
        }
        let any = shelves().any(|s| self.onboard_shelf_ids(&s).is_none_or(|ids| !ids.is_empty()));
        if any {
            Chosen::Some
        } else {
            Chosen::None
        }
    }

    /// How many paintings Download would send for: those known, and how many
    /// whole shelves have not been read yet (each holds up to a page of results).
    pub(super) fn onboard_tally(&self) -> (usize, usize) {
        let Some(o) = &self.onboarding else {
            return (0, 0);
        };
        let mut ids: BTreeSet<String> = BTreeSet::new();
        let mut unread = 0;
        let mut shelves: BTreeSet<Shelf> = o.whole.clone();
        shelves.extend(o.results.keys().cloned());
        shelves.insert(self.shelf());
        for shelf in &shelves {
            match self.onboard_shelf_ids(shelf) {
                Some(found) => ids.extend(found),
                None => unread += 1,
            }
        }
        (ids.len(), unread)
    }

    fn onboard_download(&mut self) {
        self.onboard_absorb();
        let Some(o) = &self.onboarding else { return };
        // What is already known: single paintings, and whole shelves that were looked at.
        let mut requests: BTreeMap<String, Request> = o.picked.clone();
        let mut unread: Vec<Shelf> = Vec::new();
        for shelf in &o.whole {
            match o.results.get(shelf) {
                Some(items) => {
                    for c in items {
                        if self.selectable(c) && !o.unpicked.contains(&c.id) {
                            requests.insert(c.id.clone(), request_of(c));
                        }
                    }
                }
                None => unread.push(shelf.clone()),
            }
        }
        if requests.is_empty() && unread.is_empty() {
            return;
        }
        let requests: Vec<Request> = requests.into_values().collect();
        let skip: BTreeSet<String> = o.unpicked.clone();
        let (asked, gathering) = (requests.len(), unread.len());
        if let Some(o) = self.onboard() {
            o.asked = asked;
            o.gathering = gathering;
            o.step = Step::Hanging;
            o.error = None;
        }
        self.discover.selected.clear();
        let dl = &self.snap.settings.download;
        let (landscape_only, min_long_edge) = (dl.landscape_only, dl.min_long_edge);
        let app = self.cx.app.clone();
        let handle = self.handle.clone().filter(|_| !self.immediate);
        self.cx.run(move |app| {
            actions::download_start(app, requests);
        });
        let Some(handle) = handle else {
            return; // Tests and headless renders do not go to the museums.
        };
        // The shelves nobody looked at: read each from its museum, then send for it.
        crate::host::spawn(async move {
            for (museum, theme) in unread {
                let text = themes()
                    .iter()
                    .find(|g| g.id == theme)
                    .map(|g| g.search.clone())
                    .unwrap_or_default();
                let query = Query {
                    source: museum,
                    text,
                    landscape_only,
                    min_long_edge,
                    ..Default::default()
                };
                let found: Vec<Request> = match actions::discover_search(&app, &query).await {
                    Ok(page) => page
                        .items
                        .iter()
                        .filter(|c| c.decision.as_ref().is_some_and(|d| d.allowed))
                        .filter(|c| !c.in_library && !skip.contains(&c.id))
                        .map(request_of)
                        .collect(),
                    Err(e) => {
                        log::warn!("first run: {} {}: {e}", query.source, query.text);
                        Vec::new()
                    }
                };
                let sent = actions::download_start(&app, found);
                handle.update(move |ui: &mut SettingsUi, _| {
                    if let Some(o) = ui.onboarding.as_mut() {
                        o.asked += sent;
                        o.gathering = o.gathering.saturating_sub(1);
                    }
                });
            }
        });
    }

    /// Leave the first run for the app itself.
    pub fn onboard_finish(&mut self) {
        self.onboarding = None;
        self.discover.focus = None;
        self.go(Tab::Now);
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn serif(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
        .family(Family::Named(SERIF))
        .lh((size * 1.2).round())
}

fn heading(l: &Look, title: &str, sub: &str) -> El {
    col()
        .w_full()
        .gap(8.0)
        .items_center()
        .child(
            text(title.to_string())
                .font(serif(30.0, 600.0))
                .color(l.text)
                .align(Align::Center),
        )
        .child(
            col().w_full().max_w(600.0).child(
                text(sub.to_string())
                    .font(TextStyle::new(14.5, 400.0).lh(21.0))
                    .color(l.muted)
                    .wrap()
                    .w_full()
                    .align(Align::Center),
            ),
        )
}

/// One thing Galeria does: a gilt roundel, a line and what it means.
fn point(l: &Look, glyph: &'static Icon, title: &str, body: &str) -> El {
    row()
        .w_full()
        .gap(14.0)
        .items_start()
        .child(
            row()
                .size(34.0, 34.0)
                .radius(17.0)
                .center()
                .fill(l.gold_wash)
                .border(1.0, l.gold_border)
                .child(icon(glyph, 17.0).color(l.gold))
                .a11y_hidden(),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(3.0)
                .child(
                    text(title.to_string())
                        .font(TextStyle::new(14.5, 650.0).lh(19.0))
                        .color(l.text),
                )
                .child(
                    text(body.to_string())
                        .font(TextStyle::new(13.0, 400.0).lh(19.0))
                        .color(l.muted)
                        .wrap()
                        .w_full(),
                ),
        )
}

fn welcome(l: &Look) -> El {
    col()
        .w_full()
        .max_w(640.0)
        .gap(26.0)
        .items_center()
        .child(el().size(92.0, 92.0).pixels(app_icon()).a11y_hidden())
        .child(heading(
            l,
            "Welcome to Galeria",
            "A museum on your desktop: paintings from the world's great collections, shown one at a time, each with its label.",
        ))
        .child(
            widgets::card(l).w_full().child(
                col()
                    .w_full()
                    .padding(20.0)
                    .gap(18.0)
                    .child(point(
                        l,
                        &icons::MUSEUM,
                        "Straight from the museums",
                        "Galeria searches the open collections of the National Gallery of Art, the Cleveland Museum of Art, the Rijksmuseum, The Met, the Art Institute of Chicago and Wikimedia Commons, and downloads the high-resolution scans you pick.",
                    ))
                    .child(point(
                        l,
                        &icons::CHECK,
                        "Only what is free to use",
                        "A painting is downloaded only when the museum's own record says it is in the public domain or under an open licence (CC0, Public Domain Mark, CC BY, CC BY-SA). The licence is checked again at every download and saved with the painting.",
                    ))
                    .child(point(
                        l,
                        &icons::LIBRARY,
                        "Kept on your computer",
                        "The paintings live in a folder you choose. There is no account, and nothing is uploaded.",
                    )),
            ),
        )
}

/// A small desktop with a painting on it, a label in the corner and Galeria in the menu bar.
fn mock_desktop(w: f32) -> El {
    let h = (w * 0.64).round();
    let shadow = |y: f32, blur: f32, a: f32| Shadow {
        x: 0.0,
        y,
        blur,
        spread: 0.0,
        color: Color::BLACK.alpha(a),
        inset: false,
    };
    let line = |s: &str, size: f32, weight: f32, ink: Color| {
        text(s.to_string())
            .font(
                TextStyle::new(size, weight)
                    .family(Family::Named(SERIF))
                    .lh((size * 1.3).round()),
            )
            .color(ink)
    };
    let plate = col()
        .absolute(w - 150.0, h - 78.0)
        .w(136.0)
        .padding(9.0)
        .gap(1.0)
        .radius(2.0)
        .linear_gradient(PI, [(0.0, rgb(0xfdfbf6)), (1.0, rgb(0xece6da))])
        .box_shadow(shadow(3.0, 8.0, 0.4))
        .child(
            text("View near the Coast, 1889")
                .font(
                    TextStyle::new(8.5, 500.0)
                        .family(Family::Named(SERIF))
                        .italic()
                        .lh(11.0),
                )
                .color(rgb(0x1d1b18)),
        )
        .child(line("A PAINTER", 6.5, 700.0, rgb(0x1d1b18)))
        .child(line("Oil on canvas", 6.5, 400.0, rgb(0x4a463f)))
        .child(line("A museum · Public domain", 6.0, 400.0, rgb(0x7a7468)));
    el().size(w, h)
        .radius(12.0)
        .clip_rounded(12.0)
        .a11y_hidden()
        // Sky over land: a stand-in for a painting (Galeria ships none).
        .linear_gradient(
            PI,
            [
                (0.0, rgb(0x8fb0c4)),
                (0.38, rgb(0xe7c9a0)),
                (0.58, rgb(0xe9a878)),
                (0.585, rgb(0x55684a)),
                (1.0, rgb(0x27321f)),
            ],
        )
        .child(
            el().absolute(w * 0.62, h * 0.33)
                .size(34.0, 34.0)
                .radius(17.0)
                .fill(rgb(0xfff1cf))
                .box_shadow(shadow(0.0, 26.0, 0.0))
                .box_shadow(Shadow {
                    x: 0.0,
                    y: 0.0,
                    blur: 30.0,
                    spread: 6.0,
                    color: rgba(0xffe2a8, 0.55),
                    inset: false,
                }),
        )
        .child(
            el().absolute(-w * 0.2, h * 0.5)
                .size(w * 0.8, h * 0.6)
                .radius(h)
                .fill(rgb(0x4a5c40)),
        )
        .child(
            el().absolute(w * 0.4, h * 0.56)
                .size(w * 0.9, h * 0.7)
                .radius(h)
                .fill(rgb(0x364630)),
        )
        // The menu bar, with Galeria's item in it.
        .child(
            row()
                .absolute(0.0, 0.0)
                .size(w, 15.0)
                .px(8.0)
                .gap(6.0)
                .center_y()
                .fill(Color::WHITE.alpha(0.28))
                .child(
                    el().size(5.0, 5.0)
                        .radius(3.0)
                        .fill(Color::BLACK.alpha(0.6)),
                )
                .child(
                    el().size(22.0, 3.0)
                        .radius(2.0)
                        .fill(Color::BLACK.alpha(0.45)),
                )
                .child(
                    el().size(14.0, 3.0)
                        .radius(2.0)
                        .fill(Color::BLACK.alpha(0.3)),
                )
                .child(el().grow(1.0))
                .child(icon(&icons::NOW, 10.0).color(Color::BLACK.alpha(0.75)))
                .child(
                    el().size(18.0, 3.0)
                        .radius(2.0)
                        .fill(Color::BLACK.alpha(0.35)),
                ),
        )
        .child(plate)
        .box_shadow(shadow(14.0, 34.0, 0.35))
}

fn desktop(l: &Look) -> El {
    let points = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(18.0)
        .child(point(
            l,
            &icons::WALLPAPER,
            "A painting as your wallpaper",
            "It hangs behind your desktop icons and changes every half hour, or as often as you like. Wide canvases drift slowly from edge to edge; others hang framed on a wall.",
        ))
        .child(point(
            l,
            &icons::PLATE,
            "A museum label in the corner",
            "Title, artist, date, medium and the collection it belongs to, in one of seven styles. Drag it anywhere, or hide it.",
        ))
        .child(point(
            l,
            &icons::ROTATION,
            "Lock screen and screen saver too",
            "Galeria can also set the system wallpaper, so the lock screen matches, and play your paintings as the screen saver. Both are in Settings.",
        ))
        .child(point(
            l,
            &icons::NOW,
            "It lives in the menu bar",
            "Next painting, a full-screen viewer to explore one up close, and every setting, from the picture icon at the top of the screen.",
        ));
    col()
        .w_full()
        .max_w(860.0)
        .gap(30.0)
        .items_center()
        .child(heading(
            l,
            "What it does on your desktop",
            "Galeria works quietly in the background once it has a few paintings.",
        ))
        .child(
            row()
                .w_full()
                .gap(34.0)
                .center_y()
                .child(points)
                .child(mock_desktop(360.0)),
        )
}

fn folder(s: &SettingsUi, l: &Look, o: &State) -> El {
    let recommended = recommended_folder();
    let is_recommended = o.folder.is_some() && o.folder == recommended;
    let mut rows = Vec::new();
    if let Some(path) = &recommended {
        rows.push(widgets::setting(
            l,
            "recommended",
            friendly(path),
            Some(if path.is_dir() {
                "Recommended. A Galeria folder in Pictures.".into()
            } else {
                "Recommended. Galeria creates this folder in Pictures.".into()
            }),
            Some(widgets::button(
                l,
                if is_recommended {
                    "Selected"
                } else {
                    "Use This Folder"
                },
                if o.folder.is_none() {
                    Kind::Primary
                } else {
                    Kind::Normal
                },
                |s, _| s.onboard_use_recommended(),
            )),
        ));
    }
    rows.push(widgets::setting(
        l,
        "other",
        "Somewhere else",
        Some("Any folder, including one that already has paintings and a manifest.json.".into()),
        Some(widgets::button(
            l,
            "Choose Folder…",
            Kind::Normal,
            |s, tx| s.onboard_choose_folder(tx),
        )),
    ));
    let mut card = widgets::card(l);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            card = card.child(el().w_full().h(l.hair).fill(l.separator));
        }
        card = card.child(r);
    }
    let mut c = col()
        .w_full()
        .max_w(640.0)
        .gap(22.0)
        .items_center()
        .child(heading(
            l,
            "Where should the paintings live?",
            "Galeria saves what you download in one folder, with a small index beside the images. It never touches anything outside it.",
        ))
        .child(card);
    if let Some(path) = &o.folder {
        c = c.child(
            row()
                .key("chosen")
                .w_full()
                .gap(12.0)
                .items_start()
                .padding(14.0)
                .radius(l.kit.radius.lg)
                .fill(l.gold_wash)
                .border(1.0, l.gold_border)
                .child(icon(&icons::CHECK, 16.0).color(l.gold))
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(3.0)
                        .child(
                            text(friendly(path))
                                .font(TextStyle::new(13.5, 650.0).lh(18.0))
                                .color(l.text)
                                .wrap()
                                .w_full(),
                        )
                        .child(
                            text(o.folder_note.clone())
                                .font(widgets::small())
                                .color(l.muted)
                                .wrap()
                                .w_full(),
                        ),
                ),
        );
    }
    if let Some(e) = &o.error {
        c = c.child(widgets::notice(l, e.clone(), true));
    }
    let _ = s;
    c
}

/// A box to tick: empty, a dash for "some of it", or a tick for all of it.
fn tick_box(
    l: &Look,
    key: String,
    name: &str,
    state: Chosen,
    f: impl Fn(&mut SettingsUi) + 'static,
) -> El {
    let on = state != Chosen::None;
    let mut b = row()
        .key(key)
        .role("checkbox")
        .label(name.to_string())
        .focusable(true)
        .size(18.0, 18.0)
        .radius(5.0)
        .center()
        .fill(if on { l.gold } else { Color::TRANSPARENT })
        .border(1.5, if on { l.gold } else { l.text.alpha(0.45) })
        .cursor(rune::Cursor::Pointer)
        .on_press(f);
    b = match state {
        Chosen::All => b
            .checked(true)
            .child(icon(&icons::CHECK, 12.0).color(l.on_gold)),
        Chosen::Some => b
            .checked_mixed()
            .child(el().size(8.0, 2.0).radius(1.0).fill(l.on_gold)),
        Chosen::None => b.checked(false),
    };
    b
}

fn museum_card(
    l: &Look,
    id: &'static str,
    name: &str,
    place: &str,
    grants: String,
    showing: bool,
    state: Chosen,
) -> El {
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(3.0)
        .child(
            text(name.to_string())
                .font(TextStyle::new(13.0, 650.0).lh(17.0))
                .color(l.text)
                .wrap()
                .w_full(),
        )
        .child(
            text(place.to_string())
                .font(widgets::small())
                .color(l.muted),
        )
        .child(el().pt(5.0).child(widgets::chip(l, grants, Tone::Good)));
    row()
        .key(format!("museum-{id}"))
        .role("radio")
        .label(name.to_string())
        .checked(showing)
        .focusable(true)
        .grow(1.0)
        .min_w(0.0)
        .padding(12.0)
        .gap(8.0)
        .items_start()
        .radius(l.kit.radius.lg)
        .fill(if showing { l.gold_wash } else { l.card })
        .border(
            1.0,
            if showing {
                l.gold_border
            } else {
                l.card_border
            },
        )
        .hover(l.kit.c.hover)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut SettingsUi| {
            if s.discover.source != id {
                let theme = s.shelf().1;
                s.onboard_show(id, &theme);
            }
        })
        .child(words)
        .child(tick_box(
            l,
            format!("all-{id}"),
            &format!("Everything from {name}"),
            state,
            move |s| s.onboard_toggle_museum(id),
        ))
}

/// A theme: press its name to look at it, tick its box to take all of it.
fn theme_chip(l: &Look, id: &str, name: &str, showing: bool, state: Chosen) -> El {
    let (show, take) = (id.to_string(), id.to_string());
    row()
        .key(format!("theme-{id}"))
        .role("radio")
        .label(name.to_string())
        .checked(showing)
        .focusable(true)
        .h(30.0)
        .pl(7.0)
        .pr(12.0)
        .gap(7.0)
        .center_y()
        .radius(15.0)
        .fill(if showing {
            l.gold_wash
        } else {
            Color::TRANSPARENT
        })
        .border(
            1.0,
            if showing {
                l.gold_border
            } else {
                l.card_border
            },
        )
        .hover(l.kit.c.hover)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut SettingsUi| {
            let museum = s.discover.source.clone();
            s.onboard_show(&museum, &show);
        })
        .child(tick_box(
            l,
            format!("all-theme-{id}"),
            &format!("Every painting under {name}"),
            state,
            move |s| s.onboard_toggle_theme(&take),
        ))
        .child(
            text(name.to_string())
                .font(TextStyle::new(12.5, 550.0).lh(16.0))
                .color(l.text),
        )
}

fn paintings(s: &SettingsUi, l: &Look, cx: &mut Cx<SettingsUi>) -> El {
    let d = &s.discover;
    let rules = d.catalogue.as_ref().map(|c| c.rules);
    let cards: Vec<El> = MUSEUMS
        .iter()
        .map(|(id, name, place)| {
            let grants = rules
                .and_then(|r| {
                    let rule = r.sources.iter().find(|x| x.id == *id)?;
                    let grant = rule.grants.as_ref()?;
                    Some(
                        r.licenses
                            .iter()
                            .find(|x| &x.id == grant)
                            .map(|x| x.label.clone())
                            .unwrap_or_else(|| grant.clone()),
                    )
                })
                .unwrap_or_else(|| "Public domain".into());
            museum_card(
                l,
                id,
                name,
                place,
                grants,
                d.source == *id,
                s.onboard_museum(id),
            )
        })
        .collect();
    let showing = s
        .onboarding
        .as_ref()
        .map(|o| o.theme.as_str())
        .unwrap_or("");
    let themes: Vec<El> = themes()
        .iter()
        .map(|g| theme_chip(l, &g.id, &g.name, g.id == showing, s.onboard_theme(&g.id)))
        .collect();
    let here = d
        .items
        .iter()
        .filter(|c| d.selected.contains(&c.id) && s.selectable(c))
        .count();
    let (known, unread) = s.onboard_tally();
    let total = match (known, unread) {
        (0, 0) => "Nothing ticked yet.".to_string(),
        (n, 0) => format!(
            "{} to download in all.",
            widgets::plural(n, "painting", "paintings")
        ),
        (n, u) => format!(
            "{} to download, plus {} not yet looked at (up to {} paintings each).",
            widgets::plural(n, "painting", "paintings"),
            widgets::plural(u, "theme", "themes"),
            crate::sources::PAGE_SIZE
        ),
    };
    let status = if d.loading && d.items.is_empty() {
        format!("Asking the museum… {total}")
    } else if let Some(e) = &d.error {
        e.clone()
    } else if d.items.is_empty() {
        format!("Nothing came back for this theme here. {total}")
    } else {
        format!("{here} of {} ticked here. {total}", d.items.len())
    };
    let gap = 10.0;
    let room = cx
        .layout_size("onboard-grid")
        .map(|[w, _]| w)
        .unwrap_or(820.0);
    let (_, width) = columns(room, gap);
    let tiles = result_tiles(s, l, width, false);
    col()
        .w_full()
        .max_w(900.0)
        .h_full()
        .min_h(0.0)
        .gap(14.0)
        .child(heading(
            l,
            "Hang your first paintings",
            "Look through the museums and themes and tick what you like. A box on a museum or a theme takes all of it. One painting goes straight on the wall.",
        ))
        .child(row().w_full().gap(10.0).children(cards))
        .child(
            el().w_full().child(
                row()
                    .w_full()
                    .gap(8.0)
                    .scroll_x()
                    .child(row().gap(8.0).children(themes)),
            ),
        )
        .child(
            text(status)
                .font(widgets::small())
                .color(if d.error.is_some() { l.danger } else { l.muted })
                .wrap()
                .w_full(),
        )
        .child(
            col()
                .key("onboard-grid")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(row().w_full().flex_wrap().gaps(gap, gap).children(tiles)),
        )
}

fn hanging(s: &SettingsUi, l: &Look, o: &State) -> El {
    let done = |j: &&crate::download::Job| matches!(j.state, JobState::Done { .. });
    let failed = |j: &&crate::download::Job| matches!(j.state, JobState::Failed { .. });
    let saved = s.jobs.iter().filter(done).count();
    let failures = s.jobs.iter().filter(failed).count();
    let running = s.jobs.iter().find(|j| {
        matches!(
            j.state,
            JobState::Checking | JobState::Downloading { .. } | JobState::Processing
        )
    });
    let total = o.asked.max(saved);
    if let Some(p) = s.sample() {
        let ps = &s.snap.settings.plate;
        let aspect = (p.width as f32 / p.height.max(1) as f32).clamp(0.5, 2.6);
        let rest = if o.gathering > 0 {
            format!(
                "{saved} saved so far. Galeria is still reading {} from the museums.",
                widgets::plural(o.gathering, "theme", "themes")
            )
        } else if saved < total && running.is_some() {
            format!("{saved} of {total} paintings saved; the rest are still arriving.")
        } else {
            match saved {
                0 => String::new(),
                n => format!(
                    "{} saved in your folder.",
                    widgets::plural(n, "painting", "paintings")
                ),
            }
        };
        return col()
            .w_full()
            .max_w(640.0)
            .gap(22.0)
            .items_center()
            .child(heading(
                l,
                "Your first painting is on the wall",
                "Look behind this window: it is on your desktop now. More are in Discover whenever you want them.",
            ))
            .child(preview(s, l, p, ps, 460.0, aspect, 0.5, ps.default_position).key("first"))
            .child(text(rest).font(widgets::small()).color(l.muted));
    }
    let mut c = col()
        .w_full()
        .max_w(520.0)
        .gap(18.0)
        .items_center()
        .child(heading(
            l,
            "Hanging your first painting…",
            "Galeria reads the museum's record again, checks the licence, then downloads the scan. Large scans take a moment.",
        ));
    let nothing_left = running.is_none()
        && o.gathering == 0
        && !s.jobs.iter().any(|j| matches!(j.state, JobState::Queued))
        && saved == 0;
    if nothing_left && failures > 0 {
        let why = s
            .jobs
            .iter()
            .find_map(|j| match &j.state {
                JobState::Failed { error } => Some(error.clone()),
                _ => None,
            })
            .unwrap_or_default();
        c = c.child(widgets::notice(
            l,
            format!("None of the paintings could be downloaded. {why}"),
            true,
        ));
    } else {
        let (value, label) = match running.map(|j| (&j.state, j.title.clone())) {
            Some((
                JobState::Downloading {
                    received,
                    total: Some(t),
                },
                title,
            )) if *t > 0 => (
                Some((*received as f32 / *t as f32).clamp(0.0, 1.0)),
                format!("{title} · {}", widgets::bytes(*received)),
            ),
            Some((JobState::Checking, title)) => (None, format!("{title} · checking the licence")),
            Some((JobState::Processing, title)) => (None, format!("{title} · saving")),
            Some((_, title)) => (None, title),
            None => (None, "Waiting for the museum…".to_string()),
        };
        c = c
            .child(el().w_full().child(Progress::new(value).build(&l.kit)))
            .child(
                text(label)
                    .font(widgets::small())
                    .color(l.muted)
                    .truncate()
                    .max_w(520.0),
            );
        if saved > 0 {
            c = c.child(
                text(format!(
                    "{saved} of {total} saved. Putting the first one up…"
                ))
                .font(widgets::small())
                .color(l.muted),
            );
        }
    }
    c
}

fn dots(l: &Look, step: Step) -> El {
    row()
        .gap(7.0)
        .center_y()
        .a11y_hidden()
        .children(Step::ALL.iter().map(|x| {
            let on = *x == step;
            el().size(if on { 18.0 } else { 6.0 }, 6.0)
                .radius(3.0)
                .fill(if on { l.gold } else { l.text.alpha(0.18) })
        }))
}

fn footer(s: &SettingsUi, l: &Look, o: &State) -> El {
    let back = |to: Step| {
        widgets::button(l, "Back", Kind::Quiet, move |s, _| s.onboard_go(to)).key("back")
    };
    let (left, right): (El, Vec<El>) = match o.step {
        Step::Welcome => (
            el(),
            vec![widgets::button(l, "Continue", Kind::Primary, |s, _| {
                s.onboard_go(Step::Desktop)
            })
            .key("next")],
        ),
        Step::Desktop => (
            back(Step::Welcome),
            vec![widgets::button(l, "Continue", Kind::Primary, |s, _| {
                s.onboard_go(Step::Folder)
            })
            .key("next")],
        ),
        Step::Folder => (
            back(Step::Desktop),
            vec![widgets::button_if(
                l,
                if o.working {
                    "Setting Up…"
                } else {
                    "Continue"
                },
                Kind::Primary,
                o.folder.is_some() && !o.working,
                |s, _| s.onboard_commit_folder(),
            )
            .key("next")],
        ),
        Step::Paintings => {
            let (known, unread) = s.onboard_tally();
            let n = known + unread * crate::sources::PAGE_SIZE;
            let label = match (known, unread) {
                (0, 0) => "Download".to_string(),
                (n, 0) => format!("Download {}", widgets::plural(n, "Painting", "Paintings")),
                (_, _) => format!("Download up to {n} Paintings"),
            };
            (
                back(Step::Folder),
                vec![
                    widgets::button(l, "Skip for Now", Kind::Quiet, |s, _| s.onboard_finish())
                        .key("skip"),
                    widgets::button_if(l, label, Kind::Primary, n > 0, |s, _| s.onboard_download())
                        .key("next"),
                ],
            )
        }
        Step::Hanging => {
            let hung = s.sample().is_some();
            let stuck = !hung
                && o.gathering == 0
                && !s.jobs.iter().any(|j| {
                    matches!(
                        j.state,
                        JobState::Queued
                            | JobState::Checking
                            | JobState::Downloading { .. }
                            | JobState::Processing
                    )
                });
            (
                if stuck { back(Step::Paintings) } else { el() },
                vec![
                    widgets::button_if(l, "Open Galeria", Kind::Primary, hung, |s, _| {
                        s.onboard_finish()
                    })
                    .key("next"),
                ],
            )
        }
    };
    row()
        .key("onboard-footer")
        .w_full()
        .h(64.0)
        .px(28.0)
        .center_y()
        .child(row().w(260.0).child(left))
        .child(row().grow(1.0).center().child(dots(l, o.step)))
        .child(row().w(260.0).gap(8.0).justify_end().children(right))
}

/// The first run, filling the window.
pub fn view(s: &SettingsUi, l: &Look, cx: &mut Cx<SettingsUi>) -> El {
    let Some(o) = &s.onboarding else {
        return el();
    };
    let body = match o.step {
        Step::Welcome => welcome(l),
        Step::Desktop => desktop(l),
        Step::Folder => folder(s, l, o),
        Step::Paintings => paintings(s, l, cx),
        Step::Hanging => hanging(s, l, o),
    };
    // The grid of results fills the height; the other steps sit in the middle.
    let stage = col()
        .key("onboard-stage")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(40.0)
        .items_center();
    let stage = if o.step == Step::Paintings {
        stage.pt(4.0).child(body)
    } else {
        stage.center().child(body)
    };
    col()
        .key("onboarding")
        .w_full()
        .h_full()
        .fill(l.bg)
        // The title bar: empty, and the window drags by it.
        .child(el().w_full().h(44.0).window_drag())
        .child(stage)
        .child(el().w_full().h(l.hair).fill(l.separator))
        .child(footer(s, l, o))
}
