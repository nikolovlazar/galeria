//! Application state and the logic that ties everything together: library
//! (re)loading, which painting is on which display, rotation, system
//! wallpaper sync and the background enrichment worker.

use crate::displays::{self, DisplayInfo};
use crate::enrich::{self, Enricher, Enrichment, Job};
use crate::images::{self, Source};
use crate::library::{self, DimCache, Issue, Library, Painting};
use crate::platform::{self, SystemWallpaperOptions};
use crate::settings::{self, FitMode, Matte, Order, Settings, WallpaperSettings};
use crate::styles::{self, CustomStyle};
use crate::util::now;
use crate::{tray, wall};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

/// Files modified more recently than this are assumed to still be downloading.
const FRESH_FILE_GRACE_SECS: u64 = 20;

pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
    /// Copy of the OS wallpaper configuration from before Galeria changed it (macOS).
    pub wallpaper_backup: PathBuf,
    pub settings: PathBuf,
    pub state: PathBuf,
    pub enrichment: PathBuf,
    pub styles: PathBuf,
}

impl Paths {
    pub fn resolve(app: &AppHandle) -> Result<Paths, String> {
        let config = app.path().app_config_dir().map_err(|e| e.to_string())?;
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
        for d in [&config, &data, &cache] {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        Ok(Paths {
            settings: config.join("settings.json"),
            styles: config.join("plate-styles"),
            state: data.join("state.json"),
            enrichment: data.join("enrichment.json"),
            wallpaper_backup: data.join("original-wallpaper-store.plist"),
            config,
            cache,
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Assignment {
    pub painting: String,
    pub shown_at: u64,
    pub history: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Persisted {
    pub assignments: HashMap<String, Assignment>,
    pub deck: Vec<String>,
    pub deck_pos: usize,
    pub deck_order: Option<Order>,
    /// Wallpaper each display had before Galeria first changed it.
    pub original_wallpapers: HashMap<String, PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Colors {
    pub average: String,
    pub wall: String,
}

pub struct Inner {
    pub settings: Settings,
    pub library: Library,
    pub dims: DimCache,
    pub persisted: Persisted,
    pub displays: Vec<DisplayInfo>,
    pub arrange: bool,
    pub enrich: enrich::Store,
    enrich_dirty: usize,
    enrich_attempted: HashSet<String>,
    pub enrich_busy: bool,
    /// Paintings that failed to load in a webview (id → when).
    pub bad: HashMap<String, u64>,
    pub styles: Vec<CustomStyle>,
    pub quitting: bool,
    last_snapshot: String,
    last_tray: String,
}

pub struct Core {
    pub paths: Paths,
    inner: Mutex<Inner>,
    pub enrich_wake: Notify,
    watchers: Mutex<Vec<Box<dyn std::any::Any + Send>>>,
    colors: Mutex<HashMap<String, Colors>>,
}

// ---------------------------------------------------------------------------
// Views sent to the webviews
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PaintingView {
    pub id: String,
    pub file: String,
    pub title: String,
    pub original_title: Option<String>,
    pub artist: Option<String>,
    pub artist_bio: Option<String>,
    pub date: Option<String>,
    pub medium: Option<String>,
    pub dimensions: Option<String>,
    pub description: Option<String>,
    pub description_source: Option<String>,
    pub description_license: Option<String>,
    pub description_lang: Option<String>,
    pub did_you_know: Option<String>,
    pub culture: Option<String>,
    pub museum: Option<String>,
    pub credit: Option<String>,
    pub credit_line: Option<String>,
    pub accession: Option<String>,
    pub license: Option<String>,
    pub source: Option<String>,
    pub source_page: Option<String>,
    pub wikipedia_url: Option<String>,
    pub tags: Vec<String>,
    pub kind: Option<String>,
    pub width: u32,
    pub height: u32,
    pub aspect: f64,
    pub orientation: &'static str,
    pub bytes: u64,
    pub excluded: bool,
    pub favorite: bool,
    pub enriched: bool,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayView {
    #[serde(flatten)]
    pub info: DisplayInfo,
    pub painting: Option<PaintingView>,
    pub shown_at: u64,
    pub next_change_at: Option<u64>,
    pub history_len: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    pub root: Option<PathBuf>,
    pub error: Option<String>,
    pub manifest_found: bool,
    pub entries: usize,
    pub valid: usize,
    pub eligible: usize,
    pub missing: usize,
    pub invalid: usize,
    pub duplicates: usize,
    pub unindexed: usize,
    pub excluded: usize,
    pub generated: Option<String>,
    pub loaded_at: u64,
    pub enriched: usize,
    pub enrich_busy: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinStyle {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub settings: Settings,
    pub displays: Vec<DisplayView>,
    pub arrange: bool,
    pub library: LibrarySummary,
    pub builtin_styles: Vec<BuiltinStyle>,
    pub custom_styles: Vec<CustomStyle>,
    pub styles_dir: PathBuf,
    pub platform: &'static str,
    pub live_supported: bool,
    pub version: &'static str,
    pub original_wallpapers: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub paintings: Vec<PaintingView>,
    pub missing: Vec<Issue>,
    pub invalid: Vec<Issue>,
    pub duplicates: Vec<Issue>,
    pub unindexed: Vec<String>,
}

// ---------------------------------------------------------------------------

impl Core {
    pub fn new(paths: Paths) -> Core {
        let mut settings = Settings::load(&paths.settings);
        if settings.library_path.is_none() {
            if let Some(p) = std::env::var_os("GALERIA_LIBRARY") {
                settings.library_path = Some(PathBuf::from(p));
            }
        }
        let persisted: Persisted = std::fs::read(&paths.state)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let enrich: enrich::Store = std::fs::read(&paths.enrichment)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let _ = std::fs::create_dir_all(&paths.styles);
        let styles = styles::load(&paths.styles);
        Core {
            inner: Mutex::new(Inner {
                settings,
                library: Library::default(),
                dims: DimCache::default(),
                persisted,
                displays: Vec::new(),
                arrange: false,
                enrich,
                enrich_dirty: 0,
                enrich_attempted: HashSet::new(),
                enrich_busy: false,
                bad: HashMap::new(),
                styles,
                quitting: false,
                last_snapshot: String::new(),
                last_tray: String::new(),
            }),
            paths,
            enrich_wake: Notify::new(),
            watchers: Mutex::new(Vec::new()),
            colors: Mutex::new(HashMap::new()),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn source_for(&self, id: &str) -> Option<Source> {
        let i = self.lock();
        i.library.get(id).map(|p| Source {
            path: p.path.clone(),
            width: p.width,
            height: p.height,
            bytes: p.bytes,
            modified: p.modified,
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.lock(), &self.paths)
    }

    pub fn library_view(&self) -> LibraryView {
        let i = self.lock();
        LibraryView {
            paintings: i.library.paintings.iter().map(|p| view(&i, p)).collect(),
            missing: i.library.report.missing.clone(),
            invalid: i.library.report.invalid.clone(),
            duplicates: i.library.report.duplicates.clone(),
            unindexed: i.library.report.unindexed.clone(),
        }
    }

    pub fn painting_view(&self, id: &str) -> Option<PaintingView> {
        let i = self.lock();
        i.library.get(id).map(|p| view(&i, p))
    }

    fn save_state(&self, i: &Inner) {
        if let Err(e) = settings::write_json_atomic(&self.paths.state, &i.persisted) {
            log::warn!("could not save state: {e}");
        }
    }

    fn save_enrichment(&self, i: &Inner) {
        if let Err(e) = settings::write_json_atomic(&self.paths.enrichment, &i.enrich) {
            log::warn!("could not save enrichment cache: {e}");
        }
    }

    pub fn colors_for(&self, id: &str) -> Result<Colors, String> {
        if let Some(c) = self.colors.lock().unwrap().get(id) {
            return Ok(c.clone());
        }
        let src = self.source_for(id).ok_or("unknown painting")?;
        let average = images::average_color(&src, &self.paths.cache)?;
        let c = Colors { wall: wall_color(&average), average };
        self.colors.lock().unwrap().insert(id.to_string(), c.clone());
        Ok(c)
    }
}

fn view(i: &Inner, p: &Painting) -> PaintingView {
    let e: Option<&Enrichment> = i.enrich.works.get(&p.enrich_key());
    let fill = |m: &Option<String>, f: &dyn Fn(&Enrichment) -> Option<String>| m.clone().or_else(|| e.and_then(f));
    let (title, original_title) = match e.and_then(|e| e.title_en.clone()) {
        Some(en) if !en.eq_ignore_ascii_case(&p.title) => (en, Some(p.title.clone())),
        _ => (p.title.clone(), None),
    };
    let (description, description_source, description_license, description_lang) = match &p.description {
        Some(d) => (Some(d.clone()), None, None, None),
        None => match e {
            Some(e) => (
                e.description.clone(),
                e.description_source.clone(),
                e.description_license.clone(),
                e.description_lang.clone(),
            ),
            None => (None, None, None, None),
        },
    };
    PaintingView {
        id: p.id.clone(),
        file: p.file.clone(),
        title,
        original_title,
        artist: p.artist.clone(),
        artist_bio: fill(&p.artist_bio, &|e| e.artist_bio.clone()),
        date: p.date.clone(),
        medium: fill(&p.medium, &|e| e.technique.clone()),
        dimensions: fill(&p.dimensions, &|e| e.measurements.clone()),
        description,
        description_source,
        description_license,
        description_lang,
        did_you_know: e.and_then(|e| e.did_you_know.clone()),
        culture: e.and_then(|e| e.culture.clone()),
        museum: p.museum.clone(),
        credit: p.credit.clone(),
        credit_line: fill(&p.credit_line, &|e| e.credit_line.clone()),
        accession: fill(&p.accession, &|e| e.accession.clone()),
        license: p.license.clone(),
        source: p.source.clone(),
        source_page: p.source_page.clone(),
        wikipedia_url: e.and_then(|e| e.wikipedia_url.clone()),
        tags: p.tags.clone(),
        kind: p.kind.clone(),
        width: p.width,
        height: p.height,
        aspect: (p.aspect() * 1000.0).round() / 1000.0,
        orientation: p.orientation(),
        bytes: p.bytes,
        excluded: i.settings.excluded.binary_search(&p.id).is_ok(),
        favorite: i.settings.favorites.binary_search(&p.id).is_ok(),
        enriched: e.is_some_and(|e| e.ok),
        extra: p.extra.clone(),
    }
}

fn snapshot_of(i: &Inner, paths: &Paths) -> Snapshot {
    let interval = i.settings.rotation.interval_minutes as u64 * 60;
    let displays = i
        .displays
        .iter()
        .map(|d| {
            let a = i.persisted.assignments.get(&d.key);
            let painting = a.and_then(|a| i.library.get(&a.painting)).map(|p| view(i, p));
            let shown_at = a.map(|a| a.shown_at).unwrap_or(0);
            DisplayView {
                info: d.clone(),
                next_change_at: (interval > 0 && !i.settings.rotation.paused && painting.is_some())
                    .then_some(shown_at + interval),
                history_len: a.map(|a| a.history.len()).unwrap_or(0),
                painting,
                shown_at,
            }
        })
        .collect();
    let r = &i.library.report;
    Snapshot {
        settings: i.settings.clone(),
        displays,
        arrange: i.arrange,
        library: LibrarySummary {
            root: i.settings.library_path.clone(),
            error: r.error.clone(),
            manifest_found: r.manifest_found,
            entries: r.entries,
            valid: r.valid,
            eligible: i.library.paintings.iter().filter(|p| i.settings.excluded.binary_search(&p.id).is_err()).count(),
            missing: r.missing.len(),
            invalid: r.invalid.len(),
            duplicates: r.duplicates.len(),
            unindexed: r.unindexed_count,
            excluded: i.settings.excluded.len(),
            generated: r.generated.clone(),
            loaded_at: r.loaded_at,
            enriched: i
                .library
                .paintings
                .iter()
                .filter(|p| i.enrich.works.get(&p.enrich_key()).is_some_and(|e| e.ok))
                .count(),
            enrich_busy: i.enrich_busy,
        },
        builtin_styles: styles::BUILTIN.iter().map(|(id, name)| BuiltinStyle { id, name }).collect(),
        custom_styles: i.styles.clone(),
        styles_dir: paths.styles.clone(),
        platform: platform::PLATFORM,
        live_supported: platform::live_wallpaper_supported(),
        version: env!("CARGO_PKG_VERSION"),
        original_wallpapers: if paths.wallpaper_backup.exists() {
            i.displays.len()
        } else {
            i.persisted.original_wallpapers.values().filter(|p| p.is_file()).count()
        },
    }
}

/// A muted museum wall tinted towards the painting's average colour.
pub fn wall_color(avg: &str) -> String {
    let (r, g, b) = settings::parse_hex_color(avg);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (mut h, mut s) = (0.0, 0.0);
    if d > 1e-6 {
        s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } / 6.0;
    }
    let s = (s * 0.55).min(0.32);
    let l = 0.11 + l * 0.08;
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let c = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        c(hue(p, q, h + 1.0 / 3.0)),
        c(hue(p, q, h)),
        c(hue(p, q, h - 1.0 / 3.0))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutKind {
    Cover,
    Contain,
    Pan,
}

/// Mirrors `computeLayout` in src/shared/layout.ts.
pub fn layout_kind(w: &WallpaperSettings, iw: f64, ih: f64, vw: f64, vh: f64) -> LayoutKind {
    if iw <= 0.0 || ih <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return LayoutKind::Cover;
    }
    let cover = (vw / iw).max(vh / ih);
    let contain = (vw / iw).min(vh / ih);
    let crop = 1.0 - contain / cover;
    let kind = match w.fit {
        FitMode::Fit => LayoutKind::Contain,
        FitMode::Fill => LayoutKind::Cover,
        FitMode::Pan => LayoutKind::Pan,
        FitMode::Auto if crop <= w.crop_tolerance + 1e-6 => LayoutKind::Cover,
        FitMode::Auto if w.pan => LayoutKind::Pan,
        FitMode::Auto => LayoutKind::Contain,
    };
    if kind == LayoutKind::Pan && crop < 0.004 {
        LayoutKind::Cover
    } else {
        kind
    }
}

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

fn eligible(i: &Inner, p: &Painting, display: Option<&DisplayInfo>, strict: bool) -> bool {
    if i.settings.excluded.binary_search(&p.id).is_ok() {
        return false;
    }
    if let Some(&t) = i.bad.get(&p.id) {
        if now().saturating_sub(t) < 3600 {
            return false;
        }
    }
    if now().saturating_sub(p.modified) < FRESH_FILE_GRACE_SECS {
        return false;
    }
    if strict && i.settings.wallpaper.match_orientation {
        if let Some(d) = display {
            let a = p.aspect();
            if d.is_portrait() && a > 1.1 {
                return false;
            }
            if !d.is_portrait() && a < 0.9 {
                return false;
            }
        }
    }
    true
}

fn sync_deck(i: &mut Inner) {
    let order = i.settings.rotation.order;
    let ids: Vec<String> = i.library.paintings.iter().map(|p| p.id.clone()).collect();
    let known: HashSet<&String> = ids.iter().collect();
    let rebuild = i.persisted.deck_order != Some(order) || i.persisted.deck.is_empty();
    if rebuild {
        let mut deck = ids.clone();
        if order == Order::Shuffle {
            fastrand::shuffle(&mut deck);
        }
        i.persisted.deck = deck;
        i.persisted.deck_pos = 0;
        i.persisted.deck_order = Some(order);
        return;
    }
    match order {
        Order::Sequential => {
            let current = i.persisted.deck.get(i.persisted.deck_pos.saturating_sub(1)).cloned();
            i.persisted.deck = ids.clone();
            i.persisted.deck_pos = current
                .and_then(|c| ids.iter().position(|id| *id == c))
                .map(|p| p + 1)
                .unwrap_or(0);
        }
        Order::Shuffle => {
            let pos = i.persisted.deck_pos.min(i.persisted.deck.len());
            let removed_before = i.persisted.deck[..pos].iter().filter(|id| !known.contains(id)).count();
            i.persisted.deck.retain(|id| known.contains(id));
            i.persisted.deck_pos = pos - removed_before;
            let in_deck: HashSet<String> = i.persisted.deck.iter().cloned().collect();
            for id in ids.iter().filter(|id| !in_deck.contains(*id)) {
                // New arrivals land somewhere in the not-yet-shown part of the deck.
                let lo = i.persisted.deck_pos;
                let at = fastrand::usize(lo..=i.persisted.deck.len());
                i.persisted.deck.insert(at, id.clone());
            }
        }
    }
}

fn pick_next(i: &mut Inner, display_key: &str) -> Option<String> {
    let n = i.persisted.deck.len();
    if n == 0 {
        return None;
    }
    let display = i.displays.iter().find(|d| d.key == display_key).cloned();
    let current = i.persisted.assignments.get(display_key).map(|a| a.painting.clone());
    let elsewhere: HashSet<String> = if i.settings.wallpaper.same_on_all_displays {
        HashSet::new()
    } else {
        i.persisted
            .assignments
            .iter()
            .filter(|(k, _)| k.as_str() != display_key && i.displays.iter().any(|d| &d.key == *k))
            .map(|(_, a)| a.painting.clone())
            .collect()
    };
    for strict in [true, false] {
        for avoid_others in [true, false] {
            for _ in 0..n {
                if i.persisted.deck_pos >= n {
                    i.persisted.deck_pos = 0;
                    if i.settings.rotation.order == Order::Shuffle {
                        fastrand::shuffle(&mut i.persisted.deck);
                        if n > 1 && current.as_deref() == Some(i.persisted.deck[0].as_str()) {
                            i.persisted.deck.swap(0, n - 1);
                        }
                    }
                }
                let idx = i.persisted.deck_pos;
                let id = i.persisted.deck[idx].clone();
                i.persisted.deck_pos += 1;
                let ok = i.library.get(&id).is_some_and(|p| eligible(i, p, display.as_ref(), strict))
                    && (n == 1 || current.as_deref() != Some(id.as_str()))
                    && (!avoid_others || !elsewhere.contains(&id));
                if ok {
                    return Some(id);
                }
            }
        }
    }
    None
}

fn assign(i: &mut Inner, key: &str, id: String, push_history: bool) {
    let a = i.persisted.assignments.entry(key.to_string()).or_default();
    if push_history && !a.painting.is_empty() && a.painting != id {
        a.history.push(std::mem::take(&mut a.painting));
        if a.history.len() > 100 {
            a.history.remove(0);
        }
    }
    a.painting = id;
    a.shown_at = now();
}

/// Make sure every display shows a valid painting. Returns displays that changed.
fn ensure_assignments(i: &mut Inner) -> Vec<String> {
    let mut changed = Vec::new();
    let keys: Vec<String> = i.displays.iter().map(|d| d.key.clone()).collect();
    let same = i.settings.wallpaper.same_on_all_displays;
    let primary = keys.first().cloned();
    for key in &keys {
        if same && Some(key) != primary.as_ref() {
            let p = primary.as_ref().and_then(|k| i.persisted.assignments.get(k)).map(|a| a.painting.clone());
            if let Some(p) = p {
                let cur = i.persisted.assignments.get(key).map(|a| a.painting.clone());
                if cur.as_deref() != Some(p.as_str()) && !p.is_empty() {
                    assign(i, key, p, true);
                    changed.push(key.clone());
                }
            }
            continue;
        }
        let valid = i
            .persisted
            .assignments
            .get(key)
            .and_then(|a| i.library.get(&a.painting))
            .is_some_and(|p| i.settings.excluded.binary_search(&p.id).is_err());
        if !valid {
            if let Some(id) = pick_next(i, key) {
                assign(i, key, id, false);
                changed.push(key.clone());
            }
        }
    }
    changed
}

// ---------------------------------------------------------------------------
// Actions (all take the app handle; none hold the lock while calling Tauri)
// ---------------------------------------------------------------------------

/// Broadcast the snapshot to every webview and refresh the tray menu, but
/// only when something actually changed.
pub fn publish(app: &AppHandle) {
    let core = app.state::<Core>();
    let (snap, changed, tray_changed) = {
        let mut i = core.lock();
        let snap = snapshot_of(&i, &core.paths);
        let json = serde_json::to_string(&snap).unwrap_or_default();
        let changed = json != i.last_snapshot;
        if changed {
            i.last_snapshot = json;
        }
        let sig = tray::signature(&snap);
        let tray_changed = sig != i.last_tray;
        if tray_changed {
            i.last_tray = sig;
        }
        (snap, changed, tray_changed)
    };
    if changed {
        let _ = app.emit("snapshot", &snap);
    }
    if tray_changed {
        tray::update(app, &snap);
    }
}

pub fn start(app: &AppHandle) {
    refresh_displays(app, true);
    reload_library(app);
    restart_watchers(app);
    wall::reconcile(app);
    tray::create(app);
    publish(app);

    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            n += 1;
            rotation_tick(&a);
            if n.is_multiple_of(2) {
                let a2 = a.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || refresh_displays(&a2, false)).await;
            }
        }
    });
    let a = app.clone();
    tauri::async_runtime::spawn(enrich_loop(a));
}

pub fn refresh_displays(app: &AppHandle, initial: bool) {
    let list = displays::list(app);
    if list.is_empty() {
        return;
    }
    let core = app.state::<Core>();
    let changed = {
        let mut i = core.lock();
        if i.displays == list {
            return;
        }
        log::info!("displays: {:?}", list.iter().map(|d| format!("{} {}x{}@{}", d.name, d.width, d.height, d.scale)).collect::<Vec<_>>());
        i.displays = list;
        let changed = ensure_assignments(&mut i);
        core.save_state(&i);
        changed
    };
    if !initial {
        wall::reconcile(app);
        publish(app);
        if !changed.is_empty() {
            sync_system_wallpaper(app, Some(changed));
        }
    }
}

pub fn reload_library(app: &AppHandle) {
    let core = app.state::<Core>();
    let (root, mut dims) = {
        let mut i = core.lock();
        (i.settings.library_path.clone(), std::mem::take(&mut i.dims))
    };
    let lib = match &root {
        Some(r) => library::load(r, &mut dims),
        None => Library::default(),
    };
    let changed = {
        let mut i = core.lock();
        i.dims = dims;
        let same_root = i.library.root.as_os_str() == lib.root.as_os_str();
        if lib.report.error.is_some() && lib.report.manifest_found && same_root && !i.library.paintings.is_empty() {
            // The manifest is probably mid-write; keep the last good copy.
            log::warn!("manifest unreadable, keeping previous library: {:?}", lib.report.error);
            i.library.report.error = lib.report.error;
        } else {
            log::info!(
                "library: {} paintings ({} entries, {} missing, {} invalid, {} unindexed)",
                lib.paintings.len(),
                lib.report.entries,
                lib.report.missing.len(),
                lib.report.invalid.len(),
                lib.report.unindexed_count
            );
            i.library = lib;
        }
        sync_deck(&mut i);
        let changed = ensure_assignments(&mut i);
        core.save_state(&i);
        changed
    };
    publish(app);
    if !changed.is_empty() {
        sync_system_wallpaper(app, Some(changed));
    }
    core.enrich_wake.notify_one();
}

pub fn reload_styles(app: &AppHandle) {
    let core = app.state::<Core>();
    let styles = styles::load(&core.paths.styles);
    core.lock().styles = styles;
    publish(app);
}

pub fn restart_watchers(app: &AppHandle) {
    use notify::RecursiveMode;
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};

    let core = app.state::<Core>();
    let root = core.lock().settings.library_path.clone();
    let mut watchers = core.watchers.lock().unwrap();
    watchers.clear();

    if let Some(root) = root.filter(|r| r.is_dir()) {
        let a = app.clone();
        let res = new_debouncer(Duration::from_millis(1500), None, move |res: DebounceEventResult| {
            let Ok(events) = res else { return };
            let relevant = events.iter().any(|e| {
                !matches!(e.event.kind, notify::EventKind::Access(_))
                    && e.event.paths.iter().any(|p| {
                        p.file_name().is_some_and(|n| n == library::MANIFEST) || library::is_image(p)
                    })
            });
            if relevant {
                let a = a.clone();
                tauri::async_runtime::spawn_blocking(move || reload_library(&a));
            }
        });
        match res {
            Ok(mut d) => match d.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => watchers.push(Box::new(d)),
                Err(e) => log::warn!("cannot watch {}: {e}", root.display()),
            },
            Err(e) => log::warn!("file watcher unavailable: {e}"),
        }
    }

    let a = app.clone();
    if let Ok(mut d) = new_debouncer(Duration::from_millis(400), None, move |res: DebounceEventResult| {
        if res.is_ok() {
            reload_styles(&a);
        }
    }) {
        if d.watch(&core.paths.styles, RecursiveMode::NonRecursive).is_ok() {
            watchers.push(Box::new(d));
        }
    }
}

fn rotation_tick(app: &AppHandle) {
    let core = app.state::<Core>();
    let due: Vec<String> = {
        let i = core.lock();
        let r = &i.settings.rotation;
        if r.paused || r.interval_minutes == 0 || i.arrange {
            return;
        }
        let interval = r.interval_minutes as u64 * 60;
        let t = now();
        let mut due: Vec<String> = i
            .displays
            .iter()
            .filter(|d| {
                i.persisted
                    .assignments
                    .get(&d.key)
                    .is_none_or(|a| t.saturating_sub(a.shown_at) >= interval)
            })
            .map(|d| d.key.clone())
            .collect();
        if i.settings.wallpaper.same_on_all_displays {
            due.truncate(1);
        }
        due
    };
    for key in due {
        advance(app, Some(key), true);
    }
}

/// Next (`forward`) or previous painting. `display: None` = every display.
pub fn advance(app: &AppHandle, display: Option<String>, forward: bool) {
    let core = app.state::<Core>();
    let changed = {
        let mut i = core.lock();
        let same = i.settings.wallpaper.same_on_all_displays;
        let all: Vec<String> = i.displays.iter().map(|d| d.key.clone()).collect();
        let targets: Vec<String> = match (&display, same) {
            (_, true) => all.first().cloned().into_iter().collect(),
            (Some(k), false) => vec![k.clone()],
            (None, false) => all.clone(),
        };
        let mut changed = Vec::new();
        for key in targets {
            if forward {
                if let Some(id) = pick_next(&mut i, &key) {
                    assign(&mut i, &key, id, true);
                    changed.push(key);
                }
            } else {
                let prev = i.persisted.assignments.get_mut(&key).and_then(|a| {
                    while let Some(p) = a.history.pop() {
                        if !p.is_empty() {
                            return Some(p);
                        }
                    }
                    None
                });
                if let Some(id) = prev.filter(|id| i.library.get(id).is_some()) {
                    assign(&mut i, &key, id, false);
                    changed.push(key);
                }
            }
        }
        if same && !changed.is_empty() {
            changed.extend(ensure_assignments(&mut i));
        }
        core.save_state(&i);
        changed
    };
    if !changed.is_empty() {
        publish(app);
        sync_system_wallpaper(app, Some(changed));
        core.enrich_wake.notify_one();
    }
}

/// Put a specific painting on a display (`None` = every display).
pub fn show(app: &AppHandle, id: &str, display: Option<String>) -> Result<(), String> {
    let core = app.state::<Core>();
    let changed = {
        let mut i = core.lock();
        if i.library.get(id).is_none() {
            return Err("unknown painting".into());
        }
        let keys: Vec<String> = match display {
            Some(k) if !i.settings.wallpaper.same_on_all_displays => vec![k],
            _ => i.displays.iter().map(|d| d.key.clone()).collect(),
        };
        for k in &keys {
            assign(&mut i, k, id.to_string(), true);
        }
        i.bad.remove(id);
        core.save_state(&i);
        keys
    };
    publish(app);
    sync_system_wallpaper(app, Some(changed));
    core.enrich_wake.notify_one();
    Ok(())
}

pub fn patch_settings(app: &AppHandle, patch: &Value) -> Result<Settings, String> {
    let core = app.state::<Core>();
    let (old, new) = {
        let mut i = core.lock();
        let new = i.settings.patched(patch)?;
        let old = std::mem::replace(&mut i.settings, new.clone());
        if old.rotation.order != new.rotation.order {
            sync_deck(&mut i);
        }
        (old, new)
    };
    if let Err(e) = new.save(&core.paths.settings) {
        log::warn!("could not save settings: {e}");
    }
    apply_settings_change(app, &old, &new);
    publish(app);
    Ok(new)
}

fn apply_settings_change(app: &AppHandle, old: &Settings, new: &Settings) {
    if old.library_path != new.library_path {
        reload_library(app);
        restart_watchers(app);
    }
    if old.general.show_in_dock != new.general.show_in_dock {
        apply_dock_policy(app, new.general.show_in_dock);
    }
    if old.general.launch_at_login != new.general.launch_at_login {
        use tauri_plugin_autostart::ManagerExt;
        let r = if new.general.launch_at_login { app.autolaunch().enable() } else { app.autolaunch().disable() };
        if let Err(e) = r {
            log::warn!("autostart: {e}");
        }
    }
    if old.wallpaper.live != new.wallpaper.live {
        let a = app.clone();
        tauri::async_runtime::spawn(async move { wall::reconcile(&a) });
    }
    let core = app.state::<Core>();
    let mut reassigned = Vec::new();
    if old.excluded != new.excluded
        || old.wallpaper.same_on_all_displays != new.wallpaper.same_on_all_displays
        || old.wallpaper.match_orientation != new.wallpaper.match_orientation
    {
        let mut i = core.lock();
        reassigned = ensure_assignments(&mut i);
        core.save_state(&i);
    }
    let w0 = &old.wallpaper;
    let w1 = &new.wallpaper;
    let affects_static = (!w0.sync_system && w1.sync_system)
        || w0.fit != w1.fit
        || w0.matte != w1.matte
        || w0.matte_color != w1.matte_color
        || w0.crop_tolerance != w1.crop_tolerance
        || w0.pan != w1.pan;
    if affects_static {
        sync_system_wallpaper(app, None);
    } else if !reassigned.is_empty() {
        sync_system_wallpaper(app, Some(reassigned));
    }
    if !old.general.enrich_online && new.general.enrich_online {
        core.enrich_wake.notify_one();
    }
}

pub fn apply_dock_policy(app: &AppHandle, show: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if show { tauri::ActivationPolicy::Regular } else { tauri::ActivationPolicy::Accessory };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, show);
}

pub fn set_plate_position(app: &AppHandle, scope: &str, display: Option<String>, x: f64, y: f64) {
    let patch = match (scope, display) {
        ("viewer", _) => serde_json::json!({ "viewer": { "platePosition": { "x": x, "y": y } } }),
        (_, Some(key)) => serde_json::json!({ "plate": { "positions": { key: { "x": x, "y": y } } } }),
        (_, None) => return set_plate_preset(app, x, y),
    };
    if let Err(e) = patch_settings(app, &patch) {
        log::warn!("plate position: {e}");
    }
}

/// Snap every display's plate to a preset corner.
pub fn set_plate_preset(app: &AppHandle, x: f64, y: f64) {
    let core = app.state::<Core>();
    {
        let mut i = core.lock();
        i.settings.plate.positions.clear();
        i.settings.plate.default_position = settings::PlatePosition { x, y };
        let s = i.settings.clone();
        drop(i);
        let _ = s.save(&core.paths.settings);
    }
    publish(app);
}

pub fn set_excluded(app: &AppHandle, id: &str, excluded: bool) {
    let mut list = app.state::<Core>().lock().settings.excluded.clone();
    list.retain(|x| x != id);
    if excluded {
        list.push(id.to_string());
    }
    let _ = patch_settings(app, &serde_json::json!({ "excluded": list }));
}

pub fn set_favorite(app: &AppHandle, id: &str, favorite: bool) {
    let mut list = app.state::<Core>().lock().settings.favorites.clone();
    list.retain(|x| x != id);
    if favorite {
        list.push(id.to_string());
    }
    let _ = patch_settings(app, &serde_json::json!({ "favorites": list }));
}

/// A webview could not decode a painting: skip it for an hour and move on.
pub fn report_bad_image(app: &AppHandle, id: &str, display: Option<String>) {
    let core = app.state::<Core>();
    let showing: Vec<String> = {
        let mut i = core.lock();
        i.bad.insert(id.to_string(), now());
        i.persisted
            .assignments
            .iter()
            .filter(|(k, a)| a.painting == id && display.as_ref().is_none_or(|d| d == *k))
            .map(|(k, _)| k.clone())
            .collect()
    };
    log::warn!("image failed to load: {id}");
    for key in showing {
        advance(app, Some(key), true);
    }
}

// ---------------------------------------------------------------------------
// System wallpaper
// ---------------------------------------------------------------------------

/// Set the OS wallpaper for the given displays (`None` = all) to their current painting.
pub fn sync_system_wallpaper(app: &AppHandle, keys: Option<Vec<String>>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let core = app.state::<Core>();
        let jobs: Vec<(DisplayInfo, String, Source, bool)> = {
            let i = core.lock();
            if !i.settings.wallpaper.sync_system {
                return;
            }
            i.displays
                .iter()
                .filter(|d| keys.as_ref().is_none_or(|k| k.contains(&d.key)))
                .filter_map(|d| {
                    let a = i.persisted.assignments.get(&d.key)?;
                    let p = i.library.get(&a.painting)?;
                    let kind = layout_kind(&i.settings.wallpaper, p.width as f64, p.height as f64, d.width, d.height);
                    let src = Source {
                        path: p.path.clone(),
                        width: p.width,
                        height: p.height,
                        bytes: p.bytes,
                        modified: p.modified,
                    };
                    Some((d.clone(), p.id.clone(), src, kind != LayoutKind::Contain))
                })
                .collect()
        };
        let (matte, matte_color, root) = {
            let i = core.lock();
            (i.settings.wallpaper.matte, i.settings.wallpaper.matte_color.clone(), i.settings.library_path.clone())
        };
        for (d, id, src, fill) in jobs {
            let color_hex = match matte {
                Matte::Color => matte_color.clone(),
                _ => {
                    let a = app.clone();
                    let id2 = id.clone();
                    tauri::async_runtime::spawn_blocking(move || a.state::<Core>().colors_for(&id2))
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .map(|c| c.wall)
                        .unwrap_or_else(|| matte_color.clone())
                }
            };
            let opts = SystemWallpaperOptions { fill, color: settings::parse_hex_color(&color_hex) };

            // Remember what the user had before Galeria touched it.
            let known = core.lock().persisted.original_wallpapers.contains_key(&d.key);
            if !known || !core.paths.wallpaper_backup.exists() {
                let d2 = d.clone();
                let a = app.clone();
                let original = tauri::async_runtime::spawn_blocking(move || {
                    platform::on_main(&a, move || platform::current_system_wallpaper(&d2)).flatten()
                })
                .await
                .ok()
                .flatten();
                // Aerial/dynamic wallpapers may report no URL at all; that still isn't ours.
                let ours = original.as_ref().is_some_and(|o| root.as_ref().is_some_and(|r| o.starts_with(r)));
                if !ours {
                    // Only back up a configuration that isn't already Galeria's.
                    platform::backup_wallpaper_store(&core.paths.wallpaper_backup);
                    if let (false, Some(o)) = (known, original) {
                        let mut i = core.lock();
                        i.persisted.original_wallpapers.insert(d.key.clone(), o);
                        core.save_state(&i);
                    }
                }
            }

            let a = app.clone();
            let path = src.path.clone();
            let name = d.name.clone();
            let res = tauri::async_runtime::spawn_blocking(move || {
                platform::on_main(&a, move || platform::set_system_wallpaper(&d, &path, opts))
            })
            .await;
            match res {
                Ok(Some(Ok(()))) => log::info!("system wallpaper on {name}: {id}"),
                Ok(Some(Err(e))) => log::warn!("system wallpaper on {name}: {e}"),
                _ => log::warn!("system wallpaper on {name}: main thread unavailable"),
            }
        }
    });
}

pub fn restore_original_wallpaper(app: &AppHandle) {
    let core = app.state::<Core>();
    match platform::restore_wallpaper_store(&core.paths.wallpaper_backup) {
        Ok(true) => {
            log::info!("restored the original wallpaper configuration");
            return;
        }
        Ok(false) => {}
        Err(e) => log::warn!("restore wallpaper store: {e}"),
    }
    let jobs: Vec<(DisplayInfo, PathBuf)> = {
        let i = core.lock();
        i.displays
            .iter()
            .filter_map(|d| i.persisted.original_wallpapers.get(&d.key).map(|p| (d.clone(), p.clone())))
            .filter(|(_, p)| p.is_file())
            .collect()
    };
    for (d, path) in jobs {
        let opts = SystemWallpaperOptions { fill: true, color: (0.0, 0.0, 0.0) };
        let r = platform::on_main(app, move || platform::set_system_wallpaper(&d, &path, opts));
        if let Some(Err(e)) = r {
            log::warn!("restore wallpaper: {e}");
        }
    }
}

// ---------------------------------------------------------------------------
// Enrichment worker
// ---------------------------------------------------------------------------

impl Core {
    fn next_enrich_job(&self) -> Option<Job> {
        let mut i = self.lock();
        if !i.settings.general.enrich_online {
            return None;
        }
        let showing: Vec<String> = i.persisted.assignments.values().map(|a| a.painting.clone()).collect();
        let order: Vec<usize> = showing
            .iter()
            .filter_map(|id| i.library.index.get(id).copied())
            .chain(0..i.library.paintings.len())
            .collect();
        for idx in order {
            let p = &i.library.paintings[idx];
            let key = p.enrich_key();
            if i.enrich_attempted.contains(&key) {
                continue;
            }
            if i.enrich.works.get(&key).is_some_and(|e| !e.is_stale()) {
                continue;
            }
            let job = Job {
                key: key.clone(),
                source: p.source.clone(),
                source_id: p.source_id.clone(),
                source_page: p.source_page.clone(),
                artist: p.artist.clone(),
                title: p.title.clone(),
            };
            i.enrich_attempted.insert(key);
            return Some(job);
        }
        None
    }

    /// Forget cached enrichment (for one painting or everything) and fetch again.
    pub fn refresh_enrichment(&self, id: Option<&str>) {
        let mut i = self.lock();
        match id.and_then(|id| i.library.get(id).map(|p| p.enrich_key())) {
            Some(key) => {
                i.enrich.works.remove(&key);
                i.enrich_attempted.remove(&key);
            }
            None => {
                i.enrich = enrich::Store::default();
                i.enrich_attempted.clear();
            }
        }
        self.save_enrichment(&i);
        drop(i);
        self.enrich_wake.notify_one();
    }
}

async fn enrich_loop(app: AppHandle) {
    let enricher = Enricher::new();
    loop {
        let core = app.state::<Core>();
        let Some(job) = core.next_enrich_job() else {
            {
                let mut i = core.lock();
                if i.enrich_dirty > 0 {
                    core.save_enrichment(&i);
                    i.enrich_dirty = 0;
                }
                i.enrich_busy = false;
            }
            publish(&app);
            tokio::select! {
                _ = core.enrich_wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(600)) => {}
            }
            continue;
        };
        let mut artists = {
            let mut i = core.lock();
            i.enrich_busy = true;
            i.enrich.artists.clone()
        };
        let e = enricher.enrich(&job, &mut artists).await;
        log::debug!("enriched {} ok={} desc={}", job.title, e.ok, e.description.is_some());
        let (showing, flush) = {
            let mut i = core.lock();
            i.enrich.artists = artists;
            i.enrich.works.insert(job.key.clone(), e);
            i.enrich_dirty += 1;
            let showing = i
                .persisted
                .assignments
                .values()
                .filter_map(|a| i.library.get(&a.painting))
                .any(|p| p.enrich_key() == job.key);
            let flush = showing || i.enrich_dirty >= 10;
            if flush {
                core.save_enrichment(&i);
                i.enrich_dirty = 0;
            }
            (showing, flush)
        };
        if showing || flush {
            publish(&app);
        }
        tokio::time::sleep(Duration::from_millis(1100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_kinds() {
        let w = WallpaperSettings::default();
        // 3:2 display; a 1.55 painting crops ~3% → fill.
        assert_eq!(layout_kind(&w, 4096.0, 2642.0, 2560.0, 1707.0), LayoutKind::Cover);
        // 2.96:1 panorama → pan.
        assert_eq!(layout_kind(&w, 6209.0, 2100.0, 2560.0, 1707.0), LayoutKind::Pan);
        let mut w2 = w.clone();
        w2.pan = false;
        assert_eq!(layout_kind(&w2, 6209.0, 2100.0, 2560.0, 1707.0), LayoutKind::Contain);
        w2.fit = FitMode::Fit;
        assert_eq!(layout_kind(&w2, 4096.0, 2642.0, 2560.0, 1707.0), LayoutKind::Contain);
    }

    #[test]
    fn wall_colors_are_dark_and_muted() {
        let c = wall_color("#d4a050");
        let (r, g, b) = settings::parse_hex_color(&c);
        assert!(r < 0.35 && g < 0.35 && b < 0.35, "{c}");
        assert!(r >= g && g >= b, "keeps the hue: {c}");
    }
}
