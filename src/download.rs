//! Downloading paintings into the library folder.
//!
//! A download never trusts what the window sends: it re-reads the record
//! from the museum, re-evaluates the license, and only then fetches the
//! image. The file is verified as a real image, saved next to the others and
//! added to `manifest.json` together with the license it was downloaded under
//! and the evidence for it.

use crate::core;
use crate::host::{self, AppHandle};
use crate::sources::{self, Candidate, Ctx};
use crate::{genres, images, licensing, manifest, tone};
use fast_image_resize as fr;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

const MAX_BYTES: u64 = 700 * 1024 * 1024;
/// Anything smaller is an error page or a thumbnail, not a painting scan.
const MIN_LONG_EDGE: u32 = 1000;
const JPEG_QUALITY: u8 = 92;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum State {
    Queued,
    /// Re-reading the museum's record and its license.
    Checking,
    Downloading {
        received: u64,
        total: Option<u64>,
    },
    Processing,
    Done {
        file: String,
        width: u32,
        height: u32,
        bytes: u64,
        license: String,
    },
    Failed {
        error: String,
    },
    Skipped {
        reason: String,
    },
}

impl State {
    fn finished(&self) -> bool {
        matches!(
            self,
            State::Done { .. } | State::Failed { .. } | State::Skipped { .. }
        )
    }
}

/// What the window asks for: where to find the record, plus labels for the queue.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub source: String,
    pub locator: String,
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub thumb_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub source: String,
    pub locator: String,
    pub title: String,
    pub artist: Option<String>,
    pub thumb_url: Option<String>,
    #[serde(flatten)]
    pub state: State,
}

#[derive(Default)]
pub struct Queue {
    jobs: Mutex<Vec<Job>>,
    wake: Notify,
    cancel: AtomicBool,
    last: Mutex<HashMap<String, Instant>>,
}

impl Queue {
    pub fn list(&self) -> Vec<Job> {
        self.jobs.lock().unwrap().clone()
    }

    pub fn add(&self, requests: Vec<Request>) -> usize {
        let mut jobs = self.jobs.lock().unwrap();
        let mut added = 0;
        for r in requests {
            // Asking again for something that failed retries it.
            jobs.retain(|j| {
                !(j.id == r.id && matches!(j.state, State::Failed { .. } | State::Skipped { .. }))
            });
            if jobs.iter().any(|j| j.id == r.id) || !sources::SOURCES.contains(&r.source.as_str()) {
                continue;
            }
            jobs.push(Job {
                id: r.id,
                source: r.source,
                locator: r.locator,
                title: r.title,
                artist: r.artist,
                thumb_url: r.thumb_url,
                state: State::Queued,
            });
            added += 1;
        }
        drop(jobs);
        self.wake.notify_one();
        added
    }

    /// Cancel one job (or everything that has not finished).
    pub fn cancel(&self, id: Option<&str>) {
        let mut jobs = self.jobs.lock().unwrap();
        let active = |j: &Job| {
            matches!(
                j.state,
                State::Checking | State::Downloading { .. } | State::Processing
            )
        };
        if jobs
            .iter()
            .any(|j| active(j) && id.is_none_or(|id| j.id == id))
        {
            self.cancel.store(true, Ordering::SeqCst);
        }
        jobs.retain(|j| !(j.state == State::Queued && id.is_none_or(|id| j.id == id)));
    }

    pub fn clear_finished(&self) {
        self.jobs.lock().unwrap().retain(|j| !j.state.finished());
    }

    /// Drop finished jobs for deleted paintings so Discover offers them again.
    pub fn forget(&self, ids: &HashSet<String>) {
        self.jobs
            .lock()
            .unwrap()
            .retain(|j| !(j.state.finished() && ids.contains(&j.id)));
    }

