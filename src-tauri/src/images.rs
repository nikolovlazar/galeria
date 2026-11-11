//! `galeria://` URI scheme: serves original images, display-sized renditions and
//! thumbnails straight from the library folder (nothing is bundled or copied
//! into the app). Renditions/thumbnails are cached in the app cache dir.
//!
//! Routes (the id is percent-encoded):
//!   galeria://localhost/original/<id>
//!   galeria://localhost/render/<id>?w=<physical px>&h=<physical px>&mode=cover|contain
//!   galeria://localhost/thumb/<id>

use crate::core::Core;
use fast_image_resize as fr;
use image::{DynamicImage, ImageDecoder, ImageEncoder};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};
use tokio::sync::Semaphore;

pub const THUMB_EDGE: u32 = 640;
const MAX_RENDERS_KEPT: usize = 80;

static HEAVY: LazyLock<Semaphore> = LazyLock::new(|| {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    Semaphore::new((n / 3).clamp(1, 3))
});
static THUMBS: LazyLock<Semaphore> = LazyLock::new(|| {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    Semaphore::new((n / 2).clamp(2, 6))
});

#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub modified: u64,
}

impl Source {
    fn cache_key(&self) -> String {
        crate::util::hash_str(&[
            &self.path.to_string_lossy(),
            &self.bytes.to_string(),
            &self.modified.to_string(),
        ])
    }
}

pub fn handle(ctx: UriSchemeContext<'_, Wry>, req: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let res = serve(&app, req).await;
        responder.respond(res);
    });
}

async fn serve(app: &AppHandle, req: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = req.uri().path().trim_start_matches('/').to_string();
    let Some((kind, enc_id)) = path.split_once('/') else {
        return error(StatusCode::BAD_REQUEST, "bad path");
    };
    let id = percent_encoding::percent_decode_str(enc_id).decode_utf8_lossy().to_string();
    let query = parse_query(req.uri().query().unwrap_or(""));

    let core = app.state::<Core>();
    let Some(src) = core.source_for(&id) else {
        return error(StatusCode::NOT_FOUND, "unknown painting");
    };
    let cache_dir = core.paths.cache.clone();

    let result: Result<(Vec<u8>, &'static str), String> = match kind {
        "original" => {
            let p = src.path.clone();
            if needs_transcode(&p) {
                // Formats webviews can't display everywhere (TIFF) become a full-size JPEG.
                let _permit = HEAVY.acquire().await;
                let s = src.clone();
                blocking(move || render_cached(&s, s.width, s.height, &cache_dir)).await
            } else {
                blocking(move || std::fs::read(&p).map(|b| (b, mime_for(&p))).map_err(|e| e.to_string())).await
            }
        }
        "thumb" => {
            let _permit = THUMBS.acquire().await;
            let s = src.clone();
            blocking(move || thumb_cached(&s, &cache_dir).and_then(|p| read_jpeg(&p))).await
        }
        "render" => {
            let vw = query.iter().find(|(k, _)| k == "w").and_then(|(_, v)| v.parse::<f64>().ok()).unwrap_or(0.0);
            let vh = query.iter().find(|(k, _)| k == "h").and_then(|(_, v)| v.parse::<f64>().ok()).unwrap_or(0.0);
            let contain = query.iter().any(|(k, v)| k == "mode" && v == "contain");
            let (tw, th) = target_size(src.width, src.height, vw, vh, contain);
            let p = src.path.clone();
            if !needs_transcode(&p) && (tw as f64) >= src.width as f64 * 0.85 {
                blocking(move || std::fs::read(&p).map(|b| (b, mime_for(&p))).map_err(|e| e.to_string())).await
            } else {
                let _permit = HEAVY.acquire().await;
                let s = src.clone();
                blocking(move || render_cached(&s, tw, th, &cache_dir)).await
            }
        }
        _ => Err("unknown route".into()),
    };

    match result {
        Ok((bytes, mime)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", mime)
            .header("Access-Control-Allow-Origin", "*")
            .header("Cache-Control", "max-age=31536000, immutable")
            .body(bytes)
            .unwrap(),
        Err(e) => {
            log::warn!("galeria:// {kind} {id}: {e}");
            error(StatusCode::INTERNAL_SERVER_ERROR, &e)
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

fn error(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .header("Access-Control-Allow-Origin", "*")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), percent_encoding::percent_decode_str(v).decode_utf8_lossy().to_string()))
        .collect()
}

fn mime_for(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("avif") => "image/avif",
        Some("heic") => "image/heic",
        Some("bmp") => "image/bmp",
        Some("tif") | Some("tiff") => "image/tiff",
        _ => "application/octet-stream",
    }
}

