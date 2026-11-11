//! How bright a painting is, so light paintings can follow the system's light
//! appearance and dark paintings its dark appearance.
//!
//! The measure is deliberately the one existing manifests already carry
//! (`brightness`: mean luma 0–255, `contrast`: luma standard deviation,
//! `saturation`: mean HSV saturation 0–1), so a manifest value and a value
//! Galeria computes itself are interchangeable.

use image::RgbImage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measure {
    pub brightness: f64,
    pub contrast: f64,
    pub saturation: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tone {
    Light,
    Dark,
}

/// Manual choice for one painting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToneOverride {
    Light,
    Dark,
    /// Fine in either appearance.
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cached {
    /// File identity (path + size + mtime) the measure belongs to.
    pub key: String,
    #[serde(flatten)]
    pub measure: Measure,
}

/// Measures Galeria computed itself, keyed by painting id.
pub type Store = HashMap<String, Cached>;

pub fn measure(img: &RgbImage) -> Measure {
    let n = (img.width() as f64 * img.height() as f64).max(1.0);
    let (mut sum, mut sum_sq, mut sat) = (0.0f64, 0.0f64, 0.0f64);
    for p in img.pixels() {
        let (r, g, b) = (p[0] as f64, p[1] as f64, p[2] as f64);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        sum += y;
        sum_sq += y * y;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        if max > 0.0 {
            sat += (max - min) / max;
        }
    }
    let mean = sum / n;
    let round1 = |v: f64| (v * 10.0).round() / 10.0;
    Measure {
        brightness: round1(mean),
        contrast: round1((sum_sq / n - mean * mean).max(0.0).sqrt()),
        saturation: ((sat / n) * 1000.0).round() / 1000.0,
    }
}

/// Measure a full-size image by way of a small copy (averaging is what matters).
pub fn measure_dynamic(img: &image::DynamicImage) -> Measure {
    measure(&img.thumbnail(160, 160).into_rgb8())
}

pub fn classify(brightness: f64, threshold: f64) -> Tone {
    if brightness >= threshold {
        Tone::Light
    } else {
        Tone::Dark
    }
}

/// The automatic dividing line: the library's median, so each appearance gets
/// about half of the paintings. Kept inside a sensible band so a library of
/// only night scenes still calls its brightest half "light".
pub fn auto_threshold(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 112.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let mid = values.len() / 2;
    let median = if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    };
    (median * 10.0).round() / 10.0
}

/// Flags existing manifests use (`dark` < 60 brightness, `muted` < 0.12 saturation, …).
pub fn tone_flags(m: &Measure, aspect: f64) -> Vec<&'static str> {
    let mut f = Vec::new();
    if m.brightness < 60.0 {
        f.push("dark");
    }
    if m.saturation < 0.12 {
        f.push("muted");
    }
    if (1.0..1.4).contains(&aspect) {
        f.push("squarish");
    }
    if aspect < 1.0 {
        f.push("portrait");
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn measures() {
        let white = RgbImage::from_pixel(8, 8, Rgb([255, 255, 255]));
        let m = measure(&white);
        assert_eq!((m.brightness, m.contrast, m.saturation), (255.0, 0.0, 0.0));
        let red = RgbImage::from_pixel(8, 8, Rgb([200, 0, 0]));
        let m = measure(&red);
        assert_eq!(m.brightness, 59.8);
        assert_eq!(m.saturation, 1.0);
        let half = RgbImage::from_fn(10, 10, |x, _| {
            if x < 5 {
                Rgb([0, 0, 0])
            } else {
                Rgb([200, 200, 200])
            }
        });
        let m = measure(&half);
        assert_eq!((m.brightness, m.contrast), (100.0, 100.0));
    }

    #[test]
    fn thresholds() {
        assert_eq!(auto_threshold(&mut [40.0, 200.0, 100.0]), 100.0);
        assert_eq!(auto_threshold(&mut [40.0, 100.0, 120.0, 200.0]), 110.0);
        assert_eq!(auto_threshold(&mut []), 112.0);
        assert_eq!(classify(112.5, 112.5), Tone::Light);
        assert_eq!(classify(112.4, 112.5), Tone::Dark);
        assert_eq!(
            tone_flags(
                &Measure {
                    brightness: 40.0,
                    contrast: 1.0,
                    saturation: 0.05
                },
                0.8
            ),
            vec!["dark", "muted", "portrait"]
        );
    }
}
