//! The images the windows draw: thumbnails, display-sized renditions, museum
//! search results and the plate's QR code, decoded off the UI thread into
//! [`rune::Pixels`]. A window that shows one redraws by itself when it arrives.

use crate::host::{self, AppHandle};
use crate::images::{self, Rgba};
use rune::{Color, Pixels};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use tokio::sync::Semaphore;

/// Decoding a 40-megapixel scan is heavy on memory and CPU: only a few at once.
static HEAVY: LazyLock<Semaphore> = LazyLock::new(|| {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    Semaphore::new((n / 3).clamp(1, 3))
});
static THUMBS: LazyLock<Semaphore> = LazyLock::new(|| {
    let n = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    Semaphore::new((n / 2).clamp(2, 6))
});
static REMOTE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(6));

/// Decoded thumbnails kept in memory, counted in pixels' bytes.
const THUMB_BUDGET: usize = 160 << 20;
const REMOTE_BUDGET: usize = 96 << 20;

struct Entry {
    pixels: Pixels,
    used: u64,
    bytes: usize,
}

/// Least-recently-drawn images go first once the budget is full.
struct Lru {
    map: HashMap<String, Entry>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl Lru {
    fn new(budget: usize) -> Lru {
        Lru {
            map: HashMap::new(),
            tick: 0,
            bytes: 0,
            budget,
        }
    }

    fn get(&mut self, key: &str) -> Option<Pixels> {
        self.tick += 1;
        let e = self.map.get_mut(key)?;
        e.used = self.tick;
        Some(e.pixels.clone())
    }

    fn insert(&mut self, key: String, pixels: Pixels, bytes: usize) {
        self.tick += 1;
        self.bytes += bytes;
        self.map.insert(
            key,
            Entry {
                pixels,
                used: self.tick,
                bytes,
            },
        );
        while self.bytes > self.budget && self.map.len() > 1 {
            let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.map.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(e.bytes);
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.bytes = 0;
    }
}

pub struct ArtStore {
    app: AppHandle,
    /// Decode on the calling thread, so a headless render's first frame already
    /// has its pictures (`galeria ui`). Windows never do: they must not stall.
    immediate: bool,
    thumbs: Mutex<Lru>,
    remote: Mutex<Lru>,
    qr: Mutex<HashMap<String, Pixels>>,
}

/// Where a museum image is kept on disk: one file per address.
fn remote_file(cache: &std::path::Path, url: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut a = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut a);
    let mut b = std::collections::hash_map::DefaultHasher::new();
    (url, 1u8).hash(&mut b);
    cache
        .join("remote")
        .join(format!("{:016x}{:016x}", a.finish(), b.finish()))
}

/// Keep fetched bytes for next time. Written whole and moved into place, so a
/// half-written file is never read back.
fn keep_remote(file: &std::path::Path, bytes: &[u8]) {
    let Some(dir) = file.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = file.with_extension(format!("part{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, file).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn show(pixels: &Pixels, rgba: Rgba) {
    pixels.set(rgba.width, rgba.height, rgba.data);
}

impl ArtStore {
    pub fn new(app: AppHandle) -> ArtStore {
        ArtStore {
            app,
            immediate: false,
            thumbs: Mutex::new(Lru::new(THUMB_BUDGET)),
            remote: Mutex::new(Lru::new(REMOTE_BUDGET)),
            qr: Mutex::new(HashMap::new()),
        }
    }

    /// For headless renders: every image is decoded before it is handed out.
    pub fn immediate(mut self) -> ArtStore {
        self.immediate = true;
        self
    }

    /// A painting's thumbnail, at most `edge` pixels on its long side. `version`
    /// is anything that changes when the file does (its size in bytes). The
    /// image paints nothing until it has decoded.
    pub fn thumb(&self, id: &str, version: u64, edge: u32) -> Pixels {
        let key = format!("{id}\u{1f}{version}\u{1f}{edge}");
        let mut lru = self.thumbs.lock().unwrap();
        if let Some(p) = lru.get(&key) {
            return p;
        }
        let pixels = Pixels::new();
        // A 3:2 painting at this edge; exact sizes are not known until it decodes.
        let estimate = (edge as usize * edge as usize * 4) * 2 / 3;
        lru.insert(key.clone(), pixels.clone(), estimate);
        drop(lru);

        if self.immediate {
            let core = self.app.core();
            if let Some(Ok(rgba)) = core
                .source_for(id)
                .map(|src| images::thumb_pixels(&src, &core.paths.cache, edge))
            {
                show(&pixels, rgba);
            }
            return pixels;
        }
        let (app, id, out) = (self.app.clone(), id.to_string(), pixels.clone());
        host::spawn(async move {
            let _permit = THUMBS.acquire().await;
            let a = app.clone();
            let id2 = id.clone();
            let decoded = host::spawn_blocking(move || {
                let src = a.core().source_for(&id2).ok_or("unknown painting")?;
                images::thumb_pixels(&src, &a.core().paths.cache, edge)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            match decoded {
                Ok(rgba) => show(&out, rgba),
                Err(e) => log::debug!("thumbnail of {id}: {e}"),
            }
        });
        pixels
    }

    /// The painting scaled to `tw`×`th` pixels (see `images::target_size`).
    /// `done` runs on a worker thread once it has decoded, or failed.
    pub fn render(
        &self,
        id: &str,
        tw: u32,
        th: u32,
        done: impl FnOnce(Result<Pixels, String>) + Send + 'static,
    ) {
        let (app, id) = (self.app.clone(), id.to_string());
        host::spawn(async move {
            let _permit = HEAVY.acquire().await;
            let decoded = host::spawn_blocking(move || {
                let src = app.core().source_for(&id).ok_or("unknown painting")?;
                images::render_pixels(&src, tw, th, &app.core().paths.cache)
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            done(decoded.map(|rgba| {
                let pixels = Pixels::new();
                show(&pixels, rgba);
                pixels
            }));
        });
    }

    /// An image from a museum's server (a search result's thumbnail), at most
    /// `edge` pixels on its long side.
    pub fn remote(&self, url: &str, edge: u32) -> Pixels {
        let key = format!("{url}\u{1f}{edge}");
        let mut lru = self.remote.lock().unwrap();
        if let Some(p) = lru.get(&key) {
            return p;
        }
        let pixels = Pixels::new();
        lru.insert(
            key.clone(),
            pixels.clone(),
            (edge as usize * edge as usize * 4) * 2 / 3,
        );
        drop(lru);

        if !(url.starts_with("https://") || url.starts_with("http://")) {
            return pixels;
        }
        if self.immediate {
            // Headless renders stay off the network unless asked.
            if std::env::var_os("GALERIA_UI_NETWORK").is_some() {
                let http = self.app.core().http.clone();
                let fetched = host::block_on(async {
                    let resp = http.get(url, &[]).await?;
                    resp.bytes().await.map_err(|e| e.to_string())
                });
                if let Ok(rgba) = fetched.and_then(|bytes| images::decode_pixels(&bytes, edge)) {
                    show(&pixels, rgba);
                }
            }
            return pixels;
        }
        let (app, url, out) = (self.app.clone(), url.to_string(), pixels.clone());
        host::spawn(async move {
            let _permit = REMOTE.acquire().await;
            // Seen before, in this run or an earlier one: no need to ask the museum again.
            let file = remote_file(&app.core().paths.cache, &url);
            let kept = std::fs::read(&file).ok().filter(|b| !b.is_empty());
            let fetched = match kept {
                Some(bytes) => Ok(bytes),
                None => {
                    let fetched = async {
                        let resp = app.core().http.get(&url, &[]).await?;
                        resp.bytes()
                            .await
                            .map(|b| b.to_vec())
                            .map_err(|e| e.to_string())
                    }
                    .await;
                    if let Ok(bytes) = &fetched {
                        keep_remote(&file, bytes);
                    }
                    fetched
                }
            };
            let decoded = match fetched {
                Ok(bytes) => host::spawn_blocking(move || images::decode_pixels(&bytes, edge))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r),
                Err(e) => Err(e),
            };
            match decoded {
                Ok(rgba) => show(&out, rgba),
                Err(e) => log::debug!("image {url}: {e}"),
            }
        });
        pixels
    }

    /// A QR code for `text` in `ink` on a clear ground. `None` when it does not fit in one.
    pub fn qr(&self, text: &str, ink: Color) -> Option<Pixels> {
        let key = format!("{text}\u{1f}{}", ink.hex());
        if let Some(p) = self.qr.lock().unwrap().get(&key) {
            return Some(p.clone());
        }
        let rgba = qr_pixels(text, ink)?;
        let pixels = Pixels::new();
        show(&pixels, rgba);
        let mut cache = self.qr.lock().unwrap();
        if cache.len() > 64 {
            cache.clear();
        }
        cache.insert(key, pixels.clone());
        Some(pixels)
    }

    /// Forget every decoded thumbnail (after the cache folder was cleared).
    pub fn clear(&self) {
        self.thumbs.lock().unwrap().clear();
    }
}

/// Draw a QR code with no quiet zone: the plate around it is the quiet zone.
fn qr_pixels(text: &str, ink: Color) -> Option<Rgba> {
    use qrcode::{EcLevel, QrCode};
    const MODULE: u32 = 8;
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).ok()?;
    let n = code.width() as u32;
    let cells = code.to_colors();
    let enc = |c: f32| (rune::color::srgb_encode(c).clamp(0.0, 1.0) * 255.0).round() as u8;
    let on = [
        enc(ink.r),
        enc(ink.g),
        enc(ink.b),
        (ink.a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ];
    let size = n * MODULE;
    let mut data = vec![0u8; (size * size * 4) as usize];
    for (i, cell) in cells.iter().enumerate() {
        if *cell != qrcode::Color::Dark {
            continue;
        }
        let (cx, cy) = (i as u32 % n, i as u32 / n);
        for y in cy * MODULE..(cy + 1) * MODULE {
            let row = (y * size + cx * MODULE) as usize * 4;
            for px in data[row..row + (MODULE * 4) as usize]
                .as_chunks_mut::<4>()
                .0
            {
                *px = on;
            }
        }
    }
    Some(Rgba {
        width: size,
        height: size,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn museum_images_are_kept_on_disk_per_address() {
        let dir = std::env::temp_dir().join(format!("galeria-remote-{}", fastrand::u64(..)));
        let a = remote_file(&dir, "https://example.org/a.jpg");
        assert_ne!(a, remote_file(&dir, "https://example.org/b.jpg"));
        assert_eq!(a, remote_file(&dir, "https://example.org/a.jpg"));
        assert!(std::fs::read(&a).is_err(), "nothing kept yet");
        keep_remote(&a, b"bytes");
        assert_eq!(std::fs::read(&a).unwrap(), b"bytes");
        keep_remote(&a, b"newer");
        assert_eq!(std::fs::read(&a).unwrap(), b"newer");
        let left: Vec<_> = std::fs::read_dir(a.parent().unwrap())
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(left.len(), 1, "no half-written files left behind");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn qr_codes_are_square_and_inked() {
        let ink = crate::ui::theme::rgb(0x1d1c1a);
        let px = qr_pixels(
            "https://www.metmuseum.org/art/collection/search/436535",
            ink,
        )
        .unwrap();
        assert_eq!(px.width, px.height);
        assert_eq!(px.width % 8, 0);
        assert_eq!(px.data.len(), (px.width * px.height * 4) as usize);
        // The finder pattern's corner is dark; modules are either ink or clear.
        assert_eq!(&px.data[..4], &[0x1d, 0x1c, 0x1a, 0xff]);
        let pixels = px.data.as_chunks::<4>().0;
        assert!(pixels
            .iter()
            .all(|p| *p == [0x1d, 0x1c, 0x1a, 0xff] || *p == [0, 0, 0, 0]));
        assert!(pixels.iter().any(|p| p[3] == 0));
    }

    #[test]
    fn the_cache_forgets_what_was_drawn_longest_ago() {
        let mut lru = Lru::new(250);
        lru.insert("a".into(), Pixels::new(), 100);
        lru.insert("b".into(), Pixels::new(), 100);
        assert!(lru.get("a").is_some(), "drawing `a` again keeps it fresh");
        lru.insert("c".into(), Pixels::new(), 100);
        assert!(lru.get("b").is_none(), "`b` was the stalest");
        assert!(lru.get("a").is_some() && lru.get("c").is_some());
        assert_eq!(lru.bytes, 200);
        lru.clear();
        assert_eq!((lru.bytes, lru.map.len()), (0, 0));
        // One image larger than the whole budget is still kept while it is the only one.
        let mut small = Lru::new(10);
        small.insert("big".into(), Pixels::new(), 1000);
        assert!(small.get("big").is_some());
    }
}
