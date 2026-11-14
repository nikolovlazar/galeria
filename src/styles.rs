//! Custom info-plate styles: small JSON files in `<config>/plate-styles/`.
//!
//! A custom style names a built-in to start from and lists what to change:
//! colours, the surface, the type. `ui::plate::custom` reads the fields; the
//! file may carry `//` comment lines, which plain JSON does not allow.

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const BUILTIN: &[(&str, &str)] = &[
    ("plastic", "Museum Plastic"),
    ("glass", "Liquid Glass"),
    ("smoked", "Smoked Glass"),
    ("brass", "Brass Plaque"),
    ("paper", "Paper Card"),
    ("acrylic", "Black Acrylic"),
    ("minimal", "Minimal"),
];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomStyle {
    pub id: String,
    pub name: String,
    /// The file's contents. `Null` when it could not be read as JSON.
    pub spec: Value,
    /// Why the file could not be used, to show next to its name.
    pub error: Option<String>,
    pub path: PathBuf,
}

/// Drop `//` comment lines so a style file can explain itself.
fn strip_comments(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn load(dir: &Path) -> Vec<CustomStyle> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<CustomStyle> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let id = slug(p.file_stem()?.to_str()?);
            let (spec, error) = match serde_json::from_str::<Value>(&strip_comments(&text)) {
                Ok(v) if v.is_object() => (v, None),
                Ok(_) => (
                    Value::Null,
                    Some("the file must hold one JSON object".to_string()),
                ),
                Err(e) => (Value::Null, Some(format!("not valid JSON: {e}"))),
            };
            let name = spec
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| id.clone());
            Some(CustomStyle {
                id,
                name,
                spec,
                error,
                path: p,
            })
        })
        .collect();
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "custom".into()
    } else {
        s
    }
}

pub fn create(dir: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut id = slug(name);
    if BUILTIN.iter().any(|(b, _)| *b == id) {
        id = format!("{id}-custom");
    }
    let mut path = dir.join(format!("{id}.json"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{id}-{n}.json"));
        n += 1;
    }
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&id)
        .to_string();
    let display_name = if name.trim().is_empty() {
        id.clone()
    } else {
        name.trim().to_string()
    };
    std::fs::write(&path, template(&id, &display_name)).map_err(|e| e.to_string())?;
    Ok(path)
}

fn template(_id: &str, name: &str) -> String {
    let name = serde_json::to_string(name).unwrap_or_else(|_| "\"Custom\"".into());
    format!(
        r##"// Galeria custom info plate.
// Save this file and every wallpaper updates at once.
//
// A style starts from a built-in ("base") and changes what it lists:
//   plastic · glass · smoked · brass · paper · acrylic · minimal
//
// Sizes are in em: 1 is the plate's own text size, so the Size setting
// scales the whole plate. Colours are #rgb, #rrggbb or #rrggbbaa.
//
//   "ink"         text colour
//   "font"        a font installed on this computer, or a list: "Avenir Next, Futura"
//   "background"  one colour, a list of colours for a gradient, or "none"
//   "angle"       direction of the gradient in degrees (180 = top to bottom)
//   "frost"       blur what is behind the plate, in points (use a see-through background)
//   "radius"      corner radius
//   "padding"     one number, or [top and bottom, sides], or [top, right, bottom, left]
//   "border"      {{ "width": 1, "color": "#ffffff33" }} or false
//   "shadow"      {{ "y": 0.6, "blur": 1.8, "color": "#00000059" }} or false
//   "glow"        {{ "blur": 1.4, "color": "#3ef2ff8c" }}
//   "material"    a shader-drawn surface instead of the background:
//                 "gilt" "brass" "acrylic" "glass" "smoked" "paper" or "none"
//   "textShadow"  true, false or a colour (for text on see-through plates)
//   "align"       "left" or "center"
//
// Each line of the label can be set on its own:
//   "title" "date" "originalTitle" "artist" "bio" "meta" "description" "footer"
// with any of: "color" "size" "weight" (100-900) "italic" "uppercase"
//              "tracking" (letter spacing) "opacity" "lineHeight"
{{
  "name": {name},
  "base": "paper",
  "ink": "#1d1b18",
  "background": ["#fdfbf6", "#ece6da"],
  "angle": 160,
  "radius": 0.3,
  "padding": [1.3, 1.5, 1.2, 1.5],
  "shadow": {{ "y": 0.6, "blur": 1.8, "color": "#00000059" }},
  "font": "Iowan Old Style, Palatino, Georgia",
  "title": {{ "size": 1.35, "italic": true }},
  "artist": {{ "weight": 700, "tracking": 0.02, "uppercase": false, "size": 1 }},
  "description": {{ "size": 0.86, "lineHeight": 1.5 }},
  "footer": {{ "size": 0.7, "opacity": 0.7 }}
}}
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_loads_custom_styles() {
        let dir = std::env::temp_dir().join(format!("galeria-styles-{}", fastrand::u64(..)));
        let p = create(&dir, "Neon Night!").unwrap();
        assert!(p.ends_with("neon-night.json"));
        let p2 = create(&dir, "Brass").unwrap();
        assert!(
            p2.ends_with("brass-custom.json"),
            "built-in ids are not shadowed"
        );
        let styles = load(&dir);
        assert_eq!(styles.len(), 2);
        let neon = styles.iter().find(|s| s.id == "neon-night").unwrap();
        assert_eq!(neon.name, "Neon Night!");
        // The documented template is itself a usable style.
        assert_eq!(neon.error, None);
        assert_eq!(neon.spec["base"], "paper");
        assert_eq!(neon.spec["title"]["italic"], true);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broken_files_are_listed_with_what_is_wrong() {
        let dir = std::env::temp_dir().join(format!("galeria-styles-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("half.json"), "{ \"name\": \"Half\", ").unwrap();
        std::fs::write(dir.join("list.json"), "[1, 2]").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a style").unwrap();
        let styles = load(&dir);
        assert_eq!(styles.len(), 2, "only .json files are styles");
        assert!(styles.iter().all(|s| s.error.is_some() && s.spec.is_null()));
        assert_eq!(styles.iter().find(|s| s.id == "half").unwrap().name, "half");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn names_with_quotes_stay_valid_json() {
        let dir = std::env::temp_dir().join(format!("galeria-styles-{}", fastrand::u64(..)));
        create(&dir, "The \"Good\" One").unwrap();
        let styles = load(&dir);
        assert_eq!(styles[0].error, None);
        assert_eq!(styles[0].name, "The \"Good\" One");
        std::fs::remove_dir_all(&dir).ok();
    }
}
