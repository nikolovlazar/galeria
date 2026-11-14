//! Painting images: display-sized renditions and thumbnails, decoded straight
//! from the library folder (nothing is bundled or copied into the app).
//! Renditions and thumbnails are cached as JPEG in the app cache dir; the
//! windows draw the decoded pixels.

use fast_image_resize as fr;
use image::{DynamicImage, ImageDecoder, ImageEncoder};
use std::path::{Path, PathBuf};

pub const THUMB_EDGE: u32 = 640;
const MAX_RENDERS_KEPT: usize = 80;
/// The largest side Rune draws as one texture.
pub const MAX_TEXTURE_EDGE: u32 = 8192;

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

/// Remove cached thumbnails and renditions for a painting that was deleted.
pub fn remove_cached(src: &Source, cache: &Path) {
    let key = src.cache_key();
    let _ = std::fs::remove_file(cache.join("thumbs").join(format!("{key}.jpg")));
    let prefix = format!("{key}_");
    if let Ok(rd) = std::fs::read_dir(cache.join("renders")) {
        for e in rd.flatten() {
            if e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&prefix))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
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
    (
        ((iw as f64) * s).round().max(1.0) as u32,
        ((ih as f64) * s).round().max(1.0) as u32,
    )
}

pub(crate) fn decode(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    decode_reader(reader)
}

fn decode_bytes(bytes: &[u8]) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    decode_reader(reader)
}

fn decode_reader<R: std::io::BufRead + std::io::Seek>(
    mut reader: image::ImageReader<R>,
) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
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

// ---------------------------------------------------------------------------
// Pixels for the windows
// ---------------------------------------------------------------------------

/// Straight-alpha sRGB pixels, ready to draw.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Museum scans often carry a wider colour space (Adobe RGB, ProPhoto, Display P3).
/// The windows draw sRGB, so those are converted; sRGB and untagged images pass through.
fn to_srgb(rgba: &mut [u8], icc: Option<&[u8]>) {
    let Some(icc) = icc else { return };
    // The standard sRGB profiles all name themselves in their description tag.
    if icc.windows(4).any(|w| w == b"sRGB") {
        return;
    }
    let converted = (|| {
        use moxcms::{ColorProfile, Layout, TransformOptions};
        let from = ColorProfile::new_from_slice(icc).ok()?;
        let to = ColorProfile::new_srgb();
        let transform = from
            .create_transform_8bit(Layout::Rgba, &to, Layout::Rgba, TransformOptions::default())
            .ok()?;
        let mut out = vec![0u8; rgba.len()];
        transform.transform(rgba, &mut out).ok()?;
        Some(out)
    })();
    match converted {
        Some(out) => rgba.copy_from_slice(&out),
        None => log::debug!("unreadable colour profile; drawing the image as sRGB"),
    }
}

fn into_rgba(img: DynamicImage, icc: Option<&[u8]>) -> Rgba {
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    let mut data = rgba.into_raw();
    to_srgb(&mut data, icc);
    Rgba {
        width,
        height,
        data,
    }
}

/// Fit `w`×`h` inside the largest texture the windows can draw.
pub fn clamp_to_texture(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_TEXTURE_EDGE || long == 0 {
        return (w, h);
    }
    let s = MAX_TEXTURE_EDGE as f64 / long as f64;
    (
        ((w as f64) * s).floor().max(1.0) as u32,
        ((h as f64) * s).floor().max(1.0) as u32,
    )
}

/// The painting at `tw`×`th` (see [`target_size`]), decoded. Scaled renditions are kept in the cache dir.
pub fn render_pixels(src: &Source, tw: u32, th: u32, cache: &Path) -> Result<Rgba, String> {
    let (tw, th) = clamp_to_texture(tw.max(1), th.max(1));
    let (ow, oh) = clamp_to_texture(src.width, src.height);
    // Close enough to the original (and small enough to draw): skip the rendition.
    if (ow, oh) == (src.width, src.height) && (tw as f64) >= src.width as f64 * 0.85 {
        let (img, icc) = decode(&src.path)?;
        return Ok(into_rgba(img, icc.as_deref()));
    }
    let (bytes, _) = render_cached(src, tw, th, cache)?;
    let (img, icc) = decode_bytes(&bytes)?;
    Ok(into_rgba(img, icc.as_deref()))
}

/// The painting's thumbnail (long edge [`THUMB_EDGE`], or `max_edge` when smaller), decoded.
pub fn thumb_pixels(src: &Source, cache: &Path, max_edge: u32) -> Result<Rgba, String> {
    let path = thumb_cached(src, cache)?;
    let (img, icc) = decode(&path)?;
    Ok(into_rgba(shrink(img, max_edge), icc.as_deref()))
}

/// Decode image bytes fetched from a museum (a search result's thumbnail).
pub fn decode_pixels(bytes: &[u8], max_edge: u32) -> Result<Rgba, String> {
    let (img, icc) = decode_bytes(bytes)?;
    Ok(into_rgba(shrink(img, max_edge), icc.as_deref()))
}

fn shrink(img: DynamicImage, max_edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if max_edge == 0 || w.max(h) <= max_edge {
        return img;
    }
    let s = max_edge as f64 / w.max(h) as f64;
    let (tw, th) = (
        ((w as f64) * s).round().max(1.0) as u32,
        ((h as f64) * s).round().max(1.0) as u32,
    );
    img.resize_exact(tw, th, image::imageops::FilterType::Triangle)
}

