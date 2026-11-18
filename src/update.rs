//! Serverless updates from the `latest.json` asset on a GitHub Release.
//!
//! The DMG is for people installing Galeria; the ZIP is for the updater. A
//! signed macOS app must be replaced as a whole bundle to preserve its
//! signature. A detached helper waits for Galeria to exit before swapping the
//! bundle, so the running application never overwrites itself.

use crate::core;
use crate::host::{self, AppHandle};
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const REPO: &str = "nikolovlazar/galeria";
const DAY: u64 = 24 * 60 * 60;

pub fn feed_url() -> String {
    std::env::var("GALERIA_UPDATE_FEED").unwrap_or_else(|_| {
        format!("https://github.com/{REPO}/releases/latest/download/latest.json")
    })
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("macos-aarch64"),
        ("macos", "x86_64") => Some("macos-x86_64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub check_on_launch: bool,
    pub skipped_version: Option<String>,
    pub last_check: Option<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            check_on_launch: true,
            skipped_version: None,
            last_check: None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Manifest {
    pub version: String,
    pub page: Option<String>,
    pub notes: Option<String>,
    pub platforms: HashMap<String, Asset>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Asset {
    pub url: String,
    pub sha256: String,
    pub size: Option<u64>,
    pub signature: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum State {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_at: u64,
    },
    Available {
        version: String,
        notes: Option<String>,
        page: Option<String>,
    },
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Installing {
        version: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    pub current: String,
    pub can_install: bool,
    pub hint: Option<String>,
}

static CAPABILITY: LazyLock<(bool, Option<String>)> = LazyLock::new(install_capability);
static STATUS: LazyLock<Mutex<Status>> = LazyLock::new(|| {
    Mutex::new(Status {
        state: State::Idle,
        current: current_version().to_string(),
        can_install: CAPABILITY.0,
        hint: CAPABILITY.1.clone(),
    })
});
// The selected asset is kept separately from the public, serializable status.
static OFFER: Mutex<Option<(String, Asset)>> = Mutex::new(None);
static LAST_AUTO_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static CHECK_CLAIM: Mutex<()> = Mutex::new(());

fn status_lock() -> std::sync::MutexGuard<'static, Status> {
    STATUS.lock().unwrap_or_else(|e| e.into_inner())
}

fn offer_lock() -> std::sync::MutexGuard<'static, Option<(String, Asset)>> {
    OFFER.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn status() -> Status {
    status_lock().clone()
}

fn set_status(app: &AppHandle, state: State, can_install: bool, hint: Option<String>) {
    {
        let mut status = status_lock();
        status.state = state;
        status.can_install = can_install;
        status.hint = hint;
    }
    core::publish(app);
}

fn set_state(app: &AppHandle, state: State) {
    set_status(app, state, CAPABILITY.0, CAPABILITY.1.clone());
}

fn fail(app: &AppHandle, error: String) {
    log::warn!("update: {error}");
    set_state(app, State::Failed { error });
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.strip_prefix('v').unwrap_or(s).split('.');
    fn component(s: &str) -> Option<u64> {
        if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }
    let major = component(parts.next()?)?;
    let minor = match parts.next() {
        Some(s) => component(s)?,
        None => 0,
    };
    let patch = match parts.next() {
        Some(s) => component(s)?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn newer(a: &str, b: &str) -> bool {
    match (parse_version(a), parse_version(b)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(format!("galeria/{}", current_version()))
        .build()
        .map_err(|e| format!("Could not prepare the update connection: {e}"))
}

async fn fetch_manifest() -> Result<Manifest, String> {
    let response = client()?
        .get(feed_url())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("Could not check for updates: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Could not check for updates: {e}"))?;
    let manifest: Manifest = response
        .json()
        .await
        .map_err(|e| format!("The update feed is invalid: {e}"))?;
    if parse_version(&manifest.version).is_none() {
        return Err("The update feed contains an invalid version.".into());
    }
    Ok(manifest)
}

fn recent(timestamp: u64, current: u64) -> bool {
    timestamp != 0 && current.saturating_sub(timestamp) < DAY
}

// lastCheck records only successful fetches. A separate cache marker throttles
// failed automatic attempts too, including across application restarts.
fn automatic_due(app: &AppHandle) -> bool {
    let current = now();
    let settings = app.core().lock().settings.update.clone();
    if !settings.check_on_launch || settings.last_check.is_some_and(|t| recent(t, current)) {
        return false;
    }
    if recent(LAST_AUTO_ATTEMPT.load(Ordering::Relaxed), current) {
        return false;
    }
    let marker = app.core().paths.cache.join("updates").join("last-attempt");
    let previous = fs::read_to_string(&marker)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    if previous.is_some_and(|t| recent(t, current)) {
        return false;
    }
    LAST_AUTO_ATTEMPT.store(current, Ordering::Relaxed);
    if let Some(parent) = marker.parent() {
        if let Err(e) =
            fs::create_dir_all(parent).and_then(|_| fs::write(&marker, current.to_string()))
        {
            log::warn!("could not record automatic update attempt: {e}");
        }
    }
    true
}

fn claim_check(app: &AppHandle, manual: bool) -> bool {
    {
        let _claim = CHECK_CLAIM.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(
            status_lock().state,
            State::Checking | State::Downloading { .. } | State::Installing { .. }
        ) {
            return false;
        }
        // The core snapshot takes the core lock before the status lock.
        if !manual && !automatic_due(app) {
            return false;
        }
        let mut status = status_lock();
        if matches!(
            status.state,
            State::Checking | State::Downloading { .. } | State::Installing { .. }
        ) {
            return false;
        }
        status.state = State::Checking;
        *offer_lock() = None;
    }
    core::publish(app);
    true
}

async fn run_check(app: &AppHandle, manual: bool) {
    let manifest = match fetch_manifest().await {
        Ok(manifest) => manifest,
        Err(error) => {
            fail(app, error);
            return;
        }
    };
    let checked_at = now();
    if let Err(e) = core::patch_settings(
        app,
        &serde_json::json!({"update": {"lastCheck": checked_at}}),
    ) {
        log::warn!("could not save update check time: {e}");
    }
    let skipped = app.core().lock().settings.update.skipped_version.clone();
    if newer(&manifest.version, current_version())
        && (manual || skipped.as_deref() != Some(&manifest.version))
    {
        let asset = platform_key()
            .and_then(|key| manifest.platforms.get(key))
            .cloned();
        let has_asset = asset.is_some();
        if let Some(asset) = asset {
            *offer_lock() = Some((manifest.version.clone(), asset));
        }
        let hint = if has_asset {
            CAPABILITY.1.clone()
        } else {
            Some("This release has no build for this platform. Use the download link.".into())
        };
        set_status(
            app,
            State::Available {
                version: manifest.version,
                notes: manifest.notes,
                page: manifest.page,
            },
            CAPABILITY.0 && has_asset,
            hint,
        );
    } else {
        set_state(app, State::UpToDate { checked_at });
    }
}

pub fn check(app: &AppHandle, manual: bool) {
    if !claim_check(app, manual) {
        return;
    }
    let app = app.clone();
    host::spawn(async move {
        run_check(&app, manual).await;
    });
}

pub fn check_blocking(app: &AppHandle, manual: bool) -> Status {
    if claim_check(app, manual) {
        host::block_on(run_check(app, manual));
    }
    status()
}

pub fn skip(app: &AppHandle, version: &str) {
    if let Err(e) = core::patch_settings(
        app,
        &serde_json::json!({"update": {"skippedVersion": version}}),
    ) {
        log::warn!("could not save skipped update: {e}");
    }
    let changed = {
        let mut status = status_lock();
        if matches!(&status.state, State::Available { version: offered, .. } if offered == version)
        {
            status.state = State::UpToDate { checked_at: now() };
            *offer_lock() = None;
            true
        } else {
            false
        }
    };
    if changed {
        core::publish(app);
    }
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    host::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        check(&app, false);
    });
}

pub fn install(app: &AppHandle) {
    let selected = {
        let mut status = status_lock();
        if !matches!(status.state, State::Available { .. }) {
            return;
        }
        let asset = offer_lock().clone();
        if !status.can_install {
            Err(status
                .hint
                .clone()
                .unwrap_or_else(|| "This installation cannot be updated automatically.".into()))
        } else {
            asset.ok_or_else(|| "No update is available for this platform.".to_string())
        }
        .map(|(version, asset)| {
            status.state = State::Downloading {
                version: version.clone(),
                received: 0,
                total: asset.size.unwrap_or(0),
            };
            (version, asset)
        })
    };
    let (version, asset) = match selected {
        Ok(selected) => selected,
        Err(error) => {
            fail(app, error);
            return;
        }
    };
    core::publish(app);
    let app = app.clone();
    host::spawn(async move {
        let result = match download(&app, &version, &asset).await {
            Ok(archive) => {
                let result = verify_asset(&archive, &asset).and_then(|()| {
                    set_state(
                        &app,
                        State::Installing {
                            version: version.clone(),
                        },
                    );
                    apply(&archive, &app.core().paths.cache, true)
                });
                // Windows still needs the installer file after it has been spawned.
                if !cfg!(target_os = "windows") || result.is_err() {
                    let _ = fs::remove_file(&archive);
                }
                result
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => core::quit(&app),
            Err(error) => fail(&app, error),
        }
    });
}

async fn download(app: &AppHandle, version: &str, asset: &Asset) -> Result<PathBuf, String> {
    let dir = app.core().paths.cache.join("updates");
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the update cache: {e}"))?;
    let suffix = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let path = dir.join(format!(
        "download-{}-{}{suffix}",
        std::process::id(),
        fastrand::u64(..)
    ));
    let result = async {
        let mut response = client()?
            .get(&asset.url)
            .send()
            .await
            .map_err(|e| format!("Could not download the update: {e}"))?
            .error_for_status()
            .map_err(|e| format!("Could not download the update: {e}"))?;
        let mut file =
            File::create(&path).map_err(|e| format!("Could not save the update: {e}"))?;
        let total = asset
            .size
            .or_else(|| response.content_length())
            .unwrap_or(0);
        let mut received = 0u64;
        let mut last = Instant::now();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("The update download failed: {e}"))?
        {
            file.write_all(&chunk)
                .map_err(|e| format!("Could not save the update: {e}"))?;
            received = received.saturating_add(chunk.len() as u64);
            if last.elapsed() >= Duration::from_millis(200) {
                set_state(
                    app,
                    State::Downloading {
                        version: version.to_string(),
                        received,
                        total,
                    },
                );
                last = Instant::now();
            }
        }
        file.flush()
            .map_err(|e| format!("Could not save the update: {e}"))?;
        set_state(
            app,
            State::Downloading {
                version: version.to_string(),
                received,
                total,
            },
        );
        Ok::<(), String>(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result.map(|()| path)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("Could not read the update: {e}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("Could not read the update: {e}"))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn verify_signature(bytes: &[u8], public_key: &str, signature: Option<&str>) -> Result<(), String> {
    let decode = |value: &str| base64::engine::general_purpose::STANDARD.decode(value);
    let key = decode(public_key).map_err(|_| "The update signing key is invalid.".to_string())?;
    let key: [u8; 32] = key
        .try_into()
        .map_err(|_| "The update signing key is invalid.".to_string())?;
    let key = VerifyingKey::from_bytes(&key)
        .map_err(|_| "The update signing key is invalid.".to_string())?;
    let signature = signature.ok_or_else(|| "The update has no signature.".to_string())?;
    let signature =
        decode(signature).map_err(|_| "The update signature is invalid.".to_string())?;
    let signature = Signature::from_slice(&signature)
        .map_err(|_| "The update signature is invalid.".to_string())?;
    key.verify_strict(bytes, &signature)
        .map_err(|_| "The update signature could not be verified.".to_string())
}

fn verify_asset(path: &Path, asset: &Asset) -> Result<(), String> {
    if let Some(expected) = asset.size {
        let actual = fs::metadata(path)
            .map_err(|e| format!("Could not read the update: {e}"))?
            .len();
        if actual != expected {
            return Err("The update size does not match the release.".into());
        }
    }
    let hash = sha256_file(path)?;
    if asset.sha256.len() != 64 || !hash.eq_ignore_ascii_case(&asset.sha256) {
        return Err("The update checksum does not match the release.".into());
    }
    if let Some(key) = option_env!("GALERIA_UPDATE_PUBKEY")
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        let bytes = fs::read(path).map_err(|e| format!("Could not read the update: {e}"))?;
        verify_signature(&bytes, key, asset.signature.as_deref())?;
    }
    Ok(())
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    Command::new("/bin/sh")
        .arg("-c")
        .arg("test -w \"$1\"")
        .arg("sh")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(target_os = "macos")]
fn bundle_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not find the running application: {e}"))?;
    if exe
        .components()
        .any(|c| c.as_os_str() == "AppTranslocation")
    {
        return Err("Move Galeria to Applications before installing updates.".into());
    }
    let macos = exe
        .parent()
        .ok_or("The running application is not inside an app bundle.")?;
    let contents = macos
        .parent()
        .ok_or("The running application is not inside an app bundle.")?;
    let bundle = contents
        .parent()
        .ok_or("The running application is not inside an app bundle.")?;
    if macos.file_name().is_none_or(|n| n != "MacOS")
        || contents.file_name().is_none_or(|n| n != "Contents")
        || bundle.extension().is_none_or(|e| e != "app")
    {
        return Err("The running application is not inside an app bundle.".into());
    }
    Ok(bundle.to_path_buf())
}

fn install_capability() -> (bool, Option<String>) {
    if platform_key().is_none() {
        return (
            false,
            Some("Automatic updates are not supported on this platform.".into()),
        );
    }
    #[cfg(target_os = "macos")]
    {
        return match bundle_path() {
            Ok(bundle) if bundle.parent().is_some_and(writable) => (true, None),
            Ok(_) => (
                false,
                Some("Move Galeria to a writable folder before installing updates.".into()),
            ),
            Err(hint) => (false, Some(hint)),
        };
    }
    #[cfg(target_os = "linux")]
    {
        return match std::env::current_exe() {
            Ok(exe) if writable(&exe) => (true, None),
            _ => (
                false,
                Some("The Galeria executable is not writable.".into()),
            ),
        };
    }
    #[cfg(target_os = "windows")]
    {
        return (true, None);
    }
    #[allow(unreachable_code)]
    (
        false,
        Some("Automatic updates are not supported on this platform.".into()),
    )
}

// Paths are passed as positional arguments, not interpolated into shell code.
#[cfg(any(target_os = "macos", test))]
fn helper_script() -> &'static str {
    r#"pid=$1
dest=$2
staged=$3
relaunch=$4
work=$5
backup="${dest}.old-${pid}"
while kill -0 "$pid" 2>/dev/null; do sleep 1; done
if [ -e "$backup" ]; then echo "backup already exists: $backup" >&2; exit 1; fi
if ! mv "$dest" "$backup"; then echo "could not move old application" >&2; exit 1; fi
if ! /usr/bin/ditto "$staged" "$dest"; then
    echo "could not install update; restoring old application" >&2
    rm -rf "$dest"
    mv "$backup" "$dest" || echo "could not restore old application: $backup" >&2
    exit 1
fi
rm -rf "$backup" "$work"
if [ "$relaunch" = 1 ]; then /usr/bin/open "$dest"; fi
"#
}

#[cfg(target_os = "macos")]
fn find_apps(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("Could not inspect the update: {e}"))? {
        let entry = entry.map_err(|e| format!("Could not inspect the update: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("Could not inspect the update: {e}"))?;
        if kind.is_dir() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "app") {
                found.push(path);
            } else {
                find_apps(&path, found)?;
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply(archive: &Path, cache: &Path, relaunch: bool) -> Result<(), String> {
    let dest = bundle_path()?;
    use std::os::unix::process::CommandExt;
    let parent = dest
        .parent()
        .ok_or("Could not find the application's folder.")?;
    if !writable(parent) {
        return Err("The application folder is not writable.".into());
    }
    let work = parent.join(format!(
        ".galeria-update-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    fs::create_dir(&work).map_err(|e| format!("Could not stage the update: {e}"))?;
    let result = (|| {
        let status = Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(&work)
            .status()
            .map_err(|e| format!("Could not extract the update: {e}"))?;
        if !status.success() {
            return Err("Could not extract the update archive.".into());
        }
        let mut apps = Vec::new();
        find_apps(&work, &mut apps)?;
        if apps.len() != 1 {
            return Err("The update archive must contain exactly one application.".into());
        }
        let staged = &apps[0];
        let status = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(staged)
            .status()
            .map_err(|e| format!("Could not verify the application signature: {e}"))?;
        if !status.success() {
            return Err("The application signature could not be verified.".into());
        }
        let logs = cache.join("updates");
        fs::create_dir_all(&logs).map_err(|e| format!("Could not create the update log: {e}"))?;
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs.join("helper.log"))
            .map_err(|e| format!("Could not open the update log: {e}"))?;
        let stderr = log
            .try_clone()
            .map_err(|e| format!("Could not open the update log: {e}"))?;
        Command::new("/usr/bin/nohup")
            .arg("/bin/sh")
            .arg("-c")
            .arg(helper_script())
            .arg("galeria-update")
            .arg(std::process::id().to_string())
            .arg(&dest)
            .arg(staged)
            .arg(if relaunch { "1" } else { "0" })
            .arg(&work)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| format!("Could not start the update helper: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&work);
    }
    result
}

#[cfg(target_os = "linux")]
fn find_binary(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("Could not inspect the update: {e}"))? {
        let entry = entry.map_err(|e| format!("Could not inspect the update: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("Could not inspect the update: {e}"))?;
        if kind.is_dir() {
            find_binary(&entry.path(), found)?;
        } else if kind.is_file() && entry.file_name() == "galeria" {
            found.push(entry.path());
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply(archive: &Path, cache: &Path, relaunch: bool) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()
        .map_err(|e| format!("Could not find the running application: {e}"))?;
    let work = cache.join("updates").join(format!(
        "stage-{}-{}",
        std::process::id(),
        fastrand::u64(..)
    ));
    fs::create_dir_all(&work).map_err(|e| format!("Could not stage the update: {e}"))?;
    let result = (|| {
        let status = Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(&work)
            .status()
            .map_err(|e| format!("Could not extract the update: {e}"))?;
        if !status.success() {
            return Err("Could not extract the update archive.".into());
        }
        let mut binaries = Vec::new();
        find_binary(&work, &mut binaries)?;
        if binaries.len() != 1 {
            return Err("The update archive must contain exactly one Galeria executable.".into());
        }
        let mut old_name = exe.as_os_str().to_os_string();
        old_name.push(".old");
        let old = PathBuf::from(old_name);
        if old.exists() {
            return Err("An old Galeria executable already exists beside the application.".into());
        }
        fs::rename(&exe, &old).map_err(|e| format!("Could not move the old application: {e}"))?;
        let replacement = fs::copy(&binaries[0], &exe)
            .and_then(|_| fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)));
        if let Err(e) = replacement {
            let _ = fs::remove_file(&exe);
            if let Err(restore) = fs::rename(&old, &exe) {
                log::warn!("could not restore old executable: {restore}");
            }
            return Err(format!("Could not install the update: {e}"));
        }
        if relaunch {
            let spawned = Command::new("/usr/bin/nohup")
                .arg("/bin/sh")
                .arg("-c")
                .arg("while kill -0 \"$1\" 2>/dev/null; do sleep 1; done; exec \"$2\"")
                .arg("galeria-update")
                .arg(std::process::id().to_string())
                .arg(&exe)
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            if let Err(e) = spawned {
                let _ = fs::remove_file(&exe);
                if let Err(restore) = fs::rename(&old, &exe) {
                    log::warn!("could not restore old executable: {restore}");
                }
                return Err(format!("Could not restart Galeria: {e}"));
            }
        }
        if let Err(e) = fs::remove_file(&old) {
            log::warn!("could not remove old executable: {e}");
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&work);
    result
}

#[cfg(target_os = "windows")]
fn apply(archive: &Path, _cache: &Path, _relaunch: bool) -> Result<(), String> {
    Command::new(archive)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/CLOSEAPPLICATIONS",
            "/RESTARTAPPLICATIONS",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not start the update installer: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn apply(_archive: &Path, _cache: &Path, _relaunch: bool) -> Result<(), String> {
    Err("Automatic updates are not supported on this platform.".into())
}

pub fn cli(args: &[String]) -> Result<(), String> {
    match args {
        [check] if check == "--check" => cli_check(false),
        [check, json] if check == "--check" && json == "--json" => cli_check(true),
        [apply_flag, zip] if apply_flag == "--apply" => cli_apply(Path::new(zip), true),
        [apply_flag, zip, no_relaunch]
            if apply_flag == "--apply" && no_relaunch == "--no-relaunch" =>
        {
            cli_apply(Path::new(zip), false)
        }
        _ => Err(
            "Usage: galeria update --check [--json] | --apply <archive> [--no-relaunch].".into(),
        ),
    }
}

fn cli_check(json: bool) -> Result<(), String> {
    let paths =
        core::Paths::resolve().map_err(|e| format!("Could not find Galeria's settings: {e}"))?;
    let app = AppHandle::new(core::Core::new(paths));
    let status = check_blocking(&app, true);
    if json {
        println!(
            "{}",
            serde_json::to_string(&status)
                .map_err(|e| format!("Could not print update status: {e}"))?
        );
    } else {
        match status.state {
            State::Available { version, .. } => println!("Galeria {version} is available."),
            State::UpToDate { .. } => println!("Galeria is up to date."),
            State::Failed { error } => println!("{error}"),
            _ => println!("An update check is already in progress."),
        }
    }
    Ok(())
}

fn cli_apply(archive: &Path, relaunch: bool) -> Result<(), String> {
    let manifest = host::block_on(fetch_manifest())?;
    let key = platform_key().ok_or("Automatic updates are not supported on this platform.")?;
    let asset = manifest
        .platforms
        .get(key)
        .ok_or("No update is available for this platform.")?;
    verify_asset(archive, asset)?;
    let paths =
        core::Paths::resolve().map_err(|e| format!("Could not find Galeria's cache: {e}"))?;
    apply(archive, &paths.cache, relaunch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1"), Some((1, 0, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        for bad in [
            "",
            "v",
            ".1",
            "1.",
            "1..2",
            "1.2.3.4",
            "1.0-beta",
            "-1",
            "18446744073709551616",
        ] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
        assert!(newer("1.10", "1.2.9"));
        assert!(newer("2", "1.99.99"));
        assert!(!newer("1.2", "1.2.0"));
        assert!(!newer("bad", "1"));
    }

    #[test]
    fn feed_parses_optional_signature_and_unknown_platform() {
        let manifest: Manifest = serde_json::from_str(r#"{"version":"0.3.0","platforms":{"macos-aarch64":{"url":"https://example.test/a","sha256":"abc","signature":"signed"},"future-platform":{"url":"https://example.test/b","sha256":"def"}}}"#).expect("manifest");
        assert_eq!(
            manifest.platforms["macos-aarch64"].signature.as_deref(),
            Some("signed")
        );
        assert_eq!(manifest.platforms["future-platform"].signature, None);
    }

    #[test]
    fn settings_defaults_and_status_json() {
        let settings: Settings = serde_json::from_str("{}").expect("settings");
        assert!(settings.check_on_launch);
        let status = Status {
            state: State::UpToDate { checked_at: 123 },
            current: "1.0.0".into(),
            can_install: true,
            hint: None,
        };
        assert_eq!(
            serde_json::to_value(status).expect("status"),
            serde_json::json!({
                "state": {"state": "upToDate", "checkedAt": 123},
                "current": "1.0.0", "canInstall": true, "hint": null
            })
        );
    }

    #[test]
    fn sha256_and_signatures() {
        let path = std::env::temp_dir().join(format!(
            "galeria-update-test-{}-{}",
            std::process::id(),
            fastrand::u64(..)
        ));
        fs::write(&path, b"abc").expect("write test file");
        assert_eq!(
            sha256_file(&path).expect("hash"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::remove_file(path).expect("remove test file");
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
        let signature =
            base64::engine::general_purpose::STANDARD.encode(signing.sign(b"abc").to_bytes());
        assert!(verify_signature(b"abc", &public, Some(&signature)).is_ok());
        assert!(verify_signature(b"abd", &public, Some(&signature)).is_err());
        assert!(verify_signature(b"abc", &public, Some("invalid")).is_err());
        assert!(verify_signature(b"abc", "invalid", Some(&signature)).is_err());
        assert!(verify_signature(b"abc", &public, None).is_err());
    }

    #[test]
    fn helper_waits_and_uses_arguments() {
        let script = helper_script();
        assert!(script.contains("pid=$1"));
        assert!(script.contains("dest=$2"));
        assert!(script.contains("staged=$3"));
        assert!(script.contains("while kill -0 \"$pid\""));
        assert!(script.contains("mv \"$dest\" \"$backup\""));
        assert!(script.contains("mv \"$backup\" \"$dest\""));
    }
}