    fn set(&self, id: &str, state: State) {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            j.state = state;
        }
    }

    fn next(&self) -> Option<Job> {
        let mut jobs = self.jobs.lock().unwrap();
        let j = jobs.iter_mut().find(|j| j.state == State::Queued)?;
        j.state = State::Checking;
        Some(j.clone())
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let jobs = self.jobs.lock().unwrap();
        let done = jobs
            .iter()
            .filter(|j| matches!(j.state, State::Done { .. }))
            .count();
        let failed = jobs
            .iter()
            .filter(|j| matches!(j.state, State::Failed { .. }))
            .count();
        (
            jobs.iter().filter(|j| !j.state.finished()).count(),
            done,
            failed,
        )
    }
}

/// Tell the windows the queue changed.
pub fn notify(app: &AppHandle) {
    if let Some(shell) = app.shell() {
        shell.downloads(app.core().downloads.list());
    }
}

fn emit(app: &AppHandle) {
    notify(app);
}

pub async fn worker(app: AppHandle) {
    loop {
        let core = app.core();
        let Some(job) = core.downloads.next() else {
            core.downloads.wake.notified().await;
            continue;
        };
        core.downloads.cancel.store(false, Ordering::SeqCst);
        emit(&app);
        let state = match run(&app, &job).await {
            Ok(s) => s,
            Err(error) => State::Failed { error },
        };
        log::info!("download {}: {:?}", job.id, state);
        let done = matches!(state, State::Done { .. });
        core.downloads.set(&job.id, state);
        emit(&app);
        if done {
            // Saved whole and renamed into place: it can go on the wall at once.
            core.lock().complete.insert(job.id.clone());
            let a = app.clone();
            let _ = host::spawn_blocking(move || core::reload_library(&a)).await;
        }
        core::publish(&app);
    }
}

async fn run(app: &AppHandle, job: &Job) -> Result<State, String> {
    let core = app.core();
    let (root, max_edge, known) = {
        let i = core.lock();
        (
            i.settings.library_path.clone(),
            i.settings.download.max_long_edge,
            i.library
                .contains_candidate(&job.id, &job.source, &job.locator),
        )
    };
    let root = root
        .filter(|r| r.is_dir())
        .ok_or("Choose an art folder first: downloads are saved next to your other paintings.")?;
    if !root.join(crate::library::MANIFEST).exists() {
        return Err(
            "The art folder has no manifest.json for Galeria to add the painting to.".into(),
        );
    }
    if known {
        return Ok(State::Skipped {
            reason: "Already in your library.".into(),
        });
    }

    // 1. The museum's own record, read now, decides whether this is allowed.
    let ctx = core.source_ctx(app);
    let cand = sources::resolve(&ctx, &job.source, &job.locator).await?;
    if cand.id != job.id {
        return Err("The museum's record no longer matches this painting.".into());
    }
    let decision = cand
        .decision
        .clone()
        .ok_or("No license decision could be made for this record.")?;
    if !decision.allowed {
        return Err(format!("Not downloaded. {}", decision.reason));
    }
    let plan = cand
        .plan
        .clone()
        .ok_or("The museum's record has no image to download.")?;

    // 2. Be a polite client: one download at a time, spaced as each museum asks.
    let gap = Duration::from_millis(
        licensing::source(&job.source)
            .map(|s| s.etiquette.download_interval_ms)
            .unwrap_or(1000),
    );
    let wait = core
        .downloads
        .last
        .lock()
        .unwrap()
        .get(&job.source)
        .map(|t| gap.saturating_sub(t.elapsed()));
    if let Some(w) = wait.filter(|w| !w.is_zero()) {
        tokio::time::sleep(w).await;
    }

    // 3. Fetch into a hidden temp file inside the library folder (same volume → atomic rename).
    let tmp = root.join(format!(".galeria-download-{:08x}.part", fastrand::u32(..)));
    let fetched = fetch(app, job, &ctx, &plan, &tmp).await;
    core.downloads
        .last
        .lock()
        .unwrap()
        .insert(job.source.clone(), Instant::now());
    if let Err(e) = fetched {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // 4. Verify, convert if needed, measure, save, record.
    core.downloads.set(&job.id, State::Processing);
    emit(app);
    let backups = core.paths.manifest_backups.clone();
    let journal = core.paths.downloads_journal.clone();
    let (tmp2, root2) = (tmp.clone(), root.clone());
    let saved =
        host::spawn_blocking(move || save(&tmp2, &root2, &cand, max_edge, &backups, &journal))
            .await
            .map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&tmp);
    saved?
}