pub(crate) fn resize_rgb(
    img: DynamicImage,
    tw: u32,
    th: u32,
    alg: fr::ResizeAlg,
) -> Result<Vec<u8>, String> {
    let rgb = img.into_rgb8();
    let (w, h) = rgb.dimensions();
    if (w, h) == (tw, th) {
        return Ok(rgb.into_raw());
    }
    let src = fr::images::Image::from_vec_u8(w, h, rgb.into_raw(), fr::PixelType::U8x3)
        .map_err(|e| e.to_string())?;
    let mut dst = fr::images::Image::new(tw, th, fr::PixelType::U8x3);
    let mut resizer = fr::Resizer::new();
    resizer
        .resize(&src, &mut dst, &fr::ResizeOptions::new().resize_alg(alg))
        .map_err(|e| e.to_string())?;
    Ok(dst.into_vec())
}

pub(crate) fn encode_jpeg(
    rgb: &[u8],
    w: u32,
    h: u32,
    icc: Option<Vec<u8>>,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity((w * h / 4) as usize);
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    if let Some(icc) = icc {
        let _ = enc.set_icc_profile(icc);
    }
    enc.write_image(rgb, w, h, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
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

pub fn render_cached(
    src: &Source,
    tw: u32,
    th: u32,
    cache: &Path,
) -> Result<(Vec<u8>, &'static str), String> {
    let out = cache
        .join("renders")
        .join(format!("{}_{}x{}.jpg", src.cache_key(), tw, th));
    if let Ok(b) = std::fs::read(&out) {
        return Ok((b, "image/jpeg"));
    }
    let started = std::time::Instant::now();
    let (img, icc) = decode(&src.path)?;
    let rgb = resize_rgb(
        img,
        tw,
        th,
        fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3),
    )?;
    let bytes = encode_jpeg(&rgb, tw, th, icc, 92)?;
    write_atomic(&out, &bytes)?;
    log::info!(
        "rendered {} at {tw}x{th} in {:?}",
        src.path.display(),
        started.elapsed()
    );
    prune(&cache.join("renders"), MAX_RENDERS_KEPT);
    Ok((bytes, "image/jpeg"))
}

pub fn thumb_cached(src: &Source, cache: &Path) -> Result<PathBuf, String> {
    let out = cache
        .join("thumbs")
        .join(format!("{}.jpg", src.cache_key()));
    if out.exists() {
        return Ok(out);
    }
    let (img, icc) = decode(&src.path)?;
    let (w, h) = (img.width(), img.height());
    let s = (THUMB_EDGE as f64 / w.max(h) as f64).min(1.0);
    let (tw, th) = (
        ((w as f64) * s).round().max(1.0) as u32,
        ((h as f64) * s).round().max(1.0) as u32,
    );
    let rgb = resize_rgb(
        img,
        tw,
        th,
        fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom),
    )?;
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
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let to_srgb = |l: f64| {
        let c = if l <= 0.0031308 {
            l * 12.92
        } else {
            1.055 * l.powf(1.0 / 2.4) - 0.055
        };
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
    Ok(format!(
        "#{:02x}{:02x}{:02x}",
        to_srgb(r / n),
        to_srgb(g / n),
        to_srgb(b / n)
    ))
}

fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
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
    fn textures_are_capped() {
        assert_eq!(clamp_to_texture(4000, 3000), (4000, 3000));
        assert_eq!(clamp_to_texture(16384, 4096), (8192, 2048));
        let (w, h) = clamp_to_texture(9339, 3464);
        assert!(w == 8192 && (3037..=3039).contains(&h), "{w}x{h}");
    }

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
        let Some(path) = std::env::var_os("GALERIA_BENCH").map(PathBuf::from) else {
            return;
        };
        let t = std::time::Instant::now();
        let (img, icc) = decode(&path).unwrap();
        let (w, h) = (img.width(), img.height());
        println!("decode {w}x{h}: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let (tw, th) = target_size(w, h, 5120.0, 3414.0, true);
        let rgb = resize_rgb(
            img,
            tw,
            th,
            fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3),
        )
        .unwrap();
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
        image::RgbImage::from_fn(1600, 800, |x, _| image::Rgb([(x % 255) as u8, 80, 160]))
            .save(&path)
            .unwrap();
        let src = Source {
            path: path.clone(),
            width: 1600,
            height: 800,
            bytes: 1,
            modified: 1,
        };
        let (bytes, mime) = render_cached(&src, 800, 400, &dir).unwrap();
        assert_eq!(mime, "image/jpeg");
        let out = image::load_from_memory(&bytes).unwrap();
        assert_eq!((out.width(), out.height()), (800, 400));
        let thumb = thumb_cached(&src, &dir).unwrap();
        let t = image::open(thumb).unwrap();
        assert_eq!((t.width(), t.height()), (640, 320));
        let avg = average_color(&src, &dir).unwrap();
        assert!(avg.starts_with('#') && avg.len() == 7);
        // What the windows draw: a scaled rendition, the thumbnail and a small tile.
        let px = render_pixels(&src, 800, 400, &dir).unwrap();
        assert_eq!(
            (px.width, px.height, px.data.len()),
            (800, 400, 800 * 400 * 4)
        );
        let full = render_pixels(&src, 1600, 800, &dir).unwrap();
        assert_eq!((full.width, full.height), (1600, 800));
        let tile = thumb_pixels(&src, &dir, 320).unwrap();
        assert_eq!((tile.width, tile.height), (320, 160));
        std::fs::remove_dir_all(&dir).ok();
    }
}
