//! Commands invoked from the webviews.

use crate::core::{self, Colors, Core, LibraryView, Snapshot};
use crate::settings::Settings;
use crate::sources::{self, Ctx};
use crate::{download, genres, licensing, manifest, styles, wall};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn get_snapshot(core: State<'_, Core>) -> Snapshot {
    core.snapshot()
}

#[tauri::command]
pub fn get_library(core: State<'_, Core>) -> LibraryView {
    core.library_view()
}

#[tauri::command]
pub fn patch_settings(app: AppHandle, patch: Value) -> Result<Settings, String> {
    core::patch_settings(&app, &patch)
}

#[tauri::command]
pub fn next_painting(app: AppHandle, display: Option<String>) {
    core::advance(&app, display, true);
}

#[tauri::command]
pub fn previous_painting(app: AppHandle, display: Option<String>) {
    core::advance(&app, display, false);
}

#[tauri::command]
pub fn show_painting(app: AppHandle, id: String, display: Option<String>) -> Result<(), String> {
    core::show(&app, &id, display)
}

#[tauri::command]
pub async fn open_viewer(
    app: AppHandle,
    id: Option<String>,
    display: Option<String>,
) -> Result<(), String> {
    wall::open_viewer(&app, id, display)
}

#[tauri::command]
pub async fn close_viewer(app: AppHandle) {
    wall::close_viewer(&app);
}

#[tauri::command]
pub async fn open_settings(app: AppHandle, section: Option<String>) -> Result<(), String> {
    wall::open_settings(&app, section)
}

#[tauri::command]
pub fn set_plate_position(app: AppHandle, scope: String, display: Option<String>, x: f64, y: f64) {
    core::set_plate_position(&app, &scope, display, x, y);
}

#[tauri::command]
pub async fn set_arrange(app: AppHandle, on: bool) {
    wall::set_arrange(&app, on);
}

#[tauri::command]
pub fn set_library_path(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let path = path.filter(|p| !p.trim().is_empty()).map(PathBuf::from);
    if let Some(p) = &path {
        if !p.is_dir() {
            return Err(format!("{} is not a folder", p.display()));
        }
    }
    core::patch_settings(&app, &serde_json::json!({ "libraryPath": path })).map(|_| ())
}

#[tauri::command]
pub async fn rescan_library(app: AppHandle) {
    core::reload_library(&app);
}

#[tauri::command]
pub fn set_excluded(app: AppHandle, id: String, excluded: bool) {
    core::set_excluded(&app, &id, excluded);
}

#[tauri::command]
pub fn set_favorite(app: AppHandle, id: String, favorite: bool) {
    core::set_favorite(&app, &id, favorite);
}