async fn fetch(
    app: &AppHandle,
    job: &Job,
    ctx: &Ctx,
    plan: &sources::Plan,
    tmp: &Path,
) -> Result<(), String> {
    let core = app.core();
    let headers: Vec<(&str, &str)> = plan.headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
    // Large originals can take minutes; the shared client's timeout is for API calls.
    let mut req = ctx
        .http
        .client()
        .get(&plan.url)
        .timeout(Duration::from_secs(900));
    for (k, v) in &headers {
        req = req.header(*k, *v);
    }
    let host = plan
        .url
        .split('/')
        .nth(2)
        .unwrap_or("the museum")
        .to_string();
    let mut resp = req
        .send()
        .await
        .map_err(|e| format!("Could not download from {host}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            403 | 429 => format!(
                "{host} is limiting downloads right now (HTTP {}). Try again in a few minutes.",
                status.as_u16()
            ),
            code => format!("{host} answered with HTTP {code}."),
        });
    }
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if ctype.starts_with("text/") || ctype.contains("json") || ctype.contains("html") {
        return Err(format!("{host} sent a web page instead of an image."));
    }
    let total = resp.content_length();
    if total.is_some_and(|t| t > MAX_BYTES) {
        return Err("The file is larger than 700 MB.".into());
    }
    let mut file = std::fs::File::create(tmp)
        .map_err(|e| format!("Could not write to the art folder: {e}"))?;
    let (mut received, mut last) = (0u64, Instant::now());
    core.downloads
        .set(&job.id, State::Downloading { received: 0, total });
    emit(app);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("The download was interrupted: {e}"))?
    {
        if core.downloads.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled.".into());
        }
        file.write_all(&chunk)
            .map_err(|e| format!("Could not write to the art folder: {e}"))?;
        received += chunk.len() as u64;
        if received > MAX_BYTES {
            return Err("The file is larger than 700 MB.".into());
        }
        if last.elapsed() > Duration::from_millis(150) {
            last = Instant::now();
            core.downloads
                .set(&job.id, State::Downloading { received, total });
            emit(app);
        }
    }
    file.sync_all().ok();
    Ok(())
}

fn unique_name(root: &Path, c: &Candidate) -> String {
    let title = crate::util::slug(&c.title, 70);
    let artist = c
        .artist
        .as_deref()
        .map(|a| crate::util::slug(a, 40))
        .filter(|a| !a.is_empty());
    let stem = match (artist, title.is_empty()) {
        (Some(a), false) => format!("{a}__{title}"),
        (None, false) => title,
        // Titles that fold to nothing (other scripts): name the file after its source id.
        (a, true) => format!(
            "{}{}-{}",
            a.map(|a| format!("{a}__")).unwrap_or_default(),
            c.source,
            crate::util::hash_str(&[&c.source_id])
        ),
    };
    let mut name = format!("{stem}.jpg");
    let mut n = 2;
    while root.join(&name).exists() {
        name = format!("{stem}-{n}.jpg");
        n += 1;
    }
    name
}

fn year_hint(date: Option<&str>) -> Value {
    let Some(d) = date else { return Value::Null };
    let b = d.as_bytes();
    (0..b.len().saturating_sub(3))
        .find(|&i| {
            b[i..i + 4].iter().all(u8::is_ascii_digit)
                && (i == 0 || !b[i - 1].is_ascii_digit())
                && b.get(i + 4).is_none_or(|c| !c.is_ascii_digit())
        })
        .and_then(|i| d[i..i + 4].parse::<u32>().ok())
        .map(Value::from)
        .unwrap_or(Value::Null)
}

