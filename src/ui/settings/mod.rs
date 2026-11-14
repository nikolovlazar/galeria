//! The settings window: a sidebar of pages beside the page itself.
//!
//! Every page draws from the core's [`Snapshot`]. Controls change settings
//! through [`Ctx::patch`]; the snapshot that follows redraws the page.

mod browse;
mod discover;
mod general;
mod library;
mod now;
mod onboarding;
mod plate;
mod rotation;
mod sources;
mod wallpaper;
pub mod widgets;

use super::icons;
use super::theme::{self, Look};
use super::Ctx;
use crate::actions;
use crate::core::{LibraryView, Snapshot};
use crate::download::Job;
use crate::host::CatalogProgress;
use rune::{
    col, el, row, text, Cx, El, FileDialogOptions, Icon, Key, KeyEvent, Pixels, ScrollSpec,
    TextStyle, Tx,
};
use rune_kit::shell::{PaneState, Sidebar, SidebarRow, SplitView, Toolbar};
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Now,
    Browse,
    Discover,
    Sources,
    Rotation,
    Wallpaper,
    Plate,
    Library,
    General,
}

impl Tab {
    pub const ALL: [(Tab, &'static str, &'static str, &'static Icon); 9] = [
        (Tab::General, "general", "General", &icons::GENERAL),
        (Tab::Now, "now", "Now Showing", &icons::NOW),
        (Tab::Browse, "browse", "Browse", &icons::BROWSE),
        (Tab::Discover, "discover", "Discover", &icons::DISCOVER),
        (Tab::Sources, "sources", "Museums", &icons::MUSEUM),
        (Tab::Rotation, "rotation", "Rotation", &icons::ROTATION),
        (Tab::Wallpaper, "wallpaper", "Wallpaper", &icons::WALLPAPER),
        (Tab::Plate, "plate", "Info Plate", &icons::PLATE),
        (Tab::Library, "library", "Library", &icons::LIBRARY),
    ];

    /// The tab a `--settings <section>` argument names.
    pub fn named(id: &str) -> Option<Tab> {
        Tab::ALL
            .iter()
            .find(|(_, key, _, _)| *key == id)
            .map(|(t, _, _, _)| *t)
    }

    pub fn title(self) -> &'static str {
        Tab::ALL
            .iter()
            .find(|(t, _, _, _)| *t == self)
            .map(|(_, _, title, _)| *title)
            .unwrap_or("")
    }

    /// Pages that list every painting, not only what the snapshot carries.
    fn lists_library(self) -> bool {
        matches!(self, Tab::Browse | Tab::Rotation | Tab::Library)
    }
}

pub struct SettingsUi {
    pub cx: Ctx,
    pub snap: Arc<Snapshot>,
    /// Routes results of background work back to this window. `None` in tests and headless renders.
    pub(crate) handle: Option<rune::AppHandle<SettingsUi>>,
    /// Do background work on the spot (headless renders).
    immediate: bool,
    pub tab: Tab,
    pub sidebar: PaneState,
    /// Every painting, fetched when a page that lists them is open and something about them changed.
    pub library: Option<Arc<LibraryView>>,
    library_key: String,
    /// The slider being moved and its value, until the setting catches up.
    pub sliding: Option<(String, f64)>,
    pub slider_held: bool,
    /// The in-window menu that is open, where the system has no pop-up menus.
    pub open_select: Option<String>,
    pub jobs: Vec<Job>,
    pub progress: Option<CatalogProgress>,
    pub browse: browse::State,
    pub discover: discover::State,
    pub rotation: rotation::State,
    pub plate: plate::State,
    pub library_page: library::State,
    /// Painting ids and per-museum/genre counts currently in `manifest.json`.
    pub manifest_index: Arc<crate::core::ManifestIndex>,
    manifest_stamp: (u64, usize, Option<std::path::PathBuf>),
    /// Painting id awaiting inline delete confirmation in Now Showing / Browse detail.
    pub confirm_delete: Option<String>,
    /// Museum name awaiting inline delete confirmation in Museums.
    pub confirm_delete_museum: Option<String>,
    /// The first run, while it lasts: it takes the window over.
    pub onboarding: Option<onboarding::State>,
    pub clock: f64,
}

