//! Reads `manifest.json` from the library folder, validates every entry and
//! normalizes it into [`Painting`]s. The manifest format is intentionally
//! forgiving: a bare array, `{ "items": [...] }`, `{ "paintings": [...] }` or
//! `{ "artworks": [...] }` all work, and common key aliases are accepted.

use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

pub const MANIFEST: &str = "manifest.json";
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "avif", "heic", "bmp"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Painting {
    pub id: String,
    /// Path relative to the library root, as written in the manifest.
    pub file: String,
    #[serde(skip)]
    pub path: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub artist_bio: Option<String>,
    pub date: Option<String>,
    pub medium: Option<String>,
    pub dimensions: Option<String>,
    pub description: Option<String>,
    pub museum: Option<String>,
    pub credit: Option<String>,
    pub credit_line: Option<String>,
    pub accession: Option<String>,
    pub license: Option<String>,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub source_page: Option<String>,
    pub tags: Vec<String>,
    pub kind: Option<String>,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub modified: u64,
    /// Any other scalar manifest fields, kept for custom plate styles / future use.
    pub extra: Map<String, Value>,
}

impl Painting {
    pub fn aspect(&self) -> f64 {
        if self.height == 0 {
            1.0
        } else {
            self.width as f64 / self.height as f64
        }
    }