/// Verify the downloaded bytes are a painting-sized image, store it as JPEG and add it to the manifest.
fn save(
    tmp: &Path,
    root: &Path,
    c: &Candidate,
    max_edge: u32,
    backups: &Path,
    journal: &Path,
) -> Result<State, String> {
    let plan = c.plan.as_ref().ok_or("no download plan")?;
    let decision = c.decision.as_ref().ok_or("no license decision")?;
    let format = image::ImageReader::open(tmp)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?
        .format()
        .ok_or("The downloaded file is not an image (the museum may have sent an error page).")?;
    let (img, icc) = images::decode(tmp)
        .map_err(|e| format!("The downloaded file could not be read as an image: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w.max(h) < MIN_LONG_EDGE {
        return Err(format!(
            "The museum only provided a {w}×{h} px image, which is too small for a wallpaper."
        ));
    }
    let measure = tone::measure_dynamic(&img);

    let name = unique_name(root, c);
    let dest = root.join(&name);
    let part = root.join(format!(".{name}.part"));
    let (fw, fh) = sources::fit_long_edge(w, h, max_edge);
    let untouched = format == image::ImageFormat::Jpeg && (fw, fh) == (w, h);
    let write = if untouched {
        // Already a JPEG of acceptable size: keep the museum's bytes exactly.
        drop(img);
        std::fs::copy(tmp, &part).map(|_| ())
    } else {
        let rgb = images::resize_rgb(
            img,
            fw,
            fh,
            fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3),
        )?;
        let bytes = images::encode_jpeg(&rgb, fw, fh, icc, JPEG_QUALITY)?;
        std::fs::write(&part, bytes)
    };
    write
        .and_then(|_| std::fs::rename(&part, &dest))
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("Could not save the painting to the art folder: {e}")
        })?;
    let bytes = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);

    let entry = manifest_entry(c, plan, decision, &name, &dest, fw, fh, bytes, &measure);
    if let Err(e) = manifest::append(root, std::slice::from_ref(&entry), Some(backups)) {
        // Without a manifest entry the file would be an unlabelled stray: undo.
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }
    journal_add(journal, &entry);
    Ok(State::Done {
        file: name,
        width: fw,
        height: fh,
        bytes,
        license: decision.license.clone(),
    })
}

/// A manifest entry in the shape (and key order) existing manifests use.
#[allow(clippy::too_many_arguments)]
fn manifest_entry(
    c: &Candidate,
    plan: &sources::Plan,
    d: &licensing::Decision,
    name: &str,
    dest: &Path,
    w: u32,
    h: u32,
    bytes: u64,
    m: &tone::Measure,
) -> Map<String, Value> {
    let aspect = w as f64 / h as f64;
    let mut e = Map::new();
    let mut put = |k: &str, v: Value| {
        e.insert(k.to_string(), v);
    };
    let opt = |s: &Option<String>| s.clone().map(Value::from).unwrap_or(Value::Null);
    put("file", name.into());
    put("title", c.title.clone().into());
    put("artist", opt(&c.artist));
    put("date", opt(&c.date));
    put("medium", opt(&c.medium));
    if c.dimensions.is_some() {
        put("dimensions", opt(&c.dimensions));
    }
    put(
        "kind",
        plan.kind
            .clone()
            .unwrap_or_else(|| "painting".into())
            .into(),
    );
    put(
        "category",
        if c.genres.iter().any(|g| g == "orthodox") {
            "orthodox"
        } else {
            "general"
        }
        .into(),
    );
    put("tags", genres::tags_for(&c.genres).into());
    put("width", w.into());
    put("height", h.into());
    put("aspect", ((aspect * 100.0).round() / 100.0).into());
    put(
        "orientation",
        if w >= h { "landscape" } else { "portrait" }.into(),
    );
    put(
        "megapixels",
        (((w as f64 * h as f64) / 1e6 * 10.0).round() / 10.0).into(),
    );
    put("bytes", bytes.into());
    put("museum", c.museum.clone().into());
    put("source", c.source.clone().into());
    put("source_id", c.source_id.clone().into());
    put("source_page", opt(&c.page_url));
    put("download_url", plan.url.clone().into());
    put("license", d.license.clone().into());
    put("credit", plan.credit.clone().into());
    put("added", crate::util::today().into());
    put("brightness", m.brightness.into());
    put("contrast", m.contrast.into());
    put("saturation", m.saturation.into());
    put("id", c.id.clone().into());
    put("path", dest.to_string_lossy().to_string().into());
    put(
        "tone_flags",
        tone::tone_flags(m, aspect)
            .into_iter()
            .map(Value::from)
            .collect::<Vec<_>>()
            .into(),
    );
    put("year_hint", year_hint(c.date.as_deref()));
    if c.title_original.is_some() {
        put("title_original", opt(&c.title_original));
    }
    put("attribution_required", d.attribution_required.into());
    for (k, v) in &plan.extra {
        if k != "rights_statement" {
            put(k, v.clone());
        }
    }
    // What the plate can show without looking anything up.
    for (k, v) in [
        ("artist_bio", &c.artist_bio),
        ("description", &c.description),
        ("credit_line", &c.credit_line),
        ("accession", &c.accession),
    ] {
        if v.is_some() {
            put(k, opt(v));
        }
    }
    // Why Galeria was allowed to download this, and when it checked.
    put("license_url", opt(&d.license_url));
    put("license_evidence", d.evidence.clone().into());
    put("license_checked", crate::util::today().into());
    put("added_by", "galeria".into());
    e
}

