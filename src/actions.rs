//! What the windows ask the core to do, beyond changing settings.
//!
//! These read files, talk to museums or wait on the UI thread, so the windows
//! call them from a worker (`ui::Ui::run`), never from an event handler directly.

use crate::core::{self, Core};
use crate::host::AppHandle;
use crate::images::{self, Source};
use crate::sources::{self, Ctx};
use crate::{download, genres, licensing, manifest, open, styles};
use serde_json::json;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

pub fn set_library_path(app: &AppHandle, path: Option<PathBuf>) -> Result<(), String> {
    let path = path.filter(|p| !p.as_os_str().is_empty());
    if let Some(p) = &path {
        if !p.is_dir() {
            return Err(format!("{} is not a folder", p.display()));
        }
    }
    core::patch_settings(app, &json!({ "libraryPath": path })).map(|_| ())
}

/// Make `path` the art folder of someone starting out: create the folder if it
/// is not there, and an empty manifest.json if it has none. A manifest that is
/// already there, and everything else in the folder, is left alone.
pub fn setup_library(app: &AppHandle, path: PathBuf) -> Result<(), String> {
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Could not create {}: {e}", path.display()))?;
    create_manifest(app, &path)?;
    set_library_path(app, Some(path))
}

/// Start an empty manifest.json in `root` when it has none.
pub fn create_manifest(app: &AppHandle, root: &std::path::Path) -> Result<(), String> {
    if crate::manifest::create(root)? {
        log::info!("created an empty manifest in {}", root.display());
        core::reload_library(app);
    }
    Ok(())
}

pub fn open_url(url: &str) -> Result<(), String> {
    if open::url(url) {
        Ok(())
    } else {
        Err("only web links can be opened".into())
    }
}

pub fn reveal_painting(app: &AppHandle, id: &str) -> Result<(), String> {
    let src = app.core().source_for(id).ok_or("unknown painting")?;
    open::reveal(&src.path);
    Ok(())
}

pub fn open_styles_folder(app: &AppHandle) {
    let dir = app.core().paths.styles.clone();
    let _ = std::fs::create_dir_all(&dir);
    open::path(&dir);
}

/// Write a documented style file, switch to it and open it in the user's editor.
pub fn create_custom_style(app: &AppHandle, name: &str) -> Result<String, String> {
    let dir = app.core().paths.styles.clone();
    let path = styles::create(&dir, name)?;
    core::reload_styles(app);
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    core::patch_settings(app, &json!({ "plate": { "style": id, "visible": true } }))?;
    open::path(&path);
    Ok(id)
}

pub fn open_style_file(app: &AppHandle, id: &str) -> Result<(), String> {
    let path = app
        .core()
        .lock()
        .styles
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.path.clone())
        .ok_or("unknown style")?;
    open::path(&path);
    Ok(())
}

pub fn restore_original_wallpaper(app: &AppHandle) {
    let _ = core::patch_settings(app, &json!({ "wallpaper": { "syncSystem": false } }));
    core::restore_original_wallpaper(app);
}