fn needs_transcode(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("tif") | Some("tiff") | Some("bmp")
    )
}

/// Size a rendition so it exactly covers (or fits in) a `vw`×`vh` physical-pixel
/// screen. Never upscales.
pub fn target_size(iw: u32, ih: u32, vw: f64, vh: f64, contain: bool) -> (u32, u32) {
    if vw <= 0.0 || vh <= 0.0 || iw == 0 || ih == 0 {
        return (iw, ih);
    }
    let sx = vw / iw as f64;
    let sy = vh / ih as f64;
    let s = if contain { sx.min(sy) } else { sx.max(sy) }.min(1.0);
    (((iw as f64) * s).round().max(1.0) as u32, ((ih as f64) * s).round().max(1.0) as u32)
}

fn read_jpeg(p: &Path) -> Result<(Vec<u8>, &'static str), String> {
    std::fs::read(p).map(|b| (b, "image/jpeg")).map_err(|e| e.to_string())
}

fn decode(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    reader.no_limits();
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let icc = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok((img, icc))
}

fn resize_rgb(img: DynamicImage, tw: u32, th: u32, alg: fr::ResizeAlg) -> Result<Vec<u8>, String> {
    let rgb = img.into_rgb8();
    let (w, h) = rgb.dimensions();
    if (w, h) == (tw, th) {
        return Ok(rgb.into_raw());
    }
    let src = fr::images::Image::from_vec_u8(w, h, rgb.into_raw(), fr::PixelType::U8x3).map_err(|e| e.to_string())?;
    let mut dst = fr::images::Image::new(tw, th, fr::PixelType::U8x3);
    let mut resizer = fr::Resizer::new();
    resizer
        .resize(&src, &mut dst, &fr::ResizeOptions::new().resize_alg(alg))
        .map_err(|e| e.to_string())?;
    Ok(dst.into_vec())
}

fn encode_jpeg(rgb: &[u8], w: u32, h: u32, icc: Option<Vec<u8>>, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity((w * h / 4) as usize);
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    if let Some(icc) = icc {
        let _ = enc.set_icc_profile(icc);
    }
    enc.write_image(rgb, w, h, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())?;
    Ok(out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("tmp{}", fastrand::u32(..)));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn render_cached(src: &Source, tw: u32, th: u32, cache: &Path) -> Result<(Vec<u8>, &'static str), String> {
    let out = cache.join("renders").join(format!("{}_{}x{}.jpg", src.cache_key(), tw, th));
    if let Ok(b) = std::fs::read(&out) {
        return Ok((b, "image/jpeg"));
    }
    let started = std::time::Instant::now();
    let (img, icc) = decode(&src.path)?;
    let rgb = resize_rgb(img, tw, th, fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3))?;
    let bytes = encode_jpeg(&rgb, tw, th, icc, 92)?;
    write_atomic(&out, &bytes)?;
    log::info!("rendered {} at {tw}x{th} in {:?}", src.path.display(), started.elapsed());
    prune(&cache.join("renders"), MAX_RENDERS_KEPT);
    Ok((bytes, "image/jpeg"))
}