fn journal_read(journal: &Path) -> Vec<Map<String, Value>> {
    std::fs::read(journal)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn journal_add(journal: &Path, entry: &Map<String, Value>) {
    let mut all = journal_read(journal);
    all.retain(|e| e.get("id") != entry.get("id"));
    all.push(entry.clone());
    let _ = crate::settings::write_json_atomic(journal, &all);
}

/// Forget deleted paintings in both the download journal and the finished queue.
pub fn forget(journal: &Path, queue: &Queue, ids: &HashSet<String>) {
    queue.forget(ids);
    let all = journal_read(journal);
    if all.is_empty() {
        return;
    }
    let kept: Vec<Map<String, Value>> = all
        .into_iter()
        .filter(|e| {
            !e.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| ids.contains(id))
        })
        .collect();
    let _ = crate::settings::write_json_atomic(journal, &kept);
}

/// Another tool may rewrite `manifest.json` from its own copy and drop what
/// Galeria added. If a painting Galeria downloaded is still on disk but no
/// longer listed, list it again. Entries whose file the user deleted are forgotten.
pub fn heal(root: &Path, journal: &Path, backups: &Path) {
    let all = journal_read(journal);
    if all.is_empty() {
        return;
    }
    let here: Vec<Map<String, Value>> = all
        .iter()
        .filter(|e| {
            e.get("path")
                .and_then(|p| p.as_str())
                .is_some_and(|p| Path::new(p).starts_with(root))
        })
        .cloned()
        .collect();
    let (alive, gone): (Vec<_>, Vec<_>) = here.into_iter().partition(|e| {
        e.get("file")
            .and_then(|f| f.as_str())
            .is_some_and(|f| root.join(f).exists())
    });
    if !gone.is_empty() {
        let ids: Vec<&Value> = gone.iter().filter_map(|e| e.get("id")).collect();
        let kept: Vec<Map<String, Value>> = all
            .iter()
            .filter(|e| !e.get("id").is_some_and(|id| ids.contains(&id)))
            .cloned()
            .collect();
        let _ = crate::settings::write_json_atomic(journal, &kept);
    }
    if alive.is_empty() {
        return;
    }
    match manifest::append(root, &alive, Some(backups)) {
        Ok(n) if n > 0 => log::info!("restored {n} downloaded painting(s) to the manifest"),
        Ok(_) => {}
        Err(e) => log::debug!("manifest heal skipped: {e}"),
    }
}

