//! The museum label beside a painting.
//!
//! A [`PlateStyle`] is data: ink, surface, type. The seven built-in styles are
//! written here; a custom style is a small JSON file layered over one of them
//! (`styles.rs`). [`render`] turns a painting and a style into elements. Every
//! length is in `em` of the plate's own text size, so the size setting scales
//! the whole plate.

use super::art::ArtStore;
use super::fx::{self, Material};
use super::theme::{hex_or, rgb, rgba};
use crate::core::PaintingView;
use crate::settings::{PlatePosition, PlateSettings};
use crate::styles::CustomStyle;
use rune::tok::Family;
use rune::{col, el, rich, row, text, Align, Color, El, Shadow, SurfaceFit, TextCase, TextStyle};
use serde_json::Value;
use std::collections::HashSet;
use std::f32::consts::PI;
use std::sync::{LazyLock, Mutex};

/// The plate's text size in points at scale 1.
const BASE: f32 = 14.0;

/// A length that scales with the plate (`em`) plus a part that doesn't (`px`: hairlines).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Len {
    pub em: f32,
    pub px: f32,
}

const fn em(v: f32) -> Len {
    Len { em: v, px: 0.0 }
}

const fn px(v: f32) -> Len {
    Len { em: 0.0, px: v }
}

impl Len {
    fn at(self, base: f32) -> f32 {
        self.em * base + self.px
    }
}

/// One line of the plate: how its text is set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Part {
    /// Text size in em.
    pub size: f32,
    pub weight: f32,
    pub italic: bool,
    /// `None` is the plate's ink.
    pub color: Option<Color>,
    pub opacity: f32,
    /// Letter spacing in em of this part's own size.
    pub tracking: f32,
    pub upper: bool,
    /// Line height as a multiple of the text size.
    pub line: f32,
}