impl SettingsUi {
    pub fn new(cx: Ctx, snap: Arc<Snapshot>) -> SettingsUi {
        // No art folder yet: this is someone's first run.
        let onboarding = snap.library.root.is_none().then(onboarding::State::default);
        let manifest_index = Arc::new(cx.app.core().manifest_index());
        let manifest_stamp = (
            snap.library.loaded_at,
            snap.library.valid,
            snap.library.root.clone(),
        );
        SettingsUi {
            cx,
            snap,
            handle: None,
            immediate: false,
            onboarding,
            tab: Tab::General,
            sidebar: PaneState::new(216.0).range(180.0, 320.0),
            library: None,
            library_key: String::new(),
            sliding: None,
            slider_held: false,
            open_select: None,
            jobs: Vec::new(),
            progress: None,
            browse: browse::State::default(),
            discover: discover::State::default(),
            rotation: rotation::State::default(),
            plate: plate::State::default(),
            library_page: library::State::default(),
            manifest_index,
            manifest_stamp,
            confirm_delete: None,
            confirm_delete_museum: None,
            clock: 0.0,
        }
    }

    pub fn with_handle(mut self, handle: rune::AppHandle<SettingsUi>) -> SettingsUi {
        self.handle = Some(handle);
        self
    }

    pub fn immediate(mut self) -> SettingsUi {
        self.immediate = true;
        self.jobs = self.cx.app.core().downloads.list();
        self.refresh_library();
        self
    }

    /// A new snapshot arrived.
    pub fn apply(&mut self, snap: Arc<Snapshot>, _: &mut Tx) {
        let stamp = (
            snap.library.loaded_at,
            snap.library.valid,
            snap.library.root.clone(),
        );
        self.snap = snap;
        if stamp != self.manifest_stamp {
            self.manifest_stamp = stamp;
            self.manifest_index = Arc::new(self.cx.app.core().manifest_index());
        }
        if !self.slider_held {
            self.sliding = None;
        }
        self.refresh_library();
    }

    pub(super) fn sync_manifest_now(&mut self) {
        let snap = Arc::new(self.cx.app.core().snapshot());
        self.manifest_stamp = (
            snap.library.loaded_at,
            snap.library.valid,
            snap.library.root.clone(),
        );
        self.snap = snap;
        self.manifest_index = Arc::new(self.cx.app.core().manifest_index());
    }

    pub fn navigate(&mut self, section: Option<&str>, _: &mut Tx) {
        if self.onboarding.is_some() {
            return; // The first run comes first.
        }
        if let Some(tab) = section.and_then(Tab::named) {
            self.go(tab);
        }
    }

    pub fn go(&mut self, tab: Tab) {
        self.tab = tab;
        self.open_select = None;
        self.confirm_delete = None;
        self.confirm_delete_museum = None;
        self.refresh_library();
        if tab == Tab::Discover {
            self.discover_opened();
        }
    }

    /// Fetch the full painting list again when what it shows has changed.
    fn refresh_library(&mut self) {
        if !self.tab.lists_library() {
            return;
        }
        let s = &self.snap;
        let key = serde_json::json!([
            s.library.loaded_at,
            s.library.enriched,
            s.settings.excluded.len(),
            s.settings.favorites,
            s.library.root,
            // Everything a painting's own view depends on: its tone, museum switch and rotation membership.
            s.settings.appearance.overrides,
            s.tone,
            s.settings.sources.disabled_museums,
            s.settings.rotation.scope,
            s.settings.playlists,
        ])
        .to_string();
        if key == self.library_key {
            return;
        }
        self.library_key = key;
        if self.immediate {
            self.library = Some(Arc::new(self.cx.app.core().library_view()));
            return;
        }
        let Some(handle) = self.handle.clone() else {
            return;
        };
        self.cx.run(move |app| {
            let library = Arc::new(app.core().library_view());
            handle.update(move |ui: &mut SettingsUi, _| ui.library = Some(library));
        });
    }