/// Temp files a crash may have left behind.
pub fn sweep(root: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let stale = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|a| a > Duration::from_secs(3600));
        if name.starts_with('.') && name.ends_with(".part") && stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::licensing::Evidence;
    use std::path::PathBuf;

    fn candidate() -> Candidate {
        let mut ev = Evidence::new();
        ev.insert("share_license_status".into(), "CC0".into());
        let mut extra = Map::new();
        extra.insert("department".into(), "European Painting".into());
        Candidate {
            id: "cma:1958.39".into(),
            source: "cma".into(),
            source_id: "1958.39".into(),
            locator: "1958.39".into(),
            title: "Winter Landscape near Giverny".into(),
            artist: Some("Claude Monet".into()),
            artist_bio: Some("French, 1840–1926".into()),
            date: Some("c. 1885–86".into()),
            medium: Some("oil on fabric".into()),
            museum: "Cleveland Museum of Art, Cleveland".into(),
            page_url: Some("https://www.clevelandart.org/art/1958.39".into()),
            genres: vec!["winter".into(), "landscape".into()],
            decision: Some(licensing::evaluate("cma", &ev)),
            plan: Some(sources::Plan {
                url: "https://openaccess-cdn.clevelandart.org/1958.39/1958.39_full.tif".into(),
                credit: "The Cleveland Museum of Art (CC0)".into(),
                kind: Some("painting".into()),
                extra,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn library() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("galeria-dl-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            "{\n \"items\": [],\n \"count\": 0\n}",
        )
        .unwrap();
        dir
    }

    #[test]
    fn saves_a_tiff_as_jpeg_and_records_it() {
        let dir = library();
        let tmp = dir.join(".dl.part");
        // A dark, wide "scan" in the format Cleveland serves.
        image::RgbImage::from_fn(2400, 1500, |x, y| {
            image::Rgb([((x / 40) % 60) as u8, 30, ((y / 30) % 50) as u8])
        })
        .save_with_format(&tmp, image::ImageFormat::Tiff)
        .unwrap();
        let journal = dir.join("journal.json");
        let state = save(
            &tmp,
            &dir,
            &candidate(),
            2000,
            &dir.join("backups"),
            &journal,
        )
        .unwrap();
        let State::Done {
            file,
            width,
            height,
            ..
        } = state
        else {
            panic!("{state:?}")
        };
        assert_eq!(file, "claude-monet__winter-landscape-near-giverny.jpg");
        assert_eq!(
            (width, height),
            (2000, 1250),
            "scaled to the configured long edge"
        );
        let saved = image::open(dir.join(&file)).unwrap();
        assert_eq!((saved.width(), saved.height()), (2000, 1250));

        let m: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        let e = &m["items"][0];
        assert_eq!(m["count"], 1);
        assert_eq!(e["id"], "cma:1958.39");
        assert_eq!(e["license"], "CC0");
        assert_eq!(e["license_evidence"], "share_license_status = CC0");
        assert_eq!(e["attribution_required"], false);
        assert_eq!(e["tags"], serde_json::json!(["winter", "landscape"]));
        assert_eq!(e["orientation"], "landscape");
        assert_eq!(e["aspect"], 1.6);
        assert_eq!(e["year_hint"], 1885);
        assert_eq!(e["department"], "European Painting");
        assert_eq!(e["artist_bio"], "French, 1840–1926");
        assert!(e["brightness"].as_f64().unwrap() < 60.0);
        assert!(e["tone_flags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "dark"));
        assert_eq!(e["added_by"], "galeria");
        // The library loader accepts what was written.
        let lib = crate::library::load(&dir, &mut crate::library::DimCache::default());
        assert_eq!(lib.paintings.len(), 1);
        assert_eq!(lib.paintings[0].genres, vec!["landscape", "winter"]);
        assert!(lib.paintings[0].brightness.is_some());

        // A second download of the same title never overwrites the first file.
        image::RgbImage::from_pixel(1800, 1200, image::Rgb([240, 240, 240]))
            .save_with_format(&tmp, image::ImageFormat::Jpeg)
            .unwrap();
        let mut other = candidate();
        other.id = "cma:2".into();
        other.source_id = "2".into();
        let State::Done {
            file: second,
            width,
            ..
        } = save(&tmp, &dir, &other, 8000, &dir.join("backups"), &journal).unwrap()
        else {
            panic!()
        };
        assert_eq!(second, "claude-monet__winter-landscape-near-giverny-2.jpg");
        assert_eq!(width, 1800, "a JPEG within the size limit is kept as is");
        assert_eq!(
            std::fs::read(dir.join(&second)).unwrap(),
            std::fs::read(&tmp).unwrap(),
            "byte-identical to what the museum sent"
        );

        // If another tool rewrites the manifest without Galeria's entries, they come back…
        std::fs::write(
            dir.join("manifest.json"),
            "{\n \"items\": [],\n \"count\": 0\n}",
        )
        .unwrap();
        heal(&dir, &journal, &dir.join("backups"));
        let m: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["items"].as_array().unwrap().len(), 2);
        // …unless the user deleted the file.
        std::fs::remove_file(dir.join(&second)).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            "{\n \"items\": [],\n \"count\": 0\n}",
        )
        .unwrap();
        heal(&dir, &journal, &dir.join("backups"));
        let m: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["items"].as_array().unwrap().len(), 1);
        assert_eq!(journal_read(&journal).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_error_pages_and_thumbnails() {
        let dir = library();
        let tmp = dir.join(".dl.part");
        std::fs::write(&tmp, b"<html><body>403 Forbidden</body></html>").unwrap();
        let e = save(
            &tmp,
            &dir,
            &candidate(),
            8000,
            &dir.join("b"),
            &dir.join("j.json"),
        )
        .unwrap_err();
        assert!(e.contains("not an image"), "{e}");
        image::RgbImage::from_pixel(600, 400, image::Rgb([9, 9, 9]))
            .save_with_format(&tmp, image::ImageFormat::Jpeg)
            .unwrap();
        let e = save(
            &tmp,
            &dir,
            &candidate(),
            8000,
            &dir.join("b"),
            &dir.join("j.json"),
        )
        .unwrap_err();
        assert!(e.contains("too small"), "{e}");
        let m: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(m["items"].as_array().unwrap().len(), 0);
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".jpg"))
                .count(),
            0
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names_and_years() {
        let dir = library();
        let mut c = candidate();
        c.title = "Явление Христа народу".into();
        c.artist = None;
        assert_eq!(unique_name(&dir, &c), "iavlenie-khrista-narodu.jpg");
        c.title = "山水".into();
        c.artist = Some("雪舟".into());
        let n = unique_name(&dir, &c);
        assert!(n.ends_with(".jpg") && n.len() > 12, "{n}");
        assert_eq!(year_hint(Some("c. 1885–86")), 1885);
        assert_eq!(year_hint(Some("early 1640s")), 1640);
        assert_eq!(year_hint(Some("19th century")), Value::Null);
        assert_eq!(year_hint(None), Value::Null);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn queue_bookkeeping() {
        let q = Queue::default();
        let req = |id: &str| Request {
            source: "cma".into(),
            locator: id.into(),
            id: format!("cma:{id}"),
            title: id.into(),
            artist: None,
            thumb_url: None,
        };
        assert_eq!(q.add(vec![req("1"), req("2"), req("1")]), 2);
        let mut bad = req("3");
        bad.source = "somewhere-else".into();
        assert_eq!(q.add(vec![bad]), 0, "unknown sources never enter the queue");
        let first = q.next().unwrap();
        assert_eq!(first.id, "cma:1");
        q.set("cma:1", State::Failed { error: "x".into() });
        assert_eq!(q.counts(), (1, 0, 1));
        assert_eq!(q.add(vec![req("1")]), 1, "a failed download can be retried");
        q.cancel(None);
        assert_eq!(q.list().len(), 0);
    }
}
