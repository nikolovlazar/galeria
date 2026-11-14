//! Application state and the logic that ties everything together: library
//! (re)loading, which painting is on which display, rotation, system
//! wallpaper sync and the background enrichment worker.

use crate::displays::{self, DisplayInfo};
use crate::enrich::{self, Enricher, Enrichment, Job};
use crate::host::{self, AppHandle};
use crate::images::{self, Source};
use crate::library::{self, DimCache, Issue, Library, Painting};
use crate::licensing::{self, MuseumPolicy};
use crate::platform::{self, SystemWallpaperOptions};
use crate::settings::{self, FitMode, Matte, Order, ScopeKind, Settings, WallpaperSettings};
use crate::sources;
use crate::styles::{self, CustomStyle};
use crate::tone::{self, Tone, ToneOverride};
use crate::util::now;
use crate::{autostart, download, genres, menu};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::Notify;

/// Files modified more recently than this are assumed to still be downloading.
const FRESH_FILE_GRACE_SECS: u64 = 20;

pub struct Paths {
    pub config: PathBuf,
    /// State and caches of what Galeria looked up; also where the running instance listens.
    pub data: PathBuf,
    pub cache: PathBuf,
    /// Copy of the OS wallpaper configuration from before Galeria changed it (macOS).
    pub wallpaper_backup: PathBuf,
    pub settings: PathBuf,
    pub state: PathBuf,
    pub enrichment: PathBuf,
    pub styles: PathBuf,
    /// Brightness measures Galeria computed for paintings whose manifest has none.
    pub tones: PathBuf,
    /// Record of what Galeria downloaded (lets it re-list entries another tool dropped).
    pub downloads_journal: PathBuf,
    /// Copies of `manifest.json` from before Galeria changed it.
    pub manifest_backups: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Paths, String> {
        // `GALERIA_HOME=<dir>` keeps settings, state and caches in one folder of your
        // choosing (a portable setup, or a sandbox for testing).
        if let Some(home) = std::env::var_os("GALERIA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
        {
            return Paths::in_dir(&home);
        }
        // The same folders Galeria has always used: Application Support and
        // Caches on macOS, AppData on Windows, the XDG folders on Linux.
        let dirs =
            rune::dirs::AppDirs::new(crate::APP_ID).ok_or("could not find your home folder")?;
        Paths::from_dirs(
            dirs.config().to_path_buf(),
            dirs.data().to_path_buf(),
            dirs.cache().to_path_buf(),
        )
    }

    /// Everything under one folder: `<home>/config`, `<home>/data`, `<home>/cache`.
    pub fn in_dir(home: &std::path::Path) -> Result<Paths, String> {
        Paths::from_dirs(home.join("config"), home.join("data"), home.join("cache"))
    }

    fn from_dirs(config: PathBuf, data: PathBuf, cache: PathBuf) -> Result<Paths, String> {
        for d in [&config, &data, &cache] {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        Ok(Paths {
            settings: config.join("settings.json"),
            styles: config.join("plate-styles"),
            state: data.join("state.json"),
            enrichment: data.join("enrichment.json"),
            tones: data.join("tones.json"),
            downloads_journal: data.join("downloads.json"),
            manifest_backups: data.join("manifest-backups"),
            wallpaper_backup: data.join("original-wallpaper-store.plist"),
            config,
            data,
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
    /// The full-screen viewer is up: the menu bar and Dock are tucked away, so the
    /// displays' safe areas are not what they are on the desktop.
    pub viewer_open: bool,
    pub enrich: enrich::Store,
    enrich_dirty: usize,
    enrich_attempted: HashSet<String>,
    pub enrich_busy: bool,
    /// Paintings that failed to decode in a window (id → when).
    pub bad: HashMap<String, u64>,
    /// Paintings Galeria saved itself this run: written whole, so they need no
    /// wait before they can be shown.
    pub complete: std::collections::HashSet<String>,
    pub styles: Vec<CustomStyle>,
    pub quitting: bool,
    /// Brightness of paintings whose manifest does not state it.
    pub tones: tone::Store,
    tones_dirty: usize,
    tones_failed: HashSet<String>,
    /// The library's median brightness: the automatic light/dark dividing line.
    pub auto_threshold: f64,
    /// Whether the system is in its dark appearance (`None` until first asked).
    pub dark: Option<bool>,
    /// `--debug-theme`: pretend the system is light/dark.
    pub dark_forced: Option<bool>,
    last_snapshot: String,
    last_tray: String,
}

pub struct Core {
    pub paths: Paths,
    inner: Mutex<Inner>,
    pub http: sources::Http,
    pub downloads: download::Queue,
    pub tone_wake: Notify,
    pub enrich_wake: Notify,
    watchers: Mutex<Vec<Box<dyn std::any::Any + Send>>>,
    colors: Mutex<HashMap<String, Colors>>,
}

// ---------------------------------------------------------------------------
// Views the windows draw from
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
    /// The image's license requires naming its author: the plate always shows the credit.
    pub attribution_required: bool,
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
    /// Genre ids.
    pub genres: Vec<String>,
    /// Mean luma 0–255 (from the manifest, or measured by Galeria).
    pub brightness: Option<f64>,
    /// Light or dark, after the threshold and any manual choice.
    pub tone: Option<Tone>,
    pub tone_override: Option<ToneOverride>,
    /// Its museum is switched on in Sources.
    pub museum_enabled: bool,
    /// Takes part in the current rotation.
    pub in_rotation: bool,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MuseumSummary {
    pub name: String,
    pub count: usize,
    pub enabled: bool,
    #[serde(flatten)]
    pub policy: MuseumPolicy,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GenreSummary {
    pub id: String,
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToneSummary {
    /// Brightness from which a painting counts as light.
    pub threshold: f64,
    pub auto_threshold: f64,
    pub light: usize,
    pub dark: usize,
    /// Paintings Galeria has not measured yet.
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RotationSummary {
    /// Paintings in the chosen group (before light/dark matching).
    pub count: usize,
    /// Of those, how many suit the current appearance.
    pub matching: usize,
    /// The chosen group has nothing to show, so everything is shown instead.
    pub empty: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSummary {
    pub active: usize,
    pub done: usize,
    pub failed: usize,
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
    /// "light" or "dark": the system appearance Galeria currently sees.
    pub theme: Option<&'static str>,
    pub tone: ToneSummary,
    pub rotation: RotationSummary,
    pub museums: Vec<MuseumSummary>,
    pub genres: Vec<GenreSummary>,
    pub downloads: DownloadSummary,
}

#[derive(Debug, Clone, Default)]
pub struct ManifestIndex {
    /// Every painting id in the library, plus `source:source_id` / `source:accession` aliases.
    pub ids: HashSet<String>,
    /// Paintings per provider source (`"cma"`, `"rijks"`, `"met"`, `"aic"`, …).
    pub by_source: HashMap<String, usize>,
    /// Paintings per `(source, genre_id)`.
    pub by_shelf: HashMap<(String, String), usize>,
    pub total: usize,
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
        let tones: tone::Store = std::fs::read(&paths.tones)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Core {
            inner: Mutex::new(Inner {
                settings,
                library: Library::default(),
                dims: DimCache::default(),
                persisted,
                displays: Vec::new(),
                arrange: false,
                viewer_open: false,
                enrich,
                enrich_dirty: 0,
                enrich_attempted: HashSet::new(),
                enrich_busy: false,
                bad: HashMap::new(),
                complete: Default::default(),
                styles,
                quitting: false,
                tones,
                tones_dirty: 0,
                tones_failed: HashSet::new(),
                auto_threshold: 112.0,
                dark: None,
                dark_forced: None,
                last_snapshot: String::new(),
                last_tray: String::new(),
            }),
            paths,
            http: sources::Http::new(),
            downloads: download::Queue::default(),
            tone_wake: Notify::new(),
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

    /// Fast index of what `manifest.json` currently holds, for Discover and Onboarding.
    pub fn manifest_index(&self) -> ManifestIndex {
        let i = self.lock();
        let mut idx = ManifestIndex {
            total: i.library.paintings.len(),
            ..Default::default()
        };
        for p in &i.library.paintings {
            idx.ids.insert(p.id.clone());
            let inferred_src = p
                .source
                .as_deref()
                .or_else(|| p.id.split_once(':').map(|(s, _)| s));
            if let Some(src) = inferred_src {
                *idx.by_source.entry(src.to_string()).or_default() += 1;
                for g in &p.genres {
                    *idx.by_shelf
                        .entry((src.to_string(), g.clone()))
                        .or_default() += 1;
                }
                if let Some(sid) = &p.source_id {
                    idx.ids.insert(format!("{src}:{sid}"));
                }
                if let Some(acc) = &p.accession {
                    idx.ids.insert(format!("{src}:{acc}"));
                }
            }
        }
        idx
    }

    /// What the museum providers need: the HTTP client, the cache folder and the museums already in the library.
    pub fn source_ctx(&self, app: &AppHandle) -> sources::Ctx {
        let mut museums: Vec<String> = self
            .lock()
            .library
            .paintings
            .iter()
            .filter_map(|p| p.museum.clone())
            .collect();
        museums.sort();
        museums.dedup();
        sources::Ctx {
            http: self.http.clone(),
            cache: self.paths.cache.clone(),
            app: Some(app.clone()),
            museums,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot_of(&self.lock(), &self.paths, self.downloads.counts())
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
        let c = Colors {
            wall: wall_color(&average),
            average,
        };
        self.colors
            .lock()
            .unwrap()
            .insert(id.to_string(), c.clone());
        Ok(c)
    }
}

fn view(i: &Inner, p: &Painting) -> PaintingView {
    let e: Option<&Enrichment> = i.enrich.works.get(&p.enrich_key());
    let fill = |m: &Option<String>, f: &dyn Fn(&Enrichment) -> Option<String>| {
        m.clone().or_else(|| e.and_then(f))
    };
    let (title, original_title) = match e.and_then(|e| e.title_en.clone()) {
        Some(en) if !en.eq_ignore_ascii_case(&p.title) => (en, Some(p.title.clone())),
        _ => (p.title.clone(), None),
    };
    let (description, description_source, description_license, description_lang) =
        match &p.description {
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
        attribution_required: p.attribution_required,
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
        genres: p.genres.clone(),
        brightness: brightness_of(i, p),
        tone: tone_of(i, p),
        tone_override: i.settings.appearance.overrides.get(&p.id).copied(),
        museum_enabled: museum_enabled(i, p),
        in_rotation: allowed(i, p) && in_scope(i, p),
        extra: p.extra.clone(),
    }
}

// ---------------------------------------------------------------------------
// Groups: museums, genres, playlists, light & dark
// ---------------------------------------------------------------------------

fn museum_enabled(i: &Inner, p: &Painting) -> bool {
    p.museum.as_ref().is_none_or(|m| {
        i.settings
            .sources
            .disabled_museums
            .binary_search(m)
            .is_err()
    })
}

/// Not hidden by the user and not from a switched-off museum.
fn allowed(i: &Inner, p: &Painting) -> bool {
    i.settings.excluded.binary_search(&p.id).is_err() && museum_enabled(i, p)
}

/// Whether a painting belongs to the group the user chose to rotate through.
fn in_scope(i: &Inner, p: &Painting) -> bool {
    let s = &i.settings.rotation.scope;
    match s.kind {
        ScopeKind::All => true,
        ScopeKind::Favorites => i.settings.favorites.binary_search(&p.id).is_ok(),
        ScopeKind::Playlist => s
            .playlist
            .as_ref()
            .and_then(|id| i.settings.playlists.iter().find(|pl| &pl.id == id))
            .is_some_and(|pl| pl.items.contains(&p.id)),
        ScopeKind::Filter => {
            let museum =
                s.museums.is_empty() || p.museum.as_ref().is_some_and(|m| s.museums.contains(m));
            let genre = s.genres.is_empty()
                || p.genres.iter().any(|g| s.genres.contains(g))
                || (p.genres.is_empty() && s.genres.iter().any(|g| g == genres::OTHER));
            museum && genre
        }
        ScopeKind::Selection => s.selection.contains(&p.id),
    }
}

/// The scope to apply: the chosen one, or everything when it would leave nothing to show.
fn scope_usable(i: &Inner) -> bool {
    i.settings.rotation.scope.kind == ScopeKind::All
        || i.library
            .paintings
            .iter()
            .any(|p| allowed(i, p) && in_scope(i, p))
}

fn tone_key(p: &Painting) -> String {
    format!("{}:{}:{}", p.file, p.bytes, p.modified)
}

fn brightness_of(i: &Inner, p: &Painting) -> Option<f64> {
    p.brightness.or_else(|| {
        i.tones
            .get(&p.id)
            .filter(|c| c.key == tone_key(p))
            .map(|c| c.measure.brightness)
    })
}

fn threshold(i: &Inner) -> f64 {
    i.settings.appearance.threshold.unwrap_or(i.auto_threshold)
}

fn tone_of(i: &Inner, p: &Painting) -> Option<Tone> {
    match i.settings.appearance.overrides.get(&p.id) {
        Some(ToneOverride::Light) => Some(Tone::Light),
        Some(ToneOverride::Dark) => Some(Tone::Dark),
        Some(ToneOverride::Both) => None,
        None => brightness_of(i, p).map(|b| tone::classify(b, threshold(i))),
    }
}

fn is_dark(i: &Inner) -> Option<bool> {
    i.dark_forced.or(i.dark)
}

/// Does the painting suit the system's current appearance? Unmeasured paintings
/// and those marked "both" suit either.
fn suits_appearance(i: &Inner, p: &Painting) -> bool {
    if !i.settings.appearance.match_system {
        return true;
    }
    match (is_dark(i), tone_of(i, p)) {
        (Some(dark), Some(t)) => (t == Tone::Dark) == dark,
        _ => true,
    }
}

fn recompute_threshold(i: &mut Inner) {
    let mut values: Vec<f64> = i
        .library
        .paintings
        .iter()
        .filter(|p| allowed(i, p))
        .filter_map(|p| brightness_of(i, p))
        .collect();
    i.auto_threshold = tone::auto_threshold(&mut values);
}

fn museum_summaries(i: &Inner) -> Vec<MuseumSummary> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in &i.library.paintings {
        if let Some(m) = p.museum.as_deref() {
            *counts.entry(m).or_default() += 1;
        }
    }
    let mut out: Vec<MuseumSummary> = counts
        .into_iter()
        .map(|(name, count)| MuseumSummary {
            enabled: i
                .settings
                .sources
                .disabled_museums
                .binary_search_by(|d| d.as_str().cmp(name))
                .is_err(),
            policy: licensing::museum_policy(name),
            name: name.to_string(),
            count,
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    out
}

fn genre_summaries(i: &Inner) -> Vec<GenreSummary> {
    let mut out: Vec<GenreSummary> = genres::all()
        .iter()
        .map(|g| GenreSummary {
            count: i
                .library
                .paintings
                .iter()
                .filter(|p| p.genres.contains(&g.id))
                .count(),
            id: g.id.clone(),
            name: g.name.clone(),
        })
        .filter(|g| g.count > 0)
        .collect();
    let other = i
        .library
        .paintings
        .iter()
        .filter(|p| p.genres.is_empty())
        .count();
    if other > 0 {
        out.push(GenreSummary {
            id: genres::OTHER.into(),
            name: genres::name(genres::OTHER).into(),
            count: other,
        });
    }
    out
}

fn snapshot_of(i: &Inner, paths: &Paths, downloads: (usize, usize, usize)) -> Snapshot {
    let interval = i.settings.rotation.interval_minutes as u64 * 60;
    let displays = i
        .displays
        .iter()
        .map(|d| {
            let a = i.persisted.assignments.get(&d.key);
            let painting = a
                .and_then(|a| i.library.get(&a.painting))
                .map(|p| view(i, p));
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
            eligible: i
                .library
                .paintings
                .iter()
                .filter(|p| i.settings.excluded.binary_search(&p.id).is_err())
                .count(),
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
        builtin_styles: styles::BUILTIN
            .iter()
            .map(|(id, name)| BuiltinStyle { id, name })
            .collect(),
        custom_styles: i.styles.clone(),
        styles_dir: paths.styles.clone(),
        platform: platform::PLATFORM,
        live_supported: platform::live_wallpaper_supported(),
        version: env!("CARGO_PKG_VERSION"),
        original_wallpapers: if paths.wallpaper_backup.exists() {
            i.displays.len()
        } else {
            i.persisted
                .original_wallpapers
                .values()
                .filter(|p| p.is_file())
                .count()
        },
        theme: is_dark(i).map(|d| if d { "dark" } else { "light" }),
        tone: {
            let tones: Vec<Option<Tone>> = i
                .library
                .paintings
                .iter()
                .filter(|p| allowed(i, p))
                .map(|p| tone_of(i, p))
                .collect();
            ToneSummary {
                threshold: threshold(i),
                auto_threshold: i.auto_threshold,
                light: tones.iter().filter(|t| **t == Some(Tone::Light)).count(),
                dark: tones.iter().filter(|t| **t == Some(Tone::Dark)).count(),
                unknown: i
                    .library
                    .paintings
                    .iter()
                    .filter(|p| allowed(i, p) && brightness_of(i, p).is_none())
                    .count(),
            }
        },
        rotation: {
            let usable = scope_usable(i);
            let group: Vec<&Painting> = i
                .library
                .paintings
                .iter()
                .filter(|p| allowed(i, p) && (!usable || in_scope(i, p)))
                .collect();
            RotationSummary {
                count: group.len(),
                matching: group.iter().filter(|p| suits_appearance(i, p)).count(),
                empty: !usable,
            }
        },
        museums: museum_summaries(i),
        genres: genre_summaries(i),
        downloads: DownloadSummary {
            active: downloads.0,
            done: downloads.1,
            failed: downloads.2,
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
        s = if l > 0.5 {
            d / (2.0 - max - min)
        } else {
            d / (max + min)
        };
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
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
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

fn eligible(
    i: &Inner,
    p: &Painting,
    display: Option<&DisplayInfo>,
    strict: bool,
    scoped: bool,
) -> bool {
    if !allowed(i, p) || (scoped && !in_scope(i, p)) {
        return false;
    }
    if strict && !suits_appearance(i, p) {
        return false;
    }
    if let Some(&t) = i.bad.get(&p.id) {
        if now().saturating_sub(t) < 3600 {
            return false;
        }
    }
    if now().saturating_sub(p.modified) < FRESH_FILE_GRACE_SECS && !i.complete.contains(&p.id) {
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
            let current = i
                .persisted
                .deck
                .get(i.persisted.deck_pos.saturating_sub(1))
                .cloned();
            i.persisted.deck = ids.clone();
            i.persisted.deck_pos = current
                .and_then(|c| ids.iter().position(|id| *id == c))
                .map(|p| p + 1)
                .unwrap_or(0);
        }
        Order::Shuffle => {
            let pos = i.persisted.deck_pos.min(i.persisted.deck.len());
            let removed_before = i.persisted.deck[..pos]
                .iter()
                .filter(|id| !known.contains(id))
                .count();
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
    let current = i
        .persisted
        .assignments
        .get(display_key)
        .map(|a| a.painting.clone());
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
    // The chosen group applies unless it has nothing to show at all.
    let scoped = scope_usable(i);
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
                let ok = i
                    .library
                    .get(&id)
                    .is_some_and(|p| eligible(i, p, display.as_ref(), strict, scoped))
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
            let p = primary
                .as_ref()
                .and_then(|k| i.persisted.assignments.get(k))
                .map(|a| a.painting.clone());
            if let Some(p) = p {
                let cur = i.persisted.assignments.get(key).map(|a| a.painting.clone());
                if cur.as_deref() != Some(p.as_str()) && !p.is_empty() {
                    assign(i, key, p, true);
                    changed.push(key.clone());
                }
            }
            continue;
        }
        let scoped = scope_usable(i);
        let valid = i
            .persisted
            .assignments
            .get(key)
            .and_then(|a| i.library.get(&a.painting))
            .is_some_and(|p| allowed(i, p) && (!scoped || in_scope(i, p)));
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
// Actions (all take the app handle; none hold the lock while calling the shell)
// ---------------------------------------------------------------------------

/// Hand the snapshot to every window and refresh the menu bar item, but
/// only when something actually changed.
pub fn publish(app: &AppHandle) {
    let core = app.core();
    let downloads = core.downloads.counts();
    let (snap, changed, tray_changed) = {
        let mut i = core.lock();
        let snap = snapshot_of(&i, &core.paths, downloads);
        let json = serde_json::to_string(&snap).unwrap_or_default();
        let changed = json != i.last_snapshot;
        if changed {
            i.last_snapshot = json;
        }
        let sig = menu::signature(&snap);
        let tray_changed = sig != i.last_tray;
        if tray_changed {
            i.last_tray = sig;
        }
        (snap, changed, tray_changed)
    };
    let Some(shell) = app.shell() else { return };
    if tray_changed {
        shell.menu(&snap);
    }
    if changed {
        shell.snapshot(Arc::new(snap));
    }
}

pub fn start(app: &AppHandle) {
    refresh_displays(app, true);
    app.core().lock().dark = platform::system_dark();
    reload_library(app);
    restart_watchers(app);
    reconcile_windows(app);
    publish(app);

    let a = app.clone();
    host::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            n += 1;
            rotation_tick(&a);
            let a2 = a.clone();
            let _ = host::spawn_blocking(move || {
                if let Some(dark) = platform::system_dark() {
                    set_dark(&a2, dark);
                }
                if n.is_multiple_of(2) {
                    refresh_displays(&a2, false);
                }
            })
            .await;
        }
    });
    host::spawn(enrich_loop(app.clone()));
    host::spawn(tone_loop(app.clone()));
    host::spawn(download::worker(app.clone()));
}

// ---------------------------------------------------------------------------
// Light & dark
// ---------------------------------------------------------------------------

/// The system switched appearance (or Galeria just found out which one it is in).
pub fn set_dark(app: &AppHandle, dark: bool) {
    let core = app.core();
    {
        let mut i = core.lock();
        if i.dark == Some(dark) {
            return;
        }
        log::info!("system appearance: {}", if dark { "dark" } else { "light" });
        i.dark = Some(dark);
    }
    match_appearance(app);
    publish(app);
}

/// One line about what is showing and why, for `galeria --debug`.
pub fn debug_summary(app: &AppHandle) -> String {
    let core = app.core();
    let i = core.lock();
    let showing: Vec<Value> = i
        .displays
        .iter()
        .filter_map(|d| {
            i.persisted
                .assignments
                .get(&d.key)
                .and_then(|a| i.library.get(&a.painting))
        })
        .map(|p| {
            serde_json::json!({
                "id": p.id, "museum": p.museum, "genres": p.genres,
                "brightness": brightness_of(&i, p), "tone": tone_of(&i, p),
                "inScope": in_scope(&i, p), "suitsAppearance": suits_appearance(&i, p),
            })
        })
        .collect();
    let scoped = scope_usable(&i);
    let group: Vec<&Painting> = i
        .library
        .paintings
        .iter()
        .filter(|p| allowed(&i, p) && (!scoped || in_scope(&i, p)))
        .collect();
    serde_json::json!({
        "dark": is_dark(&i), "matchSystem": i.settings.appearance.match_system, "threshold": threshold(&i),
        "scope": i.settings.rotation.scope.kind, "inRotation": group.len(),
        "suited": group.iter().filter(|p| suits_appearance(&i, p)).count(),
        "disabledMuseums": i.settings.sources.disabled_museums, "showing": showing,
    })
    .to_string()
}

/// `galeria --theme light|dark|auto`: pretend, for testing.
pub fn force_dark(app: &AppHandle, dark: Option<bool>) {
    app.core().lock().dark_forced = dark;
    match_appearance(app);
    publish(app);
}

/// Replace paintings that do not suit the current appearance (when the user asked for that).
pub fn match_appearance(app: &AppHandle) {
    let core = app.core();
    let stale: Vec<String> = {
        let i = core.lock();
        let a = &i.settings.appearance;
        if !a.match_system || !a.switch_immediately || is_dark(&i).is_none() {
            return;
        }
        i.displays
            .iter()
            .filter(|d| {
                i.persisted
                    .assignments
                    .get(&d.key)
                    .and_then(|a| i.library.get(&a.painting))
                    .is_some_and(|p| !suits_appearance(&i, p))
            })
            .map(|d| d.key.clone())
            .collect()
    };
    for key in stale {
        // `advance` prefers paintings that suit the appearance and falls back when none do.
        advance(app, Some(key), true);
    }
}

async fn tone_loop(app: AppHandle) {
    loop {
        let core = app.core();
        // One painting at a time: decoding a 40-megapixel scan is the expensive part.
        let job = {
            let i = core.lock();
            i.library
                .paintings
                .iter()
                .find(|p| brightness_of(&i, p).is_none() && !i.tones_failed.contains(&p.id))
                .map(|p| {
                    (
                        p.id.clone(),
                        tone_key(p),
                        Source {
                            path: p.path.clone(),
                            width: p.width,
                            height: p.height,
                            bytes: p.bytes,
                            modified: p.modified,
                        },
                    )
                })
        };
        let Some((id, key, src)) = job else {
            {
                let mut i = core.lock();
                if i.tones_dirty > 0 {
                    i.tones_dirty = 0;
                    let _ = settings::write_json_atomic(&core.paths.tones, &i.tones);
                    recompute_threshold(&mut i);
                }
            }
            publish(&app);
            core.tone_wake.notified().await;
            continue;
        };
        let cache = core.paths.cache.clone();
        let measured = host::spawn_blocking(move || {
            let thumb = images::thumb_cached(&src, &cache)?;
            let img = image::open(&thumb).map_err(|e| e.to_string())?.into_rgb8();
            Ok::<_, String>(tone::measure(&img))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        {
            let mut i = core.lock();
            match measured {
                Ok(measure) => {
                    i.tones.insert(id, tone::Cached { key, measure });
                    i.tones_dirty += 1;
                    if i.tones_dirty >= 25 {
                        i.tones_dirty = 1;
                        let _ = settings::write_json_atomic(&core.paths.tones, &i.tones);
                        recompute_threshold(&mut i);
                    }
                }
                Err(e) => {
                    log::debug!("could not measure {id}: {e}");
                    i.tones_failed.insert(id);
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

pub fn refresh_displays(app: &AppHandle, initial: bool) {
    // Measured while the viewer hides the menu bar, the desktop would be laid out
    // without room for it; look again once the viewer is gone.
    if !initial && app.core().lock().viewer_open {
        return;
    }
    let list = displays::list(app);
    if list.is_empty() {
        return;
    }
    let core = app.core();
    let changed = {
        let mut i = core.lock();
        if i.displays == list {
            return;
        }
        log::info!(
            "displays: {:?}",
            list.iter()
                .map(|d| format!("{} {}x{}@{}", d.name, d.width, d.height, d.scale))
                .collect::<Vec<_>>()
        );
        log::debug!(
            "display insets: {:?}",
            list.iter().map(|d| &d.insets).collect::<Vec<_>>()
        );
        i.displays = list;
        let changed = ensure_assignments(&mut i);
        core.save_state(&i);
        changed
    };
    if !initial {
        reconcile_windows(app);
        publish(app);
        if !changed.is_empty() {
            sync_system_wallpaper(app, Some(changed));
        }
    }
}

pub fn reload_library(app: &AppHandle) {
    let core = app.core();
    let (root, mut dims) = {
        let mut i = core.lock();
        (i.settings.library_path.clone(), std::mem::take(&mut i.dims))
    };
    if let Some(r) = &root {
        download::sweep(r);
        // Paintings Galeria downloaded stay listed even if another tool rewrote the manifest.
        download::heal(
            r,
            &core.paths.downloads_journal,
            &core.paths.manifest_backups,
        );
    }
    let lib = match &root {
        Some(r) => library::load(r, &mut dims),
        None => Library::default(),
    };
    let changed = {
        let mut i = core.lock();
        i.dims = dims;
        let same_root = i.library.root.as_os_str() == lib.root.as_os_str();
        if lib.report.error.is_some()
            && lib.report.manifest_found
            && same_root
            && !i.library.paintings.is_empty()
        {
            // The manifest is probably mid-write; keep the last good copy.
            log::warn!(
                "manifest unreadable, keeping previous library: {:?}",
                lib.report.error
            );
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
        // Forget measures of files that changed or left the library.
        let keys: HashMap<&str, String> = i
            .library
            .paintings
            .iter()
            .map(|p| (p.id.as_str(), tone_key(p)))
            .collect();
        let stale: Vec<String> = i
            .tones
            .iter()
            .filter(|(id, c)| keys.get(id.as_str()) != Some(&c.key))
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            i.tones.remove(&id);
        }
        recompute_threshold(&mut i);
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
    core.tone_wake.notify_one();
}

pub fn reload_styles(app: &AppHandle) {
    let core = app.core();
    let styles = styles::load(&core.paths.styles);
    core.lock().styles = styles;
    publish(app);
}

pub fn restart_watchers(app: &AppHandle) {
    use notify::RecursiveMode;
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};

    let core = app.core();
    let root = core.lock().settings.library_path.clone();
    let mut watchers = core.watchers.lock().unwrap();
    watchers.clear();

    if let Some(root) = root.filter(|r| r.is_dir()) {
        let a = app.clone();
        let res = new_debouncer(
            Duration::from_millis(1500),
            None,
            move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                let relevant = events.iter().any(|e| {
                    !matches!(e.event.kind, notify::EventKind::Access(_))
                        && e.event.paths.iter().any(|p| {
                            p.file_name().is_some_and(|n| n == library::MANIFEST)
                                || library::is_image(p)
                        })
                });
                if relevant {
                    let a = a.clone();
                    host::spawn_blocking(move || reload_library(&a));
                }
            },
        );
        match res {
            Ok(mut d) => match d.watch(&root, RecursiveMode::Recursive) {
                Ok(()) => watchers.push(Box::new(d)),
                Err(e) => log::warn!("cannot watch {}: {e}", root.display()),
            },
            Err(e) => log::warn!("file watcher unavailable: {e}"),
        }
    }

    let a = app.clone();
    if let Ok(mut d) = new_debouncer(
        Duration::from_millis(400),
        None,
        move |res: DebounceEventResult| {
            if res.is_ok() {
                reload_styles(&a);
            }
        },
    ) {
        if d.watch(&core.paths.styles, RecursiveMode::NonRecursive)
            .is_ok()
        {
            watchers.push(Box::new(d));
        }
    }
}

fn rotation_tick(app: &AppHandle) {
    let core = app.core();
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
    let core = app.core();
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
    let core = app.core();
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
    let core = app.core();
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
        if let Err(e) = autostart::set(new.general.launch_at_login) {
            log::warn!("autostart: {e}");
        }
    }
    if old.wallpaper.live != new.wallpaper.live {
        if !new.wallpaper.live {
            app.core().lock().arrange = false;
        }
        reconcile_windows(app);
    }
    let core = app.core();
    let mut reassigned = Vec::new();
    // Anything that changes which paintings may be shown.
    let group_changed = old.excluded != new.excluded
        || old.sources != new.sources
        || old.rotation.scope != new.rotation.scope
        || old.playlists != new.playlists
        || (new.rotation.scope.kind == ScopeKind::Favorites && old.favorites != new.favorites);
    if group_changed
        || old.wallpaper.same_on_all_displays != new.wallpaper.same_on_all_displays
        || old.wallpaper.match_orientation != new.wallpaper.match_orientation
    {
        let mut i = core.lock();
        if old.excluded != new.excluded || old.sources != new.sources {
            recompute_threshold(&mut i);
        }
        reassigned = ensure_assignments(&mut i);
        core.save_state(&i);
    }
    if old.appearance != new.appearance {
        match_appearance(app);
    }
    let w0 = &old.wallpaper;
    let w1 = &new.wallpaper;
    let affects_static = (!w0.sync_system && w1.sync_system)
        || w0.fit != w1.fit
        || w0.matte != w1.matte
        || w0.matte_color != w1.matte_color
        || w0.crop_tolerance != w1.crop_tolerance
        || w0.pan != w1.pan;
    if w1.screen_saver && (!w0.screen_saver || old.library_path != new.library_path) {
        sync_screen_saver(app);
    }
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
    if let Some(shell) = app.shell() {
        shell.set_dock_icon(show);
    }
}

/// One wallpaper window per display while live mode is on, none otherwise.
pub fn reconcile_windows(app: &AppHandle) {
    if let Some(shell) = app.shell() {
        shell.reconcile_windows();
    }
}

/// Whether the painting is drawn in desktop-level windows (rather than only set as the system wallpaper).
pub fn live(i: &Inner) -> bool {
    i.settings.wallpaper.live && platform::live_wallpaper_supported()
}

/// Arrange mode lifts the wallpaper windows above the desktop icons so the
/// plate can be dragged; leaving it drops them back behind everything.
pub fn set_arrange(app: &AppHandle, on: bool) {
    {
        let mut i = app.core().lock();
        if on && !live(&i) {
            return;
        }
        if i.arrange == on {
            return;
        }
        i.arrange = on;
    }
    if let Some(shell) = app.shell() {
        shell.arrange_changed(on);
    }
    publish(app);
}

/// Quit, putting the user's own wallpaper back first when they asked for that.
pub fn quit(app: &AppHandle) {
    let restore = {
        let mut i = app.core().lock();
        i.quitting = true;
        i.settings.general.restore_wallpaper_on_quit
    };
    if restore {
        restore_original_wallpaper(app);
    }
    match app.shell() {
        Some(shell) => shell.quit(),
        None => std::process::exit(0),
    }
}

pub fn set_plate_position(app: &AppHandle, scope: &str, display: Option<String>, x: f64, y: f64) {
    let patch = match (scope, display) {
        ("viewer", _) => serde_json::json!({ "viewer": { "platePosition": { "x": x, "y": y } } }),
        (_, Some(key)) => {
            serde_json::json!({ "plate": { "positions": { key: { "x": x, "y": y } } } })
        }
        (_, None) => return set_plate_preset(app, x, y),
    };
    if let Err(e) = patch_settings(app, &patch) {
        log::warn!("plate position: {e}");
    }
}

/// Snap every display's plate to a preset corner.
pub fn set_plate_preset(app: &AppHandle, x: f64, y: f64) {
    let core = app.core();
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
    let mut list = app.core().lock().settings.excluded.clone();
    list.retain(|x| x != id);
    if excluded {
        list.push(id.to_string());
    }
    let _ = patch_settings(app, &serde_json::json!({ "excluded": list }));
}

pub fn set_favorite(app: &AppHandle, id: &str, favorite: bool) {
    let mut list = app.core().lock().settings.favorites.clone();
    list.retain(|x| x != id);
    if favorite {
        list.push(id.to_string());
    }
    let _ = patch_settings(app, &serde_json::json!({ "favorites": list }));
}

/// Scrub deleted painting ids from settings,playlists, overrides and display history.
pub fn forget_deleted_ids(app: &AppHandle, ids: &std::collections::HashSet<String>) {
    if ids.is_empty() {
        return;
    }
    let core = app.core();
    let to_save = {
        let mut i = core.lock();
        for id in ids {
            i.bad.remove(id);
            i.complete.remove(id);
        }
        for a in i.persisted.assignments.values_mut() {
            a.history.retain(|id| !ids.contains(id));
        }
        let s = &mut i.settings;
        let before = (
            s.excluded.len(),
            s.favorites.len(),
            s.rotation.scope.selection.len(),
            s.appearance.overrides.len(),
            s.playlists.iter().map(|p| p.items.len()).sum::<usize>(),
        );
        s.excluded.retain(|id| !ids.contains(id));
        s.favorites.retain(|id| !ids.contains(id));
        s.rotation.scope.selection.retain(|id| !ids.contains(id));
        s.appearance.overrides.retain(|id, _| !ids.contains(id));
        for p in &mut s.playlists {
            p.items.retain(|id| !ids.contains(id));
        }
        let after = (
            s.excluded.len(),
            s.favorites.len(),
            s.rotation.scope.selection.len(),
            s.appearance.overrides.len(),
            s.playlists.iter().map(|p| p.items.len()).sum::<usize>(),
        );
        (before != after).then(|| s.clone())
    };
    if let Some(s) = to_save {
        let _ = s.save(&core.paths.settings);
    }
}

/// A window could not decode a painting: skip it for an hour and move on.
pub fn report_bad_image(app: &AppHandle, id: &str, display: Option<String>) {
    let core = app.core();
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

/// Point the screen saver at the art folder itself: no copies, no second folder.
/// Does nothing unless the setting is on.
pub fn sync_screen_saver(app: &AppHandle) {
    if !platform::screen_saver_supported() {
        return;
    }
    // A sandboxed run (`GALERIA_HOME`) or a test must not change the real screen saver.
    if cfg!(test) || std::env::var_os("GALERIA_HOME").is_some_and(|v| !v.is_empty()) {
        return;
    }
    let app = app.clone();
    host::spawn_blocking(move || {
        let core = app.core();
        let folder = {
            let i = core.lock();
            if !i.settings.wallpaper.screen_saver {
                return;
            }
            i.settings.library_path.clone()
        };
        let Some(folder) = folder.filter(|f| f.is_dir()) else {
            return;
        };
        // Before the first change, keep what the user had (the same copy "Restore" uses).
        platform::backup_wallpaper_store(&core.paths.wallpaper_backup);
        match platform::set_screen_saver(&folder) {
            Ok(()) => log::info!("screen saver shows {}", folder.display()),
            Err(e) => log::warn!("screen saver: {e}"),
        }
    });
}

/// Set the OS wallpaper for the given displays (`None` = all) to their current painting.
pub fn sync_system_wallpaper(app: &AppHandle, keys: Option<Vec<String>>) {
    let app = app.clone();
    host::spawn(async move {
        let core = app.core();
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
                    let kind = layout_kind(
                        &i.settings.wallpaper,
                        p.width as f64,
                        p.height as f64,
                        d.width,
                        d.height,
                    );
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
            (
                i.settings.wallpaper.matte,
                i.settings.wallpaper.matte_color.clone(),
                i.settings.library_path.clone(),
            )
        };
        for (d, id, src, fill) in jobs {
            let color_hex = match matte {
                Matte::Color => matte_color.clone(),
                _ => {
                    let a = app.clone();
                    let id2 = id.clone();
                    host::spawn_blocking(move || a.core().colors_for(&id2))
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .map(|c| c.wall)
                        .unwrap_or_else(|| matte_color.clone())
                }
            };
            let opts = SystemWallpaperOptions {
                fill,
                color: settings::parse_hex_color(&color_hex),
            };

            // Remember what the user had before Galeria touched it.
            let known = core
                .lock()
                .persisted
                .original_wallpapers
                .contains_key(&d.key);
            if !known || !core.paths.wallpaper_backup.exists() {
                let d2 = d.clone();
                let a = app.clone();
                let original = host::spawn_blocking(move || {
                    a.on_main(move || platform::current_system_wallpaper(&d2))
                        .flatten()
                })
                .await
                .ok()
                .flatten();
                // Aerial/dynamic wallpapers may report no URL at all; that still isn't ours.
                let ours = original
                    .as_ref()
                    .is_some_and(|o| root.as_ref().is_some_and(|r| o.starts_with(r)));
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
            let res = host::spawn_blocking(move || {
                a.on_main(move || platform::set_system_wallpaper(&d, &path, opts))
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
    let core = app.core();
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
            .filter_map(|d| {
                i.persisted
                    .original_wallpapers
                    .get(&d.key)
                    .map(|p| (d.clone(), p.clone()))
            })
            .filter(|(_, p)| p.is_file())
            .collect()
    };
    for (d, path) in jobs {
        let opts = SystemWallpaperOptions {
            fill: true,
            color: (0.0, 0.0, 0.0),
        };
        let r = app.on_main(move || platform::set_system_wallpaper(&d, &path, opts));
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
        let showing: Vec<String> = i
            .persisted
            .assignments
            .values()
            .map(|a| a.painting.clone())
            .collect();
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
            // The Rijksmuseum's linked-data id may be in `persistent_url` rather than `source_page`.
            let persistent = p
                .extra
                .get("persistent_url")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let job = Job {
                key: key.clone(),
                source: p.source.clone(),
                source_id: p.source_id.clone(),
                source_page: persistent
                    .filter(|u| u.contains("id.rijksmuseum.nl"))
                    .or_else(|| p.source_page.clone()),
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
        let core = app.core();
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
        log::debug!(
            "enriched {} ok={} desc={}",
            job.title,
            e.ok,
            e.description.is_some()
        );
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
impl PaintingView {
    /// A landscape painting with nothing but a title, for tests to fill in.
    pub fn sample() -> PaintingView {
        PaintingView {
            id: "test:1".into(),
            file: "sample.jpg".into(),
            title: "Sample".into(),
            original_title: None,
            artist: None,
            artist_bio: None,
            date: None,
            medium: None,
            dimensions: None,
            description: None,
            description_source: None,
            description_license: None,
            description_lang: None,
            did_you_know: None,
            culture: None,
            museum: None,
            credit: None,
            credit_line: None,
            accession: None,
            license: None,
            attribution_required: false,
            source: None,
            source_page: None,
            wikipedia_url: None,
            tags: Vec::new(),
            kind: None,
            width: 4000,
            height: 3000,
            aspect: 1.333,
            orientation: "landscape",
            bytes: 1,
            excluded: false,
            favorite: false,
            enriched: false,
            genres: Vec::new(),
            brightness: None,
            tone: None,
            tone_override: None,
            museum_enabled: true,
            in_rotation: true,
            extra: Map::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_kinds() {
        let w = WallpaperSettings::default();
        // 3:2 display; a 1.55 painting crops ~3% → fill.
        assert_eq!(
            layout_kind(&w, 4096.0, 2642.0, 2560.0, 1707.0),
            LayoutKind::Cover
        );
        // 2.96:1 panorama → pan.
        assert_eq!(
            layout_kind(&w, 6209.0, 2100.0, 2560.0, 1707.0),
            LayoutKind::Pan
        );
        let mut w2 = w.clone();
        w2.pan = false;
        assert_eq!(
            layout_kind(&w2, 6209.0, 2100.0, 2560.0, 1707.0),
            LayoutKind::Contain
        );
        w2.fit = FitMode::Fit;
        assert_eq!(
            layout_kind(&w2, 4096.0, 2642.0, 2560.0, 1707.0),
            LayoutKind::Contain
        );
    }

    #[test]
    fn wall_colors_are_dark_and_muted() {
        let c = wall_color("#d4a050");
        let (r, g, b) = settings::parse_hex_color(&c);
        assert!(r < 0.35 && g < 0.35 && b < 0.35, "{c}");
        assert!(r >= g && g >= b, "keeps the hue: {c}");
    }
}