    pub fn orientation(&self) -> &'static str {
        let a = self.aspect();
        if a > 1.08 {
            "landscape"
        } else if a < 0.92 {
            "portrait"
        } else {
            "square"
        }
    }

    /// Key used for the enrichment cache: museum source + id when known.
    pub fn enrich_key(&self) -> String {
        match (&self.source, &self.source_id) {
            (Some(s), Some(id)) if !s.is_empty() && !id.is_empty() => format!("{s}:{id}"),
            _ => format!("file:{}", self.file),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub file: String,
    pub problem: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryReport {
    pub root: Option<PathBuf>,
    pub manifest_found: bool,
    /// Fatal problem reading the manifest (missing, unreadable, invalid JSON).
    pub error: Option<String>,
    pub entries: usize,
    pub valid: usize,
    /// Entries whose file does not exist (yet).
    pub missing: Vec<Issue>,
    /// Entries that could not be used (no file key, unreadable image, path escapes the folder...).
    pub invalid: Vec<Issue>,
    pub duplicates: Vec<Issue>,
    /// Image files in the folder that the manifest does not mention.
    pub unindexed: Vec<String>,
    pub unindexed_count: usize,
    pub generated: Option<String>,
    pub loaded_at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Library {
    pub root: PathBuf,
    pub paintings: Vec<Painting>,
    pub index: HashMap<String, usize>,
    pub report: LibraryReport,
}

impl Library {
    pub fn get(&self, id: &str) -> Option<&Painting> {
        self.index.get(id).and_then(|&i| self.paintings.get(i))
    }
}

/// Cache of image dimensions so re-validating a growing library stays cheap.
#[derive(Default)]
pub struct DimCache(HashMap<PathBuf, (u64, u64, u32, u32)>);

impl DimCache {
    fn dims(&mut self, path: &Path, len: u64, mtime: u64) -> Result<(u32, u32), String> {
        if let Some(&(l, m, w, h)) = self.0.get(path) {
            if l == len && m == mtime {
                return Ok((w, h));
            }
        }
        let size = imagesize::size(path).map_err(|e| format!("unreadable image: {e}"))?;
        let (w, h) = (size.width as u32, size.height as u32);
        if w == 0 || h == 0 {
            return Err("image has zero size".into());
        }
        self.0.insert(path.to_path_buf(), (len, mtime, w, h));
        Ok((w, h))
    }
}

pub fn load(root: &Path, dims: &mut DimCache) -> Library {
    let mut report = LibraryReport {
        root: Some(root.to_path_buf()),
        loaded_at: crate::util::now(),
        ..Default::default()
    };
    let manifest_path = root.join(MANIFEST);
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => {
            report.error = Some(if manifest_path.exists() {
                format!("Could not read {MANIFEST}: {e}")
            } else {
                format!("No {MANIFEST} found in this folder")
            });
            return Library { root: root.to_path_buf(), report, ..Default::default() };
        }
    };
    report.manifest_found = true;
    let json: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            report.error = Some(format!("{MANIFEST} is not valid JSON: {e}"));
            return Library { root: root.to_path_buf(), report, ..Default::default() };
        }
    };

    let (items, generated) = match &json {
        Value::Array(a) => (a.clone(), None),
        Value::Object(o) => {
            let items = ["items", "paintings", "artworks", "works"]
                .iter()
                .find_map(|k| o.get(*k).and_then(|v| v.as_array()).cloned());
            let generated = o.get("generated").and_then(|v| v.as_str()).map(str::to_string);
            match items {
                Some(items) => (items, generated),
                None => {
                    report.error = Some(format!(
                        "{MANIFEST} must be an array or contain an \"items\" array"
                    ));
                    return Library { root: root.to_path_buf(), report, ..Default::default() };
                }
            }
        }
        _ => {
            report.error = Some(format!("{MANIFEST} must be an array or an object"));
            return Library { root: root.to_path_buf(), report, ..Default::default() };
        }
    };
    report.generated = generated;
    report.entries = items.len();

    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut paintings = Vec::with_capacity(items.len());
    let mut seen_ids = HashSet::new();
    let mut referenced: HashSet<PathBuf> = HashSet::new();

    for (n, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            report.invalid.push(Issue { file: format!("entry #{}", n + 1), problem: "not an object".into() });
            continue;
        };
        let Some(file) = str_field(obj, &["file", "path", "filename", "image", "src"]) else {
            report.invalid.push(Issue {
                file: format!("entry #{}", n + 1),
                problem: "no \"file\" field".into(),
            });
            continue;
        };
        let rel = Path::new(&file);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::ParentDir)) {
            report.invalid.push(Issue {
                file: file.clone(),
                problem: "path must be relative to the library folder".into(),
            });
            continue;
        }
        let path = root.join(rel);
        referenced.insert(path.clone());
        let meta = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m,
            Ok(_) => {
                report.invalid.push(Issue { file: file.clone(), problem: "not a file".into() });
                continue;
            }
            Err(_) => {
                report.missing.push(Issue { file: file.clone(), problem: "file not found".into() });
                continue;
            }
        };
        // Symlinks pointing outside the library are refused.
        if let Ok(c) = path.canonicalize() {
            if !c.starts_with(&canonical_root) {
                report.invalid.push(Issue {
                    file: file.clone(),
                    problem: "file resolves outside the library folder".into(),
                });
                continue;
            }
        }
        let len = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (w, h) = match dims.dims(&path, len, mtime) {
            Ok(d) => d,
            Err(problem) => {
                report.invalid.push(Issue { file: file.clone(), problem });
                continue;
            }
        };

        let id = str_field(obj, &["id"]).unwrap_or_else(|| file.clone());
        if !seen_ids.insert(id.clone()) {
            report.duplicates.push(Issue { file: file.clone(), problem: format!("duplicate id \"{id}\"") });
            continue;
        }

        let title = str_field(obj, &["title", "name"]).unwrap_or_else(|| title_from_file(&file));
        let mut p = Painting {
            id,
            file: file.clone(),
            path,
            title,
            artist: str_field(obj, &["artist", "painter", "creator", "author", "maker"]),
            artist_bio: str_field(obj, &["artist_bio", "artistBio", "artist_display", "artistDisplay"]),
            date: str_field(obj, &["date", "year", "period", "created", "dated"]),
            medium: str_field(obj, &["medium", "technique", "materials"]),
            dimensions: str_field(obj, &["dimensions", "measurements", "size"]),
            description: str_field(obj, &["description", "desc", "about", "summary", "label"]),
            museum: str_field(obj, &["museum", "collection", "institution", "repository"]),
            credit: str_field(obj, &["credit", "image_credit", "imageCredit", "attribution", "photo_credit"]),
            credit_line: str_field(obj, &["credit_line", "creditLine", "creditline", "acquisition"]),
            accession: str_field(obj, &["accession", "accession_number", "accessionNumber", "object_number", "objectNumber", "inventory"]),
            license: str_field(obj, &["license", "licence", "rights"]),
            source: str_field(obj, &["source"]),
            source_id: str_field(obj, &["source_id", "sourceId", "object_id", "objectId"]),
            source_page: str_field(obj, &["source_page", "sourcePage", "url", "link", "page"]),
            tags: obj
                .get("tags")
                .and_then(|t| t.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            kind: str_field(obj, &["kind", "type", "classification"]),
            width: w,
            height: h,
            bytes: len,
            modified: mtime,
            extra: Map::new(),
        };
        const KNOWN: &[&str] = &[
            "file", "path", "filename", "image", "src", "id", "title", "name", "artist", "painter", "creator",
            "author", "maker", "artist_bio", "artistBio", "artist_display", "artistDisplay", "date", "year",
            "period", "created", "dated", "medium", "technique", "materials", "dimensions", "measurements",
            "size", "description", "desc", "about", "summary", "label", "museum", "collection", "institution",
            "repository", "credit", "image_credit", "imageCredit", "attribution", "photo_credit", "credit_line",
            "creditLine", "creditline", "acquisition", "accession", "accession_number", "accessionNumber",
            "object_number", "objectNumber", "inventory", "license", "licence", "rights", "source", "source_id",
            "sourceId", "object_id", "objectId", "source_page", "sourcePage", "url", "link", "page", "tags",
            "kind", "type", "classification", "width", "height", "aspect", "orientation", "megapixels", "bytes",
            "download_url", "added",
        ];
        for (k, v) in obj {
            if !KNOWN.contains(&k.as_str()) && (v.is_string() || v.is_number() || v.is_boolean()) {
                p.extra.insert(k.clone(), v.clone());
            }
        }
        paintings.push(p);
    }

    // Report image files that exist in the folder but are not in the manifest.
    let mut unindexed = Vec::new();
    collect_images(root, 0, &mut |p| {
        if !referenced.contains(p) {
            if let Ok(rel) = p.strip_prefix(root) {
                unindexed.push(rel.to_string_lossy().to_string());
            }
        }
    });
    unindexed.sort();
    report.unindexed_count = unindexed.len();
    unindexed.truncate(200);
    report.unindexed = unindexed;
    report.valid = paintings.len();

    let index = paintings.iter().enumerate().map(|(i, p)| (p.id.clone(), i)).collect();
    Library { root: root.to_path_buf(), paintings, index, report }
}