#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only web links can be opened".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn reveal_painting(app: AppHandle, id: String) -> Result<(), String> {
    let src = app
        .state::<Core>()
        .source_for(&id)
        .ok_or("unknown painting")?;
    app.opener()
        .reveal_item_in_dir(src.path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn painting_colors(app: AppHandle, id: String) -> Result<Colors, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Core>().colors_for(&id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn report_image_error(app: AppHandle, id: String, display: Option<String>) {
    core::report_bad_image(&app, &id, display);
}

#[tauri::command]
pub fn open_styles_folder(app: AppHandle) -> Result<(), String> {
    let dir = app.state::<Core>().paths.styles.clone();
    let _ = std::fs::create_dir_all(&dir);
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_custom_style(app: AppHandle, name: String) -> Result<String, String> {
    let dir = app.state::<Core>().paths.styles.clone();
    let path = styles::create(&dir, &name)?;
    core::reload_styles(&app);
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    core::patch_settings(
        &app,
        &serde_json::json!({ "plate": { "style": id, "visible": true } }),
    )?;
    let _ = app.opener().open_path(path.to_string_lossy(), None::<&str>);
    Ok(id)
}

#[tauri::command]
pub fn open_style_file(app: AppHandle, id: String) -> Result<(), String> {
    let core = app.state::<Core>();
    let path = core
        .lock()
        .styles
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.path.clone())
        .ok_or("unknown style")?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn restore_original_wallpaper(app: AppHandle) {
    let _ = core::patch_settings(
        &app,
        &serde_json::json!({ "wallpaper": { "syncSystem": false } }),
    );
    core::restore_original_wallpaper(&app);
}

#[tauri::command]
pub fn refresh_enrichment(app: AppHandle, id: Option<String>) {
    app.state::<Core>().refresh_enrichment(id.as_deref());
}

#[tauri::command]
pub fn clear_image_cache(app: AppHandle) -> Result<u64, String> {
    let cache = app.state::<Core>().paths.cache.clone();
    let mut freed = 0;
    for sub in ["renders", "thumbs"] {
        let dir = cache.join(sub);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                freed += e.metadata().map(|m| m.len()).unwrap_or(0);
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(freed)
}

// ---------------------------------------------------------------------------
// Sources, licensing & downloads
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    /// The licensing rules exactly as compiled into the app.
    rules: &'static licensing::Rules,
    genres: &'static [genres::Genre],
    /// Local catalogues some sources need before they can be searched.
    catalogs: Vec<sources::nga::CatalogStatus>,
}

#[tauri::command]
pub fn get_catalogue(core: State<'_, Core>) -> Catalogue {
    Catalogue {
        rules: licensing::rules(),
        genres: genres::all(),
        catalogs: vec![sources::nga::status(&core.paths.cache)],
    }
}

fn ctx(app: &AppHandle) -> Ctx {
    app.state::<Core>().source_ctx(app)
}

/// Search one museum. Every result carries the license decision made from the museum's own record.
#[tauri::command]
pub async fn discover_search(
    app: AppHandle,
    query: sources::Query,
) -> Result<sources::Page, String> {
    let mut page = sources::search(&ctx(&app), &query).await?;
    let core = app.state::<Core>();
    let i = core.lock();
    for c in &mut page.items {
        c.in_library = i.library.get(&c.id).is_some();
    }
    Ok(page)
}

/// Download (or refresh) the open dataset a source is searched from.
#[tauri::command]
pub async fn discover_prepare(app: AppHandle, source: String) -> Result<usize, String> {
    match source.as_str() {
        "nga" => sources::nga::prepare(&ctx(&app)).await,
        _ => Ok(0),
    }
}

#[tauri::command]
pub fn download_start(app: AppHandle, items: Vec<download::Request>) -> usize {
    let n = app.state::<Core>().downloads.add(items);
    core::publish(&app);
    n
}

#[tauri::command]
pub fn download_cancel(app: AppHandle, id: Option<String>) {
    app.state::<Core>().downloads.cancel(id.as_deref());
    core::publish(&app);
}

#[tauri::command]
pub fn download_clear(app: AppHandle) {
    app.state::<Core>().downloads.clear_finished();
    core::publish(&app);
}

#[tauri::command]
pub fn get_downloads(core: State<'_, Core>) -> Vec<download::Job> {
    core.downloads.list()
}

/// Remove manifest entries whose file no longer exists (after the user deleted paintings).
#[tauri::command]
pub async fn manifest_cleanup(app: AppHandle) -> Result<usize, String> {
    let (root, backups) = {
        let core = app.state::<Core>();
        let root = core.lock().settings.library_path.clone();
        (
            root.ok_or("No art folder chosen.")?,
            core.paths.manifest_backups.clone(),
        )
    };
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let n = manifest::remove_missing(&root, Some(&backups))?;
        core::reload_library(&a);
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The webview noticed the system appearance change (the core also polls).
#[tauri::command]
pub async fn report_theme(app: AppHandle, dark: bool) {
    let dark = crate::platform::system_dark().unwrap_or(dark);
    core::set_dark(&app, dark);
}

/// Lets the webviews write to the app log (diagnostics).
#[tauri::command]
pub fn frontend_log(window: tauri::WebviewWindow, level: String, message: String) {
    let label = window.label().to_string();
    match level.as_str() {
        "error" => log::error!("[{label}] {message}"),
        "warn" => log::warn!("[{label}] {message}"),
        _ => log::info!("[{label}] {message}"),
    }
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    crate::quit(&app);
}