pub fn thumb_cached(src: &Source, cache: &Path) -> Result<PathBuf, String> {
    let out = cache.join("thumbs").join(format!("{}.jpg", src.cache_key()));
    if out.exists() {
        return Ok(out);
    }
    let (img, icc) = decode(&src.path)?;
    let (w, h) = (img.width(), img.height());
    let s = (THUMB_EDGE as f64 / w.max(h) as f64).min(1.0);
    let (tw, th) = (((w as f64) * s).round().max(1.0) as u32, ((h as f64) * s).round().max(1.0) as u32);
    let rgb = resize_rgb(img, tw, th, fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom))?;
    let bytes = encode_jpeg(&rgb, tw, th, icc, 85)?;
    write_atomic(&out, &bytes)?;
    Ok(out)
}

/// Average colour of a painting (computed from its thumbnail, in linear light).
pub fn average_color(src: &Source, cache: &Path) -> Result<String, String> {
    let thumb = thumb_cached(src, cache)?;
    let img = image::open(&thumb).map_err(|e| e.to_string())?.into_rgb8();
    let to_lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let to_srgb = |l: f64| {
        let c = if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
        (c.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let (mut r, mut g, mut b, mut n) = (0.0, 0.0, 0.0, 0.0);
    for p in img.pixels() {
        r += to_lin(p[0]);
        g += to_lin(p[1]);
        b += to_lin(p[2]);
        n += 1.0;
    }
    if n == 0.0 {
        return Ok("#202020".into());
    }
    Ok(format!("#{:02x}{:02x}{:02x}", to_srgb(r / n), to_srgb(g / n), to_srgb(b / n)))
}

fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, p) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_sizes() {
        // Wide painting on a 3:2 5K display: cover keeps full height.
        assert_eq!(target_size(9339, 3464, 5120.0, 3414.0, false), (9204, 3414));
        // Contain fits the width.
        assert_eq!(target_size(9339, 3464, 5120.0, 3414.0, true), (5120, 1899));
        // Never upscale.
        assert_eq!(target_size(1000, 500, 5120.0, 3414.0, false), (1000, 500));
    }

    /// `GALERIA_BENCH=/path/to/big.jpg cargo test --release bench_pipeline -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_pipeline() {
        let Some(path) = std::env::var_os("GALERIA_BENCH").map(PathBuf::from) else { return };
        let t = std::time::Instant::now();
        let (img, icc) = decode(&path).unwrap();
        let (w, h) = (img.width(), img.height());
        println!("decode {w}x{h}: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let (tw, th) = target_size(w, h, 5120.0, 3414.0, true);
        let rgb = resize_rgb(img, tw, th, fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3)).unwrap();
        println!("resize → {tw}x{th}: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let out = encode_jpeg(&rgb, tw, th, icc, 92).unwrap();
        println!("encode {} KB: {:?}", out.len() / 1024, t.elapsed());
    }

    #[test]
    fn renders_and_thumbnails() {
        let dir = std::env::temp_dir().join(format!("galeria-img-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.jpg");
        image::RgbImage::from_fn(1600, 800, |x, _| image::Rgb([(x % 255) as u8, 80, 160])).save(&path).unwrap();
        let src = Source { path: path.clone(), width: 1600, height: 800, bytes: 1, modified: 1 };
        let (bytes, mime) = render_cached(&src, 800, 400, &dir).unwrap();
        assert_eq!(mime, "image/jpeg");
        let out = image::load_from_memory(&bytes).unwrap();
        assert_eq!((out.width(), out.height()), (800, 400));
        let thumb = thumb_cached(&src, &dir).unwrap();
        let t = image::open(thumb).unwrap();
        assert_eq!((t.width(), t.height()), (640, 320));
        let avg = average_color(&src, &dir).unwrap();
        assert!(avg.starts_with('#') && avg.len() == 7);
        std::fs::remove_dir_all(&dir).ok();
    }
}