impl Part {
    const fn new(size: f32, weight: f32) -> Part {
        Part {
            size,
            weight,
            italic: false,
            color: None,
            opacity: 1.0,
            tracking: 0.0,
            upper: false,
            line: 1.38,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    None,
    Solid(Color),
    /// CSS angle in degrees (0 up, 90 right, 180 down) and colour stops.
    Linear(f32, Vec<(f32, Color)>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    pub y: Len,
    pub blur: Len,
    pub spread: Len,
    pub color: Color,
    pub inset: bool,
}

const fn shade(y: Len, blur: Len, color: Color) -> Shade {
    Shade {
        y,
        blur,
        spread: px(0.0),
        color,
        inset: false,
    }
}

const fn inset(y: Len, blur: Len, spread: Len, color: Color) -> Shade {
    Shade {
        y,
        blur,
        spread,
        color,
        inset: true,
    }
}

/// What a style draws besides its surface and text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decor {
    None,
    /// Specular sweeps across frosted glass.
    Glass,
    /// Brushed metal and four screws.
    Brass,
    /// A glossy reflection across the top.
    Acrylic,
    /// A rule down the left edge instead of a surface.
    Rule,
}

/// How the text stands off what is behind it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Halo {
    None,
    /// A soft dark shadow under the text, for text straight on a painting.
    Soft {
        blur: f32,
        color: Color,
    },
    /// Engraved: a light edge below each letter.
    Engraved(Color),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlateStyle {
    pub ink: Color,
    /// A CSS-like family list; empty is the interface font.
    pub family: &'static str,
    /// Top, right, bottom, left.
    pub padding: [Len; 4],
    pub radius: Len,
    pub fill: Fill,
    /// Blur what is behind the plate, in points (frosted glass). 0 is off.
    pub frost: f32,
    pub border: Option<(f32, Color)>,
    pub shades: Vec<Shade>,
    pub centered: bool,
    pub halo: Halo,
    pub decor: Decor,
    /// What the plate is made of, where a shader draws it (`fx`). `None` is the plain fill.
    pub material: Option<Material>,
    pub title: Part,
    pub date: Part,
    pub original: Part,
    pub artist: Part,
    pub bio: Part,
    pub meta: Part,
    pub description: Part,
    pub footer: Part,
}

const HELVETICA: &str = "Helvetica Neue, Helvetica, Arial";
const BASKERVILLE: &str = "Baskerville, Libre Baskerville, Baskerville Old Face, Times New Roman";
const IOWAN: &str = "Iowan Old Style, Palatino Linotype, Palatino, Book Antiqua, Georgia";

/// What every style starts from (the shared rules of the label).
fn base(ink: Color) -> PlateStyle {
    PlateStyle {
        ink,
        family: "",
        padding: [em(1.35), em(1.5), em(1.25), em(1.5)],
        radius: em(0.22),
        fill: Fill::None,
        frost: 0.0,
        border: None,
        shades: Vec::new(),
        centered: false,
        halo: Halo::None,
        decor: Decor::None,
        material: None,
        title: Part {
            italic: true,
            line: 1.22,
            ..Part::new(1.3, 400.0)
        },
        date: Part {
            line: 1.22,
            ..Part::new(1.3, 400.0)
        },
        original: Part {
            italic: true,
            opacity: 0.68,
            ..Part::new(0.82, 400.0)
        },
        artist: Part::new(1.0, 600.0),
        bio: Part {
            opacity: 0.74,
            ..Part::new(0.86, 400.0)
        },
        meta: Part::new(0.86, 400.0),
        description: Part {
            line: 1.52,
            ..Part::new(0.86, 400.0)
        },
        footer: Part {
            opacity: 0.72,
            line: 1.45,
            ..Part::new(0.7, 400.0)
        },
    }
}

/// A built-in style by id (`styles::BUILTIN` lists them).
pub fn builtin(id: &str) -> Option<PlateStyle> {
    let black = Color::BLACK;
    let white = Color::WHITE;
    Some(match id {
        // Museum Plastic: the standard matte acrylic label next to every painting.
        "plastic" => {
            let mut s = base(rgb(0x1d1c1a));
            s.family = HELVETICA;
            s.fill = Fill::Linear(180.0, vec![(0.0, rgb(0xfcfbf8)), (1.0, rgb(0xf1efea))]);
            s.border = Some((1.0, black.alpha(0.07)));
            s.shades = vec![
                inset(px(1.0), px(0.0), px(0.0), white.alpha(0.95)),
                inset(px(-1.0), px(0.0), px(0.0), black.alpha(0.07)),
                shade(em(0.12), em(0.25), black.alpha(0.2)),
                shade(em(0.8), em(2.2), black.alpha(0.3)),
            ];
            s.title = Part {
                italic: false,
                weight: 700.0,
                tracking: -0.005,
                ..s.title
            };
            s.date.color = Some(rgb(0x55524c));
            s.artist.weight = 500.0;
            s
        }
        // Liquid Glass: frosted, refractive glass.
        "glass" => {
            let mut s = base(white);
            s.padding = [em(1.4), em(1.6), em(1.3), em(1.6)];
            s.radius = em(1.6);
            s.fill = Fill::Solid(rgba(0x1c1c20, 0.14));
            s.frost = 26.0;
            s.border = Some((1.0, white.alpha(0.2)));
            s.shades = vec![
                inset(px(1.0), px(0.5), px(0.0), white.alpha(0.7)),
                inset(px(-1.0), px(1.0), px(0.0), white.alpha(0.15)),
                shade(em(1.2), em(3.0), black.alpha(0.3)),
            ];
            s.halo = Halo::Soft {
                blur: 1.4,
                color: black.alpha(0.5),
            };
            s.decor = Decor::Glass;
            s.material = Some(Material::Glass {
                frost: 0.5,
                smoke: 0.1,
            });
            s.title = Part {
                italic: false,
                weight: 650.0,
                tracking: -0.01,
                ..s.title
            };
            s.footer.opacity = 0.8;
            s
        }
        // Smoked Glass: darker, higher contrast frosted glass.
        "smoked" => {
            let mut s = base(rgb(0xf6f5f2));
            s.padding = [em(1.35), em(1.55), em(1.25), em(1.55)];
            s.radius = em(1.1);
            s.fill = Fill::Solid(rgba(0x101012, 0.5));
            s.frost = 30.0;
            s.material = Some(Material::Glass {
                frost: 0.85,
                smoke: 0.5,
            });
            s.border = Some((1.0, white.alpha(0.07)));
            s.shades = vec![
                inset(px(1.0), px(0.0), px(0.0), white.alpha(0.14)),
                shade(em(1.0), em(2.6), black.alpha(0.35)),
            ];
            s.title = Part {
                italic: false,
                weight: 600.0,
                ..s.title
            };
            s
        }
        // Brass Plaque: engraved, screwed to the frame.
        "brass" => {
            let mut s = base(rgb(0x33260b));
            s.family = BASKERVILLE;
            s.padding = [em(1.55), em(1.9), em(1.45), em(1.9)];
            s.radius = em(0.18);
            s.centered = true;
            s.fill = Fill::Linear(
                135.0,
                vec![
                    (0.0, rgb(0xe7cd84)),
                    (0.2, rgb(0xc49a43)),
                    (0.41, rgb(0xf2e1a3)),
                    (0.62, rgb(0xb58833)),
                    (0.8, rgb(0xdfc06e)),
                    (1.0, rgb(0xa27627)),
                ],
            );
            s.border = Some((1.0, rgba(0xfff2c4, 0.65)));
            s.shades = vec![
                inset(px(0.0), px(0.0), em(0.32), rgba(0x6e4e12, 0.2)),
                inset(px(2.0), px(3.0), px(0.0), white.alpha(0.5)),
                inset(px(-2.0), px(4.0), px(0.0), rgba(0x3c2800, 0.42)),
                shade(em(0.18), em(0.35), black.alpha(0.38)),
                shade(em(0.9), em(2.3), black.alpha(0.36)),
            ];
            s.halo = Halo::Engraved(rgba(0xfff6d6, 0.6));
            s.decor = Decor::Brass;
            s.material = Some(Material::Brass);
            s.title.size = 1.38;
            s.date.size = 1.38;
            s.artist = Part {
                upper: true,
                size: 0.84,
                tracking: 0.1,
                ..s.artist
            };
            s.footer.opacity = 0.8;
            s
        }
        // Paper Card: a letterpress card, old-museum style.
        "paper" => {
            let mut s = base(rgb(0x2a251d));
            s.family = IOWAN;
            s.padding = [em(1.5), em(1.7), em(1.35), em(1.7)];
            s.radius = px(2.0);
            s.fill = Fill::Linear(175.0, vec![(0.0, rgb(0xf7f2e6)), (1.0, rgb(0xeee6d4))]);
            s.shades = vec![
                shade(px(1.0), px(1.0), black.alpha(0.16)),
                shade(em(0.6), em(1.7), black.alpha(0.27)),
            ];
            s.material = Some(Material::Grain { amount: 0.045 });
            s.title.size = 1.36;
            s.date.size = 1.36;
            s.artist = Part {
                upper: true,
                size: 0.84,
                tracking: 0.09,
                ..s.artist
            };
            s
        }
        // Black Acrylic: glossy, modern gallery.
        "acrylic" => {
            let mut s = base(rgb(0xf3f2ef));
            s.family = HELVETICA;
            s.padding = [em(1.4), em(1.6), em(1.3), em(1.6)];
            s.radius = em(0.3);
            s.fill = Fill::Linear(
                160.0,
                vec![
                    (0.0, rgb(0x2a2a2e)),
                    (0.52, rgb(0x0c0c0e)),
                    (1.0, rgb(0x17171a)),
                ],
            );
            s.border = Some((1.0, white.alpha(0.06)));
            s.shades = vec![
                inset(px(1.0), px(0.0), px(0.0), white.alpha(0.2)),
                shade(em(0.15), em(0.3), black.alpha(0.4)),
                shade(em(1.0), em(2.5), black.alpha(0.5)),
            ];
            s.decor = Decor::Acrylic;
            s.material = Some(Material::Acrylic);
            s.title = Part {
                italic: false,
                weight: 300.0,
                size: 1.3 * 1.08,
                tracking: 0.005,
                ..s.title
            };
            s.artist = Part {
                upper: true,
                size: 0.8,
                tracking: 0.14,
                ..s.artist
            };
            s
        }
        // Minimal: vinyl lettering straight on the wall.
        "minimal" => {
            let mut s = base(white);
            s.padding = [em(0.25), em(0.0), em(0.25), em(1.1)];
            s.radius = px(0.0);
            s.halo = Halo::Soft {
                blur: 2.6,
                color: black.alpha(0.75),
            };
            s.decor = Decor::Rule;
            s.title = Part {
                italic: false,
                weight: 600.0,
                ..s.title
            };
            s.footer.opacity = 0.8;
            s
        }
        _ => return None,
    })
}

/// Family names live as long as the app: Rune's text styles hold `&'static str`.
fn intern(name: &str) -> &'static str {
    static NAMES: LazyLock<Mutex<HashSet<&'static str>>> =
        LazyLock::new(|| Mutex::new(HashSet::new()));
    let mut names = NAMES.lock().unwrap();
    if let Some(n) = names.get(name) {
        return n;
    }
    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
    names.insert(leaked);
    leaked
}

fn num(v: &Value) -> Option<f32> {
    v.as_f64().map(|n| n as f32).filter(|n| n.is_finite())
}

fn color(v: &Value, fallback: Color) -> Color {
    v.as_str().map(|s| hex_or(s, fallback)).unwrap_or(fallback)
}

fn apply_part(part: &mut Part, spec: &Value) {
    let Some(o) = spec.as_object() else { return };
    if let Some(v) = o.get("size").and_then(num) {
        part.size = v.clamp(0.3, 4.0);
    }
    if let Some(v) = o.get("weight").and_then(num) {
        part.weight = v.clamp(100.0, 900.0);
    }
    if let Some(v) = o.get("italic").and_then(Value::as_bool) {
        part.italic = v;
    }
    if let Some(v) = o.get("uppercase").and_then(Value::as_bool) {
        part.upper = v;
    }
    if let Some(v) = o.get("tracking").and_then(num) {
        part.tracking = v.clamp(-0.2, 1.0);
    }
    if let Some(v) = o.get("opacity").and_then(num) {
        part.opacity = v.clamp(0.0, 1.0);
    }
    if let Some(v) = o.get("lineHeight").and_then(num) {
        part.line = v.clamp(0.9, 3.0);
    }
    if let Some(v) = o.get("color") {
        part.color = Some(color(v, Color::WHITE));
    }
}

/// A custom style: a built-in (`"base"`, Museum Plastic unless said otherwise) with the
/// file's fields laid over it. Unknown fields are ignored, so a typo never breaks the plate.
pub fn custom(spec: &Value) -> PlateStyle {
    let base_id = spec
        .get("base")
        .and_then(Value::as_str)
        .unwrap_or("plastic");
    let mut s = builtin(base_id).unwrap_or_else(|| builtin("plastic").expect("built-in style"));
    let Some(o) = spec.as_object() else { return s };
    if let Some(v) = o.get("ink") {
        s.ink = color(v, s.ink);
    }
    if let Some(v) = o.get("font").and_then(Value::as_str) {
        s.family = intern(v.trim());
    }
    match o.get("background") {
        Some(Value::String(c)) if c.eq_ignore_ascii_case("none") => s.fill = Fill::None,
        Some(Value::String(c)) => s.fill = Fill::Solid(hex_or(c, Color::WHITE)),
        Some(Value::Array(stops)) if !stops.is_empty() => {
            let colors: Vec<Color> = stops.iter().map(|c| color(c, Color::WHITE)).collect();
            let angle = o.get("angle").and_then(num).unwrap_or(180.0);
            let last = (colors.len() - 1).max(1) as f32;
            s.fill = Fill::Linear(
                angle,
                colors
                    .into_iter()
                    .take(8)
                    .enumerate()
                    .map(|(i, c)| (i as f32 / last, c))
                    .collect(),
            );
        }
        _ => {}
    }
    if let Some(v) = o.get("frost").and_then(num) {
        s.frost = v.clamp(0.0, 60.0);
    }
    if let Some(v) = o.get("radius").and_then(num) {
        s.radius = em(v.clamp(0.0, 4.0));
    }
    match o.get("padding") {
        Some(Value::Number(_)) => {
            let v = o
                .get("padding")
                .and_then(num)
                .unwrap_or(1.3)
                .clamp(0.0, 6.0);
            s.padding = [em(v); 4];
        }
        Some(Value::Array(p)) => {
            let v: Vec<f32> = p
                .iter()
                .filter_map(num)
                .map(|v| v.clamp(0.0, 6.0))
                .collect();
            s.padding = match v.as_slice() {
                [a] => [em(*a); 4],
                [a, b] => [em(*a), em(*b), em(*a), em(*b)],
                [a, b, c] => [em(*a), em(*b), em(*c), em(*b)],
                [a, b, c, d, ..] => [em(*a), em(*b), em(*c), em(*d)],
                [] => s.padding,
            };
        }
        _ => {}
    }
    match o.get("border") {
        Some(Value::Bool(false)) | Some(Value::Null) => s.border = None,
        Some(Value::String(c)) => s.border = Some((1.0, hex_or(c, Color::WHITE))),
        Some(Value::Object(b)) => {
            let width = b.get("width").and_then(num).unwrap_or(1.0).clamp(0.0, 12.0);
            let c = b
                .get("color")
                .map(|c| color(c, Color::WHITE))
                .unwrap_or(Color::WHITE);
            s.border = (width > 0.0).then_some((width, c));
        }
        _ => {}
    }
    match o.get("shadow") {
        Some(Value::Bool(false)) | Some(Value::Null) => s.shades.retain(|sh| sh.inset),
        Some(Value::Object(sh)) => {
            s.shades.retain(|sh| sh.inset);
            s.shades.push(shade(
                em(sh.get("y").and_then(num).unwrap_or(0.6).clamp(-4.0, 4.0)),
                em(sh.get("blur").and_then(num).unwrap_or(1.8).clamp(0.0, 8.0)),
                sh.get("color")
                    .map(|c| color(c, Color::BLACK.alpha(0.35)))
                    .unwrap_or(Color::BLACK.alpha(0.35)),
            ));
        }
        _ => {}
    }
    if let Some(glow) = o.get("glow").and_then(Value::as_object) {
        // A coloured halo around the plate (neon signs, lit boxes).
        s.shades.push(shade(
            px(0.0),
            em(glow
                .get("blur")
                .and_then(num)
                .unwrap_or(1.4)
                .clamp(0.0, 8.0)),
            glow.get("color")
                .map(|c| color(c, Color::WHITE))
                .unwrap_or(Color::WHITE),
        ));
    }
    match o.get("material").and_then(Value::as_str) {
        Some("none") => s.material = None,
        Some("gilt") => s.material = Some(Material::Gilt { border: 0.0 }),
        Some("brass") => s.material = Some(Material::Brass),
        Some("acrylic") => s.material = Some(Material::Acrylic),
        Some("glass") => {
            s.material = Some(Material::Glass {
                frost: 0.5,
                smoke: 0.1,
            })
        }
        Some("smoked") => {
            s.material = Some(Material::Glass {
                frost: 0.85,
                smoke: 0.5,
            })
        }
        Some("paper") => s.material = Some(Material::Grain { amount: 0.045 }),
        _ => {}
    }
    match o.get("align").and_then(Value::as_str) {
        Some("center") => s.centered = true,
        Some("left") => s.centered = false,
        _ => {}
    }
    match o.get("textShadow") {
        Some(Value::Bool(false)) | Some(Value::Null) => s.halo = Halo::None,
        Some(Value::Bool(true)) => {
            s.halo = Halo::Soft {
                blur: 2.0,
                color: Color::BLACK.alpha(0.6),
            }
        }
        Some(Value::String(c)) => {
            s.halo = Halo::Soft {
                blur: 2.0,
                color: hex_or(c, Color::BLACK.alpha(0.6)),
            }
        }
        _ => {}
    }
    for (name, part) in [
        ("title", &mut s.title),
        ("date", &mut s.date),
        ("originalTitle", &mut s.original),
        ("artist", &mut s.artist),
        ("bio", &mut s.bio),
        ("meta", &mut s.meta),
        ("description", &mut s.description),
        ("footer", &mut s.footer),
    ] {
        if let Some(spec) = o.get(name) {
            apply_part(part, spec);
        }
    }
    s
}

/// The style a setting names: built in, custom, or Museum Plastic when it no longer exists.
pub fn resolve(id: &str, customs: &[CustomStyle]) -> PlateStyle {
    if let Some(s) = builtin(id) {
        return s;
    }
    customs
        .iter()
        .find(|c| c.id == id)
        .map(|c| custom(&c.spec))
        .unwrap_or_else(|| builtin("plastic").expect("built-in style"))
}

// ---------------------------------------------------------------------------
// What the label says
// ---------------------------------------------------------------------------

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A museum's name boiled down for comparing with an image credit:
/// "Rijksmuseum, Amsterdam (CC0)" and "Courtesy of the Rijksmuseum, Amsterdam" are the same.
fn normalize(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut depth = 0usize;
    for c in lower.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("public domain", " ");
    out.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !matches!(*w, "courtesy" | "the" | "of" | "cc0" | "photo" | "image"))
        .flat_map(|w| w.chars())
        .filter(|c| c.is_ascii_lowercase())
        .collect()
}