    /// Ask for the art folder with the system's folder picker.
    pub fn choose_folder(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Choose the folder that contains manifest.json".into(),
            directory: self.snap.library.root.clone(),
            ..Default::default()
        };
        tx.open_folder(options, |s: &mut SettingsUi, result, _| match result {
            Ok(Some(path)) => s.cx.run(move |app| {
                if let Err(e) = actions::set_library_path(app, Some(path)) {
                    log::warn!("art folder: {e}");
                }
            }),
            Ok(None) => {}
            Err(e) => {
                s.library_page.notice = Some(format!("The folder picker could not open: {e}"))
            }
        });
    }

    /// The painting on the first display that shows one.
    pub fn sample(&self) -> Option<&crate::core::PaintingView> {
        self.snap.displays.iter().find_map(|d| d.painting.as_ref())
    }
}

/// Named states for `galeria ui --window settings --scene <name>`.
pub const SCENES: &[crate::ui::Scene<SettingsUi>] = &[
    ("now", |s| s.go(Tab::Now)),
    ("browse", |s| s.go(Tab::Browse)),
    ("browse-detail", |s| {
        s.go(Tab::Browse);
        s.browse.selected = s
            .library
            .as_ref()
            .and_then(|l| l.paintings.get(2))
            .map(|p| p.id.clone());
    }),
    ("browse-select", |s| {
        s.go(Tab::Browse);
        s.browse.picking = true;
        if let Some(l) = &s.library {
            s.browse.picked = l
                .paintings
                .iter()
                .skip(1)
                .step_by(3)
                .map(|p| p.id.clone())
                .collect();
        }
    }),
    ("discover", |s| s.go(Tab::Discover)),
    ("discover-cleveland", |s| {
        s.discover.text = rune::TextEdit::new("Monet");
        s.discover_from("cma", "");
        s.discover.focus = s.discover.items.first().map(|c| c.id.clone());
        s.discover.selected = s
            .discover
            .items
            .iter()
            .skip(1)
            .take(2)
            .map(|c| c.id.clone())
            .collect();
    }),
    ("sources", |s| s.go(Tab::Sources)),
    ("rotation", |s| s.go(Tab::Rotation)),
    ("wallpaper", |s| s.go(Tab::Wallpaper)),
    ("plate", |s| s.go(Tab::Plate)),
    ("library", |s| s.go(Tab::Library)),
    ("general", |s| s.go(Tab::General)),
    ("onboard-welcome", |s| {
        s.onboarding = Some(onboarding::State::default());
    }),
    ("onboard-desktop", |s| {
        s.onboarding = Some(onboarding::State {
            step: onboarding::Step::Desktop,
            ..Default::default()
        });
    }),
    ("onboard-folder", |s| {
        let folder = onboarding::recommended_folder();
        s.onboarding = Some(onboarding::State {
            step: onboarding::Step::Folder,
            folder_note: "An empty folder. Galeria starts a small index in it (manifest.json) and saves your downloads next to it.".into(),
            folder,
            ..Default::default()
        });
    }),
    ("onboard-paintings", |s| {
        s.onboarding = Some(onboarding::State {
            step: onboarding::Step::Paintings,
            ..Default::default()
        });
        s.discover.catalogue = Some(crate::actions::catalogue(s.cx.app.core()));
        s.onboard_show("cma", "landscape");
        // A theme and a museum taken whole, to show how that looks.
        s.onboard_toggle_theme("winter");
        s.onboard_toggle_museum("rijks");
    }),
    ("onboard-hanging", |s| {
        s.onboarding = Some(onboarding::State {
            step: onboarding::Step::Hanging,
            asked: 12,
            ..Default::default()
        });
    }),
];

pub fn poll(s: &mut SettingsUi, clock: f64, tx: &mut Tx) -> bool {
    // "Next painting in 12 min" counts down while the page is open.
    let tick = s.tab == Tab::Now && clock - s.clock >= 30.0;
    if tick || s.clock == 0.0 {
        s.clock = clock.max(f64::MIN_POSITIVE);
        if s.tab == Tab::Now {
            tx.wake_after(std::time::Duration::from_secs(30));
        }
    }
    tick
}

pub fn on_key(s: &mut SettingsUi, ev: &KeyEvent, _: &mut Tx) -> bool {
    if s.onboarding.is_some() {
        return false;
    }
    // ⌘1 … ⌘9 switch pages, as in other settings windows.
    if let Key::Command(c) = ev.key {
        if let Some(i) = c.to_digit(10).filter(|d| (1..=9).contains(d)) {
            s.go(Tab::ALL[i as usize - 1].0);
            return true;
        }
    }
    false
}

