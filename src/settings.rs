//! User settings. Persisted as JSON in the app config dir. Every field has a
//! default so older/newer settings files always deserialize.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Folder that contains `manifest.json` and the image files.
    pub library_path: Option<PathBuf>,
    pub wallpaper: WallpaperSettings,
    pub rotation: RotationSettings,
    pub plate: PlateSettings,
    pub viewer: ViewerSettings,
    pub general: GeneralSettings,
    pub update: crate::update::Settings,
    /// Painting ids that should never be shown.
    pub excluded: Vec<String>,
    pub favorites: Vec<String>,
    pub sources: SourcesSettings,
    /// User-made groups of paintings ("Favorites" is built in and lives in `favorites`).
    pub playlists: Vec<Playlist>,
    pub appearance: AppearanceSettings,
    pub download: DownloadSettings,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SourcesSettings {
    /// Museums (as named in the manifest) whose paintings are left out of the rotation.
    pub disabled_museums: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub items: Vec<String>,
}

/// Which paintings take part in the rotation.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScopeKind {
    #[default]
    All,
    Favorites,
    Playlist,
    /// Chosen museums and/or genres.
    Filter,
    /// An ad-hoc set picked in Browse.
    Selection,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Scope {
    pub kind: ScopeKind,
    pub playlist: Option<String>,
    /// Empty = any museum.
    pub museums: Vec<String>,
    /// Genre ids; empty = any genre.
    pub genres: Vec<String>,
    pub selection: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceSettings {
    /// Light paintings in the system's light appearance, dark paintings in dark.
    pub match_system: bool,
    /// Brightness (0–255) from which a painting counts as light. `None` = the
    /// library's median, so each appearance gets about half.
    pub threshold: Option<f64>,
    /// Manual light/dark/both choices per painting id.
    pub overrides: HashMap<String, crate::tone::ToneOverride>,
    /// Change the painting right away when the appearance flips.
    pub switch_immediately: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            match_system: false,
            threshold: None,
            overrides: HashMap::new(),
            switch_immediately: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadSettings {
    /// Larger originals are scaled down to this long edge when saved.
    pub max_long_edge: u32,
    /// Discover: only offer landscape-format works.
    pub landscape_only: bool,
    /// Discover: hide images whose long edge is smaller than this.
    pub min_long_edge: u32,
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            max_long_edge: 8000,
            landscape_only: true,
            min_long_edge: 3000,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FitMode {
    /// Fill when the crop is small, otherwise pan (or fit when panning is off).
    Auto,
    /// Always show the entire painting on a wall.
    Fit,
    /// Always fill the screen, cropping the overflow.
    Fill,
    /// Always fill and slowly pan across the overflow.
    Pan,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Matte {
    /// A blurred, darkened copy of the painting.
    Blur,
    /// A fixed wall colour.
    Color,
    /// The painting's average colour, darkened (a wall tinted to the work).
    Average,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FrameStyle {
    None,
    Black,
    White,
    Gilded,
    Walnut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WallpaperSettings {
    /// Render the painting in a desktop-level window (animation + info plate).
    pub live: bool,
    /// Also set the operating system's wallpaper to the current painting.
    pub sync_system: bool,
    /// Also point the system screen saver at the painting on each display (macOS).
    pub screen_saver: bool,
    pub fit: FitMode,
    /// In auto mode, the largest fraction of the painting that may be cropped
    /// before Galeria switches to panning (or fitting).
    pub crop_tolerance: f64,
    /// Pan across paintings whose aspect ratio is far from the display's.
    pub pan: bool,
    /// Panning speed in logical pixels per second.
    pub pan_speed: f64,
    /// Seconds to rest at each edge before reversing.
    pub pan_pause: f64,
    pub pan_paused: bool,
    pub matte: Matte,
    pub matte_color: String,
    pub frame: FrameStyle,
    /// Margin around fitted paintings, as a fraction of the screen.
    pub fit_margin: f64,
    pub shadow: bool,
    /// Gallery spotlight vignette on the wall.
    pub spotlight: bool,
    pub same_on_all_displays: bool,
    /// Prefer landscape works on landscape displays and portrait works on portrait displays.
    pub match_orientation: bool,
    pub transition_seconds: f64,
}

impl Default for WallpaperSettings {
    fn default() -> Self {
        Self {
            live: true,
            sync_system: true,
            screen_saver: false,
            fit: FitMode::Auto,
            crop_tolerance: 0.1,
            pan: true,
            pan_speed: 5.0,
            pan_pause: 20.0,
            pan_paused: false,
            matte: Matte::Average,
            matte_color: "#1d1c1a".into(),
            frame: FrameStyle::None,
            fit_margin: 0.06,
            shadow: true,
            spotlight: true,
            same_on_all_displays: false,
            match_orientation: false,
            transition_seconds: 2.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    Shuffle,
    Sequential,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RotationSettings {
    /// 0 = never change automatically.
    pub interval_minutes: u32,
    pub order: Order,
    pub paused: bool,
    pub scope: Scope,
}

impl Default for RotationSettings {
    fn default() -> Self {
        Self {
            interval_minutes: 30,
            order: Order::Shuffle,
            paused: false,
            scope: Scope::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlatePosition {
    /// 0 = left edge of the safe area, 1 = right edge.
    pub x: f64,
    /// 0 = top edge of the safe area, 1 = bottom edge.
    pub y: f64,
}

impl Default for PlatePosition {
    fn default() -> Self {
        Self { x: 1.0, y: 1.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PlateFields {
    pub artist: bool,
    pub artist_bio: bool,
    pub date: bool,
    pub medium: bool,
    pub dimensions: bool,
    pub description: bool,
    pub museum: bool,
    pub credit: bool,
    pub license: bool,
    pub accession: bool,
    pub qr: bool,
}

impl Default for PlateFields {
    fn default() -> Self {
        Self {
            artist: true,
            artist_bio: true,
            date: true,
            medium: true,
            dimensions: true,
            description: true,
            museum: true,
            credit: true,
            license: false,
            accession: false,
            qr: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PlateSettings {
    pub visible: bool,
    /// Built-in style id (`plastic`, `glass`, ...) or a custom style file name.
    pub style: String,
    pub scale: f64,
    /// Width in logical pixels at scale 1.
    pub width: f64,
    pub fields: PlateFields,
    /// Maximum description lines; 0 = no limit.
    pub description_lines: u32,
    pub default_position: PlatePosition,
    /// Per-display positions, keyed by display key.
    pub positions: HashMap<String, PlatePosition>,
    /// Fade the plate out this many seconds after a painting appears (0 = never).
    pub auto_hide_seconds: u32,
    /// Show descriptions that are only available in another language (e.g. Dutch).
    pub foreign_descriptions: bool,
    /// Distance from the screen edges, in logical pixels.
    pub margin: f64,
}

impl Default for PlateSettings {
    fn default() -> Self {
        Self {
            visible: true,
            style: "plastic".into(),
            scale: 1.0,
            width: 380.0,
            fields: PlateFields::default(),
            description_lines: 6,
            default_position: PlatePosition::default(),
            positions: HashMap::new(),
            auto_hide_seconds: 0,
            foreign_descriptions: false,
            margin: 36.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewerSettings {
    pub show_plate: bool,
    pub plate_position: PlatePosition,
    pub hide_cursor: bool,
    /// 0 = instant, 1 = very floaty.
    pub smoothing: f64,
    pub minimap: bool,
    /// Fraction of each screen edge that maps to "fully revealed".
    pub edge_dead_zone: f64,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            show_plate: true,
            plate_position: PlatePosition { x: 0.0, y: 1.0 },
            hide_cursor: true,
            smoothing: 0.6,
            minimap: true,
            edge_dead_zone: 0.08,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GeneralSettings {
    pub show_in_dock: bool,
    pub launch_at_login: bool,
    /// Fetch descriptions and artist details from museum APIs / Wikidata.
    pub enrich_online: bool,
    pub restore_wallpaper_on_quit: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            show_in_dock: false,
            launch_at_login: false,
            enrich_online: true,
            restore_wallpaper_on_quit: false,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Settings {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(mut s) => {
                    s.normalize();
                    s
                }
                Err(err) => {
                    log::warn!("settings file is invalid ({err}); using defaults");
                    let backup = path.with_extension("json.bak");
                    let _ = std::fs::copy(path, backup);
                    Settings::default()
                }
            },
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_json_atomic(path, self)
    }

    /// Clamp values into sane ranges after loading or patching.
    pub fn normalize(&mut self) {
        let w = &mut self.wallpaper;
        w.crop_tolerance = w.crop_tolerance.clamp(0.0, 0.5);
        w.pan_speed = w.pan_speed.clamp(0.5, 200.0);
        w.pan_pause = w.pan_pause.clamp(0.0, 600.0);
        w.fit_margin = w.fit_margin.clamp(0.0, 0.25);
        w.transition_seconds = w.transition_seconds.clamp(0.0, 10.0);
        if !is_hex_color(&w.matte_color) {
            w.matte_color = WallpaperSettings::default().matte_color;
        }
        let p = &mut self.plate;
        p.scale = p.scale.clamp(0.5, 2.5);
        p.width = p.width.clamp(200.0, 900.0);
        p.margin = p.margin.clamp(0.0, 400.0);
        if p.style.trim().is_empty() {
            p.style = "plastic".into();
        }
        clamp_pos(&mut p.default_position);
        for pos in p.positions.values_mut() {
            clamp_pos(pos);
        }
        clamp_pos(&mut self.viewer.plate_position);
        self.viewer.smoothing = self.viewer.smoothing.clamp(0.0, 0.98);
        self.viewer.edge_dead_zone = self.viewer.edge_dead_zone.clamp(0.0, 0.4);
        self.excluded.sort();
        self.excluded.dedup();
        self.favorites.sort();
        self.favorites.dedup();
        self.sources.disabled_museums.sort();
        self.sources.disabled_museums.dedup();
        if let Some(t) = self.appearance.threshold {
            self.appearance.threshold = t.is_finite().then_some(t.clamp(1.0, 254.0));
        }
        self.download.max_long_edge = self.download.max_long_edge.clamp(2000, 16000);
        self.download.min_long_edge = self.download.min_long_edge.min(12000);
        // Playlists need a unique id and a name; items keep their order, without repeats.
        let mut seen = std::collections::HashSet::new();
        self.playlists.retain_mut(|p| {
            p.name = p.name.trim().to_string();
            if p.id.trim().is_empty() {
                p.id = format!("pl-{:08x}", fastrand::u32(..));
            }
            let mut items = std::collections::HashSet::new();
            p.items.retain(|i| items.insert(i.clone()));
            !p.name.is_empty() && seen.insert(p.id.clone())
        });
        let scope = &mut self.rotation.scope;
        if scope.kind == ScopeKind::Playlist
            && !scope
                .playlist
                .as_ref()
                .is_some_and(|id| self.playlists.iter().any(|p| &p.id == id))
        {
            scope.kind = ScopeKind::All;
            scope.playlist = None;
        }
    }

    /// Apply a JSON merge patch (RFC 7396-ish: objects merge, everything else replaces).
    pub fn patched(&self, patch: &Value) -> Result<Settings, String> {
        let mut base = serde_json::to_value(self).map_err(|e| e.to_string())?;
        merge(&mut base, patch);
        // In keyed maps, patching a key to `null` removes it.
        for path in [["plate", "positions"], ["appearance", "overrides"]] {
            if let Some(Value::Object(map)) = base.get_mut(path[0]).and_then(|v| v.get_mut(path[1]))
            {
                map.retain(|_, v| !v.is_null());
            }
        }
        let mut next: Settings = serde_json::from_value(base).map_err(|e| e.to_string())?;
        next.normalize();
        Ok(next)
    }
}

fn clamp_pos(p: &mut PlatePosition) {
    p.x = if p.x.is_finite() {
        p.x.clamp(0.0, 1.0)
    } else {
        1.0
    };
    p.y = if p.y.is_finite() {
        p.y.clamp(0.0, 1.0)
    } else {
        1.0
    };
}

pub fn is_hex_color(s: &str) -> bool {
    let s = s.trim();
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn parse_hex_color(s: &str) -> (f64, f64, f64) {
    if !is_hex_color(s) {
        return (0.0, 0.0, 0.0);
    }
    let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
    (v(1), v(3), v(5))
}

pub fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                if v.is_null() {
                    // `null` resets optional fields (e.g. libraryPath) but keeps required ones.
                    b.insert(k.clone(), Value::Null);
                } else {
                    merge(b.entry(k.clone()).or_insert(Value::Null), v);
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp: PathBuf = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_merges_nested_objects() {
        let s = Settings::default();
        let next = s
            .patched(
                &serde_json::json!({ "plate": { "style": "glass", "fields": { "qr": true } } }),
            )
            .unwrap();
        assert_eq!(next.plate.style, "glass");
        assert!(next.plate.fields.qr);
        assert!(
            next.plate.fields.artist,
            "untouched fields keep their value"
        );
        assert_eq!(next.wallpaper, s.wallpaper);
    }

    #[test]
    fn unknown_and_missing_fields_are_tolerated() {
        let s: Settings =
            serde_json::from_str(r#"{"plate":{"style":"brass"},"fromTheFuture":1}"#).unwrap();
        assert_eq!(s.plate.style, "brass");
        assert_eq!(s.rotation.interval_minutes, 30);
    }

    #[test]
    fn keyed_maps_and_playlists() {
        let s = Settings::default()
            .patched(&serde_json::json!({
                "appearance": { "matchSystem": true, "overrides": { "a": "light", "b": "both" } },
                "playlists": [{ "id": "", "name": " Evening ", "items": ["x", "y", "x"] }, { "id": "p2", "name": "" }],
            }))
            .unwrap();
        assert!(s.appearance.match_system);
        assert_eq!(s.appearance.overrides.len(), 2);
        assert_eq!(s.playlists.len(), 1, "nameless playlists are dropped");
        assert_eq!(s.playlists[0].name, "Evening");
        assert!(s.playlists[0].id.starts_with("pl-"));
        assert_eq!(s.playlists[0].items, vec!["x", "y"]);
        let s2 = s
            .patched(&serde_json::json!({ "appearance": { "overrides": { "a": null } } }))
            .unwrap();
        assert_eq!(s2.appearance.overrides.len(), 1);
        assert!(s2.appearance.overrides.contains_key("b"));
        // A scope pointing at a playlist that no longer exists falls back to everything.
        let s3 = s2.patched(&serde_json::json!({ "rotation": { "scope": { "kind": "playlist", "playlist": "gone" } } })).unwrap();
        assert_eq!(s3.rotation.scope.kind, ScopeKind::All);
    }

    #[test]
    fn normalize_clamps() {
        let s = Settings::default()
            .patched(
                &serde_json::json!({ "wallpaper": { "panSpeed": 10000.0, "matteColor": "red" } }),
            )
            .unwrap();
        assert_eq!(s.wallpaper.pan_speed, 200.0);
        assert_eq!(s.wallpaper.matte_color, "#1d1c1a");
    }
}