fn collect_images(dir: &Path, depth: usize, f: &mut dyn FnMut(&Path)) {
    if depth > 3 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        if p.is_dir() {
            collect_images(&p, depth + 1, f);
        } else if is_image(&p) {
            f(&p);
        }
    }
}

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn str_field(obj: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for k in keys {
        match obj.get(*k) {
            Some(Value::String(s)) if !s.trim().is_empty() => return Some(s.trim().to_string()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

/// `claude-monet__water-lilies.jpg` → `Water Lilies`
fn title_from_file(file: &str) -> String {
    let stem = Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or(file);
    let stem = stem.rsplit("__").next().unwrap_or(stem);
    stem.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, w: u32, h: u32) {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 100, 50]));
        img.save(path).unwrap();
    }

    #[test]
    fn validates_entries() {
        let dir = std::env::temp_dir().join(format!("galeria-test-{}", fastrand::u64(..)));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        write_png(&dir.join("a.png"), 30, 10);
        write_png(&dir.join("sub/b.png"), 10, 30);
        write_png(&dir.join("stray.png"), 5, 5);
        std::fs::write(dir.join("broken.jpg"), b"not an image").unwrap();
        let manifest = serde_json::json!({ "items": [
            { "file": "a.png", "title": "Wide", "artist": "A. Painter", "year": 1890 },
            { "path": "sub/b.png", "painter": "B" },
            { "file": "missing.jpg" },
            { "file": "../escape.jpg" },
            { "file": "broken.jpg" },
            { "file": "a.png" },
            { "title": "no file" }
        ]});
        std::fs::write(dir.join(MANIFEST), manifest.to_string()).unwrap();

        let lib = load(&dir, &mut DimCache::default());
        assert_eq!(lib.report.error, None);
        assert_eq!(lib.report.entries, 7);
        assert_eq!(lib.paintings.len(), 2);
        let a = lib.get("a.png").unwrap();
        assert_eq!((a.width, a.height), (30, 10));
        assert_eq!(a.date.as_deref(), Some("1890"));
        assert_eq!(a.orientation(), "landscape");
        let b = lib.get("sub/b.png").unwrap();
        assert_eq!(b.title, "B");
        assert_eq!(b.artist.as_deref(), Some("B"));
        assert_eq!(b.orientation(), "portrait");
        assert_eq!(lib.report.missing.len(), 1);
        assert_eq!(lib.report.invalid.len(), 3, "{:?}", lib.report.invalid);
        assert_eq!(lib.report.duplicates.len(), 1);
        assert_eq!(lib.report.unindexed, vec!["stray.png".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn titles_from_file_names() {
        assert_eq!(title_from_file("claude-monet__water-lilies.jpg"), "Water Lilies");
        assert_eq!(title_from_file("sub/the_night_watch.png"), "The Night Watch");
    }

    #[test]
    fn invalid_json_is_reported() {
        let dir = std::env::temp_dir().join(format!("galeria-test-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), "{ \"items\": [ { \"file\": ").unwrap();
        let lib = load(&dir, &mut DimCache::default());
        assert!(lib.report.error.unwrap().contains("not valid JSON"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