pub(super) fn app_icon() -> &'static Pixels {
    static ICON: OnceLock<Pixels> = OnceLock::new();
    ICON.get_or_init(|| {
        let pixels = Pixels::new();
        if let Ok(img) =
            image::load_from_memory(include_bytes!("../../../assets/icons/128x128@2x.png"))
        {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            pixels.set(w, h, rgba.into_raw());
        }
        pixels
    })
}

fn sidebar(s: &SettingsUi, l: &Look) -> El {
    let snap = &s.snap;
    let brand = row()
        .key("brand")
        .w_full()
        .gap(10.0)
        .center_y()
        .px(6.0)
        .pb(6.0)
        .window_drag()
        .child(el().size(34.0, 34.0).pixels(app_icon()).a11y_hidden())
        .child(
            col()
                .child(
                    text("Galeria")
                        .font(TextStyle::new(15.0, 650.0).lh(19.0))
                        .color(l.text),
                )
                .child(
                    text(widgets::plural(snap.library.valid, "painting", "paintings"))
                        .font(widgets::small())
                        .color(l.muted),
                ),
        );
    let issues = snap.library.missing + snap.library.invalid + snap.library.duplicates;
    let off = snap.museums.iter().filter(|m| !m.enabled).count();
    let mut bar = Sidebar::new()
        .label("Pages")
        .inset(0.0)
        .header_gap(4.0)
        .header_padding(0.0, 12.0)
        .row_gap(2.0)
        .header(brand);
    for (tab, key, label, icon) in Tab::ALL {
        let mut r = SidebarRow::new(label)
            .icon(icon)
            .height(32.0)
            .selected(s.tab == tab)
            .on_select(move |s: &mut SettingsUi, _| s.go(tab));
        r = match tab {
            Tab::Library if issues > 0 => r.badge(issues.to_string()),
            Tab::Discover if snap.downloads.active > 0 => {
                r.badge(format!("↓ {}", snap.downloads.active))
            }
            Tab::Sources if off > 0 => r.detail(format!("{off} off")),
            _ => r,
        };
        bar = bar.child(r.build(&l.side).key(key));
    }
    bar.build(&l.side)
}

/// A page that scrolls: sections stacked down a centred column.
pub fn scroll_page(key: &str, sections: Vec<El>) -> El {
    col()
        .key(key)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col().w_full().items_center().child(
                col()
                    .w_full()
                    .max_w(860.0)
                    .px(28.0)
                    .pt(6.0)
                    .pb(36.0)
                    .gap(30.0)
                    .children(sections),
            ),
        )
}

pub fn view(s: &SettingsUi, cx: &mut Cx<SettingsUi>) -> El {
    s.cx.report_dark(cx.system_dark());
    let l = theme::look(cx.system_dark(), cx.env().scale);
    let mut layer: Vec<El> = Vec::new();
    if s.onboarding.is_some() {
        return rune_kit::root(onboarding::view(s, &l, cx), layer);
    }
    let page = match s.tab {
        Tab::Now => now::page(s, &l, &mut layer),
        Tab::Browse => browse::page(s, &l, &mut layer, cx),
        Tab::Discover => discover::page(s, &l, &mut layer, cx),
        Tab::Sources => sources::page(s, &l, &mut layer),
        Tab::Rotation => rotation::page(s, &l, &mut layer),
        Tab::Wallpaper => wallpaper::page(s, &l, &mut layer),
        Tab::Plate => plate::page(s, &l, &mut layer, cx),
        Tab::Library => library::page(s, &l, &mut layer),
        Tab::General => general::page(s, &l, &mut layer),
    };
    let body = SplitView::new(col().key("page").w_full().h_full().min_h(0.0).child(page))
        .sidebar(
            &s.sidebar,
            |s: &mut SettingsUi| &mut s.sidebar,
            sidebar(s, &l),
        )
        .toolbar(Toolbar::new().title(s.tab.title()))
        .titlebar_inset(cx.titlebar_inset())
        .frame_inset(8.0)
        .toolbar_in_panel()
        .sidebar_padding(14.0, 10.0)
        .hide_sidebar_toggle()
        .reduced_motion(cx.is_reduced_motion())
        .build(&l.kit);
    rune_kit::root(body, layer)
}

#[cfg(test)]
mod tests;