/// Delete cached thumbnails, renditions and museum search images. Returns the bytes freed.
pub fn clear_image_cache(core: &Core) -> u64 {
    let mut freed = 0;
    for sub in ["renders", "thumbs", "remote"] {
        let dir = core.paths.cache.join(sub);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                freed += e.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    freed
}

// ---------------------------------------------------------------------------
// Sources, licensing & downloads
// ---------------------------------------------------------------------------

/// What Discover and Museums show that is not part of the snapshot.
#[derive(Clone)]
pub struct Catalogue {
    /// The licensing rules exactly as compiled into the app.
    pub rules: &'static licensing::Rules,
    pub genres: &'static [genres::Genre],
    /// Local catalogues some sources need before they can be searched.
    pub catalogs: Vec<sources::nga::CatalogStatus>,
}

pub fn catalogue(core: &Core) -> Catalogue {
    Catalogue {
        rules: licensing::rules(),
        genres: genres::all(),
        catalogs: vec![sources::nga::status(&core.paths.cache)],
    }
}

fn ctx(app: &AppHandle) -> Ctx {
    app.core().source_ctx(app)
}

/// Search one museum. Every result carries the license decision made from the museum's own record.
pub async fn discover_search(
    app: &AppHandle,
    query: &sources::Query,
) -> Result<sources::Page, String> {
    let mut page = sources::search(&ctx(app), query).await?;
    let i = app.core().lock();
    for c in &mut page.items {
        c.in_library = i.library.get(&c.id).is_some();
    }
    Ok(page)
}

/// Download (or refresh) the open dataset a source is searched from.
pub async fn discover_prepare(app: &AppHandle, source: &str) -> Result<usize, String> {
    match source {
        "nga" => sources::nga::prepare(&ctx(app)).await,
        _ => Ok(0),
    }
}

pub fn download_start(app: &AppHandle, items: Vec<download::Request>) -> usize {
    let n = app.core().downloads.add(items);
    download::notify(app);
    core::publish(app);
    n
}

pub fn download_cancel(app: &AppHandle, id: Option<&str>) {
    app.core().downloads.cancel(id);
    download::notify(app);
    core::publish(app);
}

pub fn download_clear(app: &AppHandle) {
    app.core().downloads.clear_finished();
    download::notify(app);
    core::publish(app);
}

/// Remove manifest entries whose file no longer exists (after the user deleted paintings).
pub fn manifest_cleanup(app: &AppHandle) -> Result<usize, String> {
    let core = app.core();
    let root = core
        .lock()
        .settings
        .library_path
        .clone()
        .ok_or("No art folder chosen.")?;
    let n = manifest::remove_missing(&root, Some(&core.paths.manifest_backups))?;
    core::reload_library(app);
    Ok(n)
}

/// Permanently delete paintings: remove their image files inside the art folder,
/// remove their entries from `manifest.json` (keeping a backup), forget them in
/// the download journal, and advance any display that was showing one.
pub fn delete_paintings(app: &AppHandle, ids: &[String]) -> Result<usize, String> {
    let wanted: HashSet<String> = ids.iter().cloned().collect();
    if wanted.is_empty() {
        return Ok(0);
    }
    let core = app.core();
    let (root, targets, shared_paths) = {
        let i = core.lock();
        let root = i
            .settings
            .library_path
            .clone()
            .filter(|r| r.is_dir())
            .ok_or("No art folder chosen.")?;
        let shared: HashSet<PathBuf> = i
            .library
            .paintings
            .iter()
            .filter(|p| !wanted.contains(&p.id))
            .map(|p| p.path.clone())
            .collect();
        let targets: Vec<(String, String, PathBuf, Source)> = i
            .library
            .paintings
            .iter()
            .filter(|p| wanted.contains(&p.id))
            .map(|p| {
                let src = Source {
                    path: p.path.clone(),
                    width: p.width,
                    height: p.height,
                    bytes: p.bytes,
                    modified: p.modified,
                };
                (p.id.clone(), p.file.clone(), p.path.clone(), src)
            })
            .collect();
        (root, targets, shared)
    };
    if targets.is_empty() {
        return Ok(0);
    }
    let mut deleted_ids = HashSet::with_capacity(targets.len());
    for (id, file, path, src) in &targets {
        let rel = Path::new(file);
        if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(format!("Refusing to delete unsafe path: {file}"));
        }
        if *path != root.join(rel) {
            return Err(format!("Refusing to delete path outside library: {file}"));
        }
        if !shared_paths.contains(path) {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!("Could not delete {file}: {e}"));
                }
            }
            images::remove_cached(src, &core.paths.cache);
        }
        deleted_ids.insert(id.clone());
    }
    let removed = manifest::remove_ids(&root, &deleted_ids, Some(&core.paths.manifest_backups))?;
    download::forget(&core.paths.downloads_journal, &core.downloads, &deleted_ids);
    download::notify(app);
    core::forget_deleted_ids(app, &deleted_ids);
    core::reload_library(app);
    log::info!(
        "deleted {} painting(s) ({removed} manifest entries)",
        deleted_ids.len()
    );
    Ok(deleted_ids.len())
}

/// Delete every painting belonging to `museum`.
pub fn delete_museum(app: &AppHandle, museum: &str) -> Result<usize, String> {
    let ids: Vec<String> = app
        .core()
        .lock()
        .library
        .paintings
        .iter()
        .filter(|p| p.museum.as_deref() == Some(museum))
        .map(|p| p.id.clone())
        .collect();
    delete_paintings(app, &ids)
}
