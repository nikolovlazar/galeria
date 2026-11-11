//! Custom info-plate styles: plain CSS files in `<config>/plate-styles/`.

use serde::Serialize;
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
    pub css: String,
    pub path: PathBuf,
}

pub fn load(dir: &Path) -> Vec<CustomStyle> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<CustomStyle> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("css"))
        .filter_map(|p| {
            let css = std::fs::read_to_string(&p).ok()?;
            let id = slug(p.file_stem()?.to_str()?);
            let name = css
                .lines()
                .take(12)
                .find_map(|l| {
                    let l = l
                        .trim()
                        .trim_start_matches("/*")
                        .trim_start_matches('*')
                        .trim();
                    l.strip_prefix("name:")
                        .map(|n| n.trim().trim_end_matches("*/").trim().to_string())
                })
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            Some(CustomStyle {
                id,
                name,
                css,
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
    let mut path = dir.join(format!("{id}.css"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{id}-{n}.css"));
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

fn template(id: &str, name: &str) -> String {
    format!(
        r#"/* name: {name} */
/*
  Galeria custom info plate.
  Save this file and every wallpaper updates instantly.

  The plate markup:
    .plate[data-style="{id}"]
      .plate-body
        .plate-heading
          .plate-title  .plate-date
        .plate-original-title   (original-language title, when translated)
        .plate-artist
        .plate-bio              (nationality, life dates)
        .plate-meta
          .plate-medium  .plate-dimensions
        .plate-description
        .plate-footer
          .plate-museum  .plate-credit-line  .plate-accession
          .plate-credit  .plate-license  .plate-text-credit
        .plate-qr

  Useful variables: --plate-scale (size multiplier), --plate-width.
  Everything is in `em`, so font-size on the plate scales the whole thing.
*/

.plate[data-style="{id}"] {{
  background: linear-gradient(160deg, #fdfbf6, #ece6da);
  color: #1d1b18;
  border-radius: 4px;
  padding: 1.3em 1.5em 1.2em;
  box-shadow: 0 0.6em 1.8em rgba(0, 0, 0, 0.35);
  font-family: "Iowan Old Style", "Palatino", Georgia, serif;
}}

.plate[data-style="{id}"] .plate-title {{
  font-size: 1.35em;
  font-style: italic;
}}

.plate[data-style="{id}"] .plate-artist {{
  font-weight: 700;
  letter-spacing: 0.02em;
}}

.plate[data-style="{id}"] .plate-description {{
  font-size: 0.86em;
  line-height: 1.5;
}}

.plate[data-style="{id}"] .plate-footer {{
  font-size: 0.7em;
  opacity: 0.7;
}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_loads_custom_styles() {
        let dir = std::env::temp_dir().join(format!("galeria-styles-{}", fastrand::u64(..)));
        let p = create(&dir, "Neon Night!").unwrap();
        assert!(p.ends_with("neon-night.css"));
        let p2 = create(&dir, "Brass").unwrap();
        assert!(
            p2.ends_with("brass-custom.css"),
            "built-in ids are not shadowed"
        );
        let styles = load(&dir);
        assert_eq!(styles.len(), 2);
        let neon = styles.iter().find(|s| s.id == "neon-night").unwrap();
        assert_eq!(neon.name, "Neon Night!");
        assert!(neon.css.contains(r#"[data-style="neon-night"]"#));
        std::fs::remove_dir_all(&dir).ok();
    }
}