fn language_name(code: &str) -> &str {
    match code {
        "nl" => "Dutch",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "el" => "Greek",
        "sr" => "Serbian",
        "bg" => "Bulgarian",
        "mk" => "Macedonian",
        "pl" => "Polish",
        "cs" => "Czech",
        "hu" => "Hungarian",
        "sv" => "Swedish",
        "da" => "Danish",
        "no" | "nb" => "Norwegian",
        "fi" => "Finnish",
        "ja" => "Japanese",
        "zh" => "Chinese",
        "ko" => "Korean",
        other => other,
    }
}

/// The lines of a label, decided once so the plate and anything that
/// describes it agree.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Label {
    pub title: String,
    pub date: Option<String>,
    pub original_title: Option<String>,
    pub artist: Option<String>,
    pub bio: Option<String>,
    pub medium: Option<String>,
    pub dimensions: Option<String>,
    pub description: Option<String>,
    /// Museum, then its credit line.
    pub museum: Option<(String, Option<String>)>,
    pub accession: Option<String>,
    pub credit: Option<String>,
    pub license: Option<String>,
    pub text_credit: Option<String>,
    /// What the QR code opens.
    pub qr: Option<String>,
}

impl Label {
    pub fn of(p: &PaintingView, s: &PlateSettings) -> Label {
        let f = &s.fields;
        let some = |on: bool, v: &Option<String>| v.clone().filter(|v| on && !v.is_empty());
        let show_description = f.description
            && p.description.as_ref().is_some_and(|d| !d.is_empty())
            && (s.foreign_descriptions || p.description_lang.as_deref().is_none_or(|l| l == "en"));
        // An attribution license (CC BY, CC BY-SA) is honoured even when the credit
        // and license details are switched off: the author and license always show.
        let owed = p.attribution_required;
        let museum = some(f.museum, &p.museum);
        let credit = some(f.credit || owed, &p.credit).filter(|c| {
            // Don't repeat the museum's own name as its image credit.
            !museum
                .as_ref()
                .is_some_and(|m| normalize(c) == normalize(m))
        });
        let credit_names_license = p.credit.as_ref().is_some_and(|c| {
            let c = c.to_uppercase();
            c.contains("CC BY") || c.contains("CC-BY")
        });
        let license = p
            .license
            .clone()
            .filter(|l| !l.is_empty() && (f.license || (owed && !credit_names_license)));
        let text_credit = p
            .description_source
            .as_ref()
            .filter(|_| show_description)
            .map(|source| {
                let lang = p
                    .description_lang
                    .as_deref()
                    .filter(|l| *l != "en")
                    .map(|l| format!(" ({})", language_name(l)))
                    .unwrap_or_default();
                let license = p
                    .description_license
                    .as_ref()
                    .map(|l| format!(", {l}"))
                    .unwrap_or_default();
                format!("Text: {source}{license}{lang}")
            });
        Label {
            title: p.title.clone(),
            date: some(f.date, &p.date),
            original_title: p.original_title.clone().filter(|t| !t.is_empty()),
            artist: some(f.artist, &p.artist),
            bio: some(f.artist_bio, &p.artist_bio),
            medium: some(f.medium, &p.medium).map(|m| capitalize(&m)),
            dimensions: some(f.dimensions, &p.dimensions),
            description: p.description.clone().filter(|_| show_description),
            museum: museum.map(|m| (m, p.credit_line.clone().filter(|c| !c.is_empty()))),
            accession: some(f.accession, &p.accession),
            credit,
            license,
            text_credit,
            qr: p
                .source_page
                .clone()
                .or_else(|| p.wikipedia_url.clone())
                .filter(|_| f.qr),
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn family(name: &'static str) -> Family {
    if name.is_empty() {
        Family::Sans
    } else {
        Family::Named(name)
    }
}

fn type_of(style: &PlateStyle, part: &Part, base: f32) -> TextStyle {
    let size = part.size * base;
    let mut t = TextStyle::new(size, part.weight)
        .family(family(style.family))
        .lh((size * part.line).round())
        .tracking(part.tracking * size);
    if part.italic {
        t = t.italic();
    }
    t
}

fn ink_of(style: &PlateStyle, part: &Part, wash: Option<Color>) -> Color {
    let c = wash.unwrap_or(part.color.unwrap_or(style.ink));
    c.alpha(c.a * part.opacity)
}

fn line(style: &PlateStyle, part: &Part, base: f32, wash: Option<Color>, content: &str) -> El {
    let mut t = text(content.to_string())
        .font(type_of(style, part, base))
        .color(ink_of(style, part, wash))
        .wrap();
    if part.upper {
        t = t.text_case(TextCase::Uppercase);
    }
    if style.centered {
        t = t.align(Align::Center);
    }
    t.w_full()
}

/// The text column. `wash` paints every line in one colour (the halo under the text).
fn body(
    label: &Label,
    s: &PlateSettings,
    style: &PlateStyle,
    base: f32,
    wash: Option<Color>,
) -> El {
    let mut c = col().grow(1.0).min_w(0.0);

    // Title and date share a line and wrap together: "The Truncated Windmill, 1872".
    let mut heading = label.title.clone();
    let title_end = heading.len();
    if let Some(d) = &label.date {
        heading.push_str(", ");
        heading.push_str(d);
    }
    let date_span = rune::Span::new(title_end, heading.len());
    let date_span = if style.date.weight != style.title.weight {
        date_span.weight(style.date.weight)
    } else {
        date_span
    };
    let mut spans =
        vec![date_span
            .size(style.date.size * base)
            .color(ink_of(style, &style.date, wash))];
    if style.title.italic && !style.date.italic {
        // The span list restyles ranges; the element's own style is the title's.
        spans.insert(0, rune::Span::new(0, title_end).italic());
    }
    let title_type = TextStyle {
        italic: false,
        ..type_of(style, &style.title, base)
    };
    let mut h = rich(heading, spans)
        .font(title_type)
        .color(ink_of(style, &style.title, wash))
        .wrap()
        .w_full();
    if style.centered {
        h = h.align(Align::Center);
    }
    c = c.child(h);

    if let Some(t) = &label.original_title {
        c = c.child(line(style, &style.original, base, wash, t).mt(0.2 * base));
    }
    if let Some(a) = &label.artist {
        c = c.child(line(style, &style.artist, base, wash, a).mt(0.6 * base));
    }
    if let Some(b) = &label.bio {
        c = c.child(line(style, &style.bio, base, wash, b));
    }
    if label.medium.is_some() || label.dimensions.is_some() {
        let mut meta = col().w_full().mt(0.5 * base);
        for v in [&label.medium, &label.dimensions].into_iter().flatten() {
            meta = meta.child(line(style, &style.meta, base, wash, v));
        }
        c = c.child(meta);
    }
    if let Some(d) = &label.description {
        // The description reads as prose even on a centred plaque.
        let mut t = text(d.clone())
            .font(type_of(style, &style.description, base))
            .color(ink_of(style, &style.description, wash))
            .wrap()
            .w_full()
            .mt(0.85 * base);
        if s.description_lines > 0 {
            t = t.max_lines(s.description_lines as usize);
        }
        c = c.child(t);
    }

    let mut footer: Vec<String> = Vec::new();
    if let Some((museum, credit_line)) = &label.museum {
        footer.push(match credit_line {
            Some(c) => format!("{museum} · {c}"),
            None => museum.clone(),
        });
    }
    footer.extend(
        [
            &label.accession,
            &label.credit,
            &label.license,
            &label.text_credit,
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    if !footer.is_empty() {
        let mut f = col().w_full().mt(0.95 * base);
        for v in &footer {
            f = f.child(line(style, &style.footer, base, wash, v));
        }
        c = c.child(f);
    }
    c
}

fn css_angle(degrees: f32) -> f32 {
    degrees * PI / 180.0
}

/// A screw head: a bright dome, a dark rim.
fn screw(d: f32) -> El {
    el().size(d, d)
        .radius(d / 2.0)
        .radial_gradient(
            [0.5, 0.5],
            d / 2.0,
            [
                (0.0, rgb(0xfff4c8)),
                (0.32, rgb(0xfff4c8)),
                (0.44, rgb(0x9a7629)),
                (0.86, rgb(0x9a7629)),
                (1.0, rgba(0x321e00, 0.55)),
            ],
        )
        .shadow(0.5, 1.0, Color::BLACK.alpha(0.35))
}

/// What sits between the surface and the text.
fn decorations(style: &PlateStyle, base: f32, radius: f32) -> Vec<El> {
    let white = Color::WHITE;
    let cover = || {
        el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .radius(radius)
            .inert()
            .a11y_hidden()
    };
    match style.decor {
        Decor::None | Decor::Rule => Vec::new(),
        Decor::Glass => vec![
            // Light catching the glass: a diagonal sheen, then a bloom at the top-left corner.
            cover().linear_gradient(
                css_angle(135.0),
                [
                    (0.0, white.alpha(0.26)),
                    (0.42, white.alpha(0.05)),
                    (1.0, white.alpha(0.14)),
                ],
            ),
            cover().clip_rounded(radius).child(
                el().pin(Some(-0.2 * base), Some(-2.0 * base), None, None)
                    .size(22.0 * base, 9.0 * base)
                    .radial_gradient(
                        [0.2, 0.0],
                        [16.0 * base, 6.0 * base],
                        [(0.0, white.alpha(0.2)), (0.55, white.alpha(0.0))],
                    ),
            ),
        ],
        Decor::Acrylic => vec![cover().linear_gradient(
            css_angle(172.0),
            [
                (0.0, white.alpha(0.13)),
                (0.38, white.alpha(0.02)),
                (0.4, white.alpha(0.0)),
                (1.0, white.alpha(0.0)),
            ],
        )],
        Decor::Brass => {
            let (d, m) = (0.75 * base, 0.5 * base);
            let corner = |left: bool, top: bool| {
                screw(d)
                    .pin(
                        left.then_some(m),
                        top.then_some(m),
                        (!left).then_some(m),
                        (!top).then_some(m),
                    )
                    .inert()
                    .a11y_hidden()
            };
            vec![
                // Brushed metal: a faint grain across the plate.
                cover().linear_gradient(
                    css_angle(90.0),
                    [
                        (0.0, white.alpha(0.05)),
                        (0.25, Color::BLACK.alpha(0.03)),
                        (0.5, white.alpha(0.06)),
                        (0.75, Color::BLACK.alpha(0.03)),
                        (1.0, white.alpha(0.05)),
                    ],
                ),
                corner(true, true),
                corner(false, true),
                corner(true, false),
                corner(false, false),
            ]
        }
    }
}

/// The plate for `p`. `zoom` shrinks the whole thing (previews in settings).
pub fn render(
    p: &PaintingView,
    s: &PlateSettings,
    style: &PlateStyle,
    art: &ArtStore,
    zoom: f32,
) -> El {
    let base = (BASE * s.scale as f32 * zoom).max(4.0);
    let width = (s.width * s.scale) as f32 * zoom;
    let label = Label::of(p, s);
    let radius = style.radius.at(base);
    let [top, right, bottom, left] = style.padding.map(|l| l.at(base));

    // Text, with whatever makes it readable on this style's surface underneath.
    let words = body(&label, s, style, base, None);
    let words = match style.halo {
        Halo::None => words,
        Halo::Soft { blur, color } => el()
            .grow(1.0)
            .min_w(0.0)
            .child(
                el().pin(Some(0.0), Some(0.07 * base), Some(0.0), None)
                    .blur(blur * base / BASE)
                    .inert()
                    .a11y_hidden()
                    .child(body(&label, s, style, base, Some(color))),
            )
            .child(words),
        Halo::Engraved(light) => el()
            .grow(1.0)
            .min_w(0.0)
            .child(
                el().pin(Some(0.0), Some(1.0), Some(0.0), None)
                    .inert()
                    .a11y_hidden()
                    .child(body(&label, s, style, base, Some(light))),
            )
            .child(words),
    };

    let mut content = row().w_full().items_end().gap(1.1 * base).child(words);
    if let Some(pixels) = label.qr.as_ref().and_then(|url| art.qr(url, style.ink)) {
        let side = 4.6 * base;
        content = content.child(
            el().size(side, side)
                .opacity(0.85)
                .label("QR code to the museum page")
                .pixels_fit(&pixels, SurfaceFit::Fit),
        );
    }

    let mut plate = el()
        .key("plate")
        .role("note")
        .label(match &label.artist {
            Some(a) => format!("{}, {a}", label.title),
            None => label.title.clone(),
        })
        .w(width)
        .radius(radius);
    // Shaded when the window draws materials; otherwise the style's plain surface.
    let material = style.material.filter(|_| fx::enabled());
    let glass = matches!(material, Some(Material::Glass { .. }));
    if !glass {
        match &style.fill {
            Fill::None => {}
            Fill::Solid(c) => plate = plate.fill(*c),
            Fill::Linear(angle, stops) => {
                plate = plate.linear_gradient(css_angle(*angle), stops.as_slice())
            }
        }
        if style.frost > 0.0 {
            plate = plate.backdrop_blur(style.frost);
        }
    }
    for sh in &style.shades {
        plate = plate.box_shadow(Shadow {
            x: 0.0,
            y: sh.y.at(base),
            blur: sh.blur.at(base),
            spread: sh.spread.at(base),
            color: sh.color,
            inset: sh.inset,
        });
    }
    if let Some((w, c)) = style.border {
        plate = plate.border(w, c);
    }
    if let Some(m) = material {
        plate = plate.child(
            fx::slot(m, base / BASE)
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(radius),
        );
    }
    for d in decorations(style, base, radius) {
        plate = plate.child(d);
    }
    if style.decor == Decor::Rule {
        plate = plate.child(
            el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                .w(2.0)
                .fill(style.ink.alpha(0.75))
                .shadow(1.0, 3.0, Color::BLACK.alpha(0.5))
                .a11y_hidden(),
        );
    }
    plate.child(
        el().w_full()
            .pt(top)
            .pr(right)
            .pb(bottom)
            .pl(left)
            .child(content),
    )
}

/// Lay `plate` out inside `area` (`[x, y, w, h]` in window points) at a
/// normalized position: 0 is the left/top edge of the area, 1 the right/bottom.
pub fn place(plate: El, pos: PlatePosition, area: [f32; 4]) -> El {
    let (x, y) = (pos.x.clamp(0.0, 1.0) as f32, pos.y.clamp(0.0, 1.0) as f32);
    let [ax, ay, aw, ah] = area;
    col()
        .absolute(ax, ay)
        .size(aw.max(0.0), ah.max(0.0))
        .child(el().grow(y))
        .child(
            row()
                .w_full()
                .child(el().grow(x))
                .child(plate)
                .child(el().grow(1.0 - x)),
        )
        .child(el().grow(1.0 - y))
}

const SNAPS: [f64; 3] = [0.0, 0.5, 1.0];

/// Where a plate of `size` whose top-left is at `(left, top)` sits in `area`,
/// as a normalized position that snaps to the edges and centre lines.
pub fn position_at(left: f32, top: f32, size: [f32; 2], area: [f32; 4]) -> PlatePosition {
    let [ax, ay, aw, ah] = area;
    let frac = |v: f32, origin: f32, room: f32| {
        let f = if room > 0.0 {
            ((v - origin) / room) as f64
        } else {
            0.0
        };
        let c = f.clamp(0.0, 1.0);
        SNAPS
            .iter()
            .copied()
            .find(|s| (s - c).abs() < 0.025)
            .unwrap_or(c)
    };
    PlatePosition {
        x: frac(left, ax, aw - size[0]),
        y: frac(top, ay, ah - size[1]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::styles::BUILTIN;

    fn painting() -> PaintingView {
        let mut p = PaintingView::sample();
        p.title = "Wheat Field with Cypresses".into();
        p.artist = Some("Vincent van Gogh".into());
        p.artist_bio = Some("Dutch, 1853–1890".into());
        p.date = Some("1889".into());
        p.medium = Some("oil on canvas".into());
        p.dimensions = Some("73 × 93.4 cm".into());
        p.museum = Some("The Metropolitan Museum of Art".into());
        p.credit_line = Some("Purchase, The Annenberg Foundation Gift, 1993".into());
        p.credit = Some("Image: The Met (CC0)".into());
        p.license = Some("CC0".into());
        p.accession = Some("1993.132".into());
        p.source_page = Some("https://www.metmuseum.org/art/collection/search/436535".into());
        p
    }

    #[test]
    fn every_listed_style_exists() {
        for (id, _) in BUILTIN {
            assert!(builtin(id).is_some(), "{id}");
        }
        assert!(builtin("nope").is_none());
        // A style that was deleted falls back to the default label.
        assert_eq!(resolve("nope", &[]), builtin("plastic").unwrap());
    }

    #[test]
    fn the_label_follows_the_field_switches() {
        let p = painting();
        let mut s = PlateSettings::default();
        let l = Label::of(&p, &s);
        assert_eq!(l.date.as_deref(), Some("1889"));
        assert_eq!(l.medium.as_deref(), Some("Oil on canvas"), "capitalized");
        assert_eq!(
            l.museum,
            Some((
                "The Metropolitan Museum of Art".into(),
                Some("Purchase, The Annenberg Foundation Gift, 1993".into())
            ))
        );
        assert_eq!(l.credit.as_deref(), Some("Image: The Met (CC0)"));
        assert_eq!(l.license, None, "off by default");
        assert_eq!(l.accession, None);
        assert_eq!(l.qr, None);

        s.fields.artist = false;
        s.fields.date = false;
        s.fields.license = true;
        s.fields.accession = true;
        s.fields.qr = true;
        let l = Label::of(&p, &s);
        assert_eq!((l.artist, l.date), (None, None));
        assert_eq!(l.license.as_deref(), Some("CC0"));
        assert_eq!(l.accession.as_deref(), Some("1993.132"));
        assert!(l.qr.unwrap().starts_with("https://www.metmuseum.org"));
    }

    #[test]
    fn a_credit_that_only_repeats_the_museum_is_dropped() {
        let mut p = painting();
        p.museum = Some("Rijksmuseum, Amsterdam".into());
        p.credit = Some("Courtesy of the Rijksmuseum, Amsterdam (CC0)".into());
        let s = PlateSettings::default();
        assert_eq!(Label::of(&p, &s).credit, None);
        p.credit = Some("Photo: Jane Doe".into());
        assert_eq!(Label::of(&p, &s).credit.as_deref(), Some("Photo: Jane Doe"));
    }

    #[test]
    fn attribution_licenses_are_always_credited() {
        let mut p = painting();
        p.attribution_required = true;
        p.credit = Some("Photo: Jane Doe".into());
        p.license = Some("CC BY-SA 4.0".into());
        let mut s = PlateSettings::default();
        s.fields.credit = false;
        s.fields.license = false;
        let l = Label::of(&p, &s);
        assert_eq!(l.credit.as_deref(), Some("Photo: Jane Doe"));
        assert_eq!(l.license.as_deref(), Some("CC BY-SA 4.0"));
        // A credit that already names the license doesn't repeat it.
        p.credit = Some("Photo: Jane Doe, CC BY-SA 4.0".into());
        assert_eq!(Label::of(&p, &s).license, None);
    }

    #[test]
    fn foreign_descriptions_wait_for_their_switch() {
        let mut p = painting();
        p.description = Some("Een korenveld met cipressen.".into());
        p.description_lang = Some("nl".into());
        p.description_source = Some("Rijksmuseum".into());
        let mut s = PlateSettings::default();
        let l = Label::of(&p, &s);
        assert_eq!((l.description, l.text_credit), (None, None));
        s.foreign_descriptions = true;
        let l = Label::of(&p, &s);
        assert!(l.description.is_some());
        assert_eq!(l.text_credit.as_deref(), Some("Text: Rijksmuseum (Dutch)"));
    }

    #[test]
    fn custom_styles_lay_over_a_built_in() {
        let spec = serde_json::json!({
            "base": "smoked",
            "ink": "#e8fbff",
            "background": "#080a18c7",
            "radius": 0.8,
            "padding": [1.3, 1.5],
            "border": { "width": 1, "color": "#3ef2ff" },
            "font": "Avenir Next, Futura",
            "title": { "color": "#ff5ec8", "weight": 700, "italic": false },
            "artist": { "color": "#3ef2ff", "uppercase": true, "tracking": 0.12, "size": 0.85 },
            "someFutureField": 1
        });
        let s = custom(&spec);
        let smoked = builtin("smoked").unwrap();
        assert_eq!(s.frost, smoked.frost, "kept from the base");
        assert_eq!(s.radius, em(0.8));
        assert_eq!(s.padding, [em(1.3), em(1.5), em(1.3), em(1.5)]);
        assert_eq!(s.family, "Avenir Next, Futura");
        assert_eq!(s.title.weight, 700.0);
        assert!(s.artist.upper && (s.artist.tracking - 0.12).abs() < 1e-6);
        assert!(matches!(s.fill, Fill::Solid(c) if (c.a - 0xc7 as f32 / 255.0).abs() < 1e-3));
        assert_eq!(s.border.map(|b| b.0), Some(1.0));
        // Nonsense never breaks the plate.
        assert_eq!(
            custom(&serde_json::json!("oops")),
            builtin("plastic").unwrap()
        );
        assert_eq!(
            custom(&serde_json::json!({ "base": "nope" })),
            builtin("plastic").unwrap()
        );
    }

    #[test]
    fn dragged_positions_snap_to_edges_and_centre() {
        let area = [36.0, 20.0, 1000.0, 600.0];
        let size = [400.0, 200.0];
        // Room to move: 600 across, 400 down.
        assert_eq!(
            position_at(36.0, 20.0, size, area),
            PlatePosition { x: 0.0, y: 0.0 }
        );
        assert_eq!(
            position_at(636.0, 420.0, size, area),
            PlatePosition { x: 1.0, y: 1.0 }
        );
        assert_eq!(
            position_at(340.0, 216.0, size, area),
            PlatePosition { x: 0.5, y: 0.5 },
            "near the centre lines"
        );
        let free = position_at(186.0, 120.0, size, area);
        assert!((free.x - 0.25).abs() < 1e-6 && (free.y - 0.25).abs() < 1e-6);
        // Dragged past the edge: stays inside.
        assert_eq!(
            position_at(-500.0, 9000.0, size, area),
            PlatePosition { x: 0.0, y: 1.0 }
        );
    }
}
