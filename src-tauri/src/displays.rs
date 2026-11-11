//! Display enumeration shared by every platform.

use serde::Serialize;
use tauri::AppHandle;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Insets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayInfo {
    /// Stable key used for per-display settings (plate position, assignments).
    pub key: String,
    pub name: String,
    /// Label of this display's wallpaper window.
    pub label: String,
    /// Logical (point) frame in global top-left-origin coordinates.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub physical_x: i32,
    pub physical_y: i32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub scale: f64,
    pub primary: bool,
    /// Areas covered by the menu bar, Dock or taskbar (logical px).
    pub insets: Insets,
}

impl DisplayInfo {
    pub fn is_portrait(&self) -> bool {
        self.height > self.width * 1.05
    }
}

pub fn list(app: &AppHandle) -> Vec<DisplayInfo> {
    let monitors = app.available_monitors().unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten();
    let mut out = Vec::new();
    for (i, m) in monitors.iter().enumerate() {
        let scale = m.scale_factor().max(0.5);
        let pos = m.position();
        let size = m.size();
        let wa = m.work_area();
        let is_primary = primary
            .as_ref()
            .map(|p| p.position() == pos && p.size() == size)
            .unwrap_or(i == 0);
        let inset = |v: i64| (v.max(0) as f64) / scale;
        let insets = Insets {
            top: inset(wa.position.y as i64 - pos.y as i64),
            left: inset(wa.position.x as i64 - pos.x as i64),
            bottom: inset(
                (pos.y as i64 + size.height as i64)
                    - (wa.position.y as i64 + wa.size.height as i64),
            ),
            right: inset(
                (pos.x as i64 + size.width as i64) - (wa.position.x as i64 + wa.size.width as i64),
            ),
        };
        let name = m
            .name()
            .cloned()
            .unwrap_or_else(|| format!("Display {}", i + 1));
        out.push(DisplayInfo {
            key: name.clone(),
            name,
            label: String::new(),
            x: pos.x as f64 / scale,
            y: pos.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
            physical_x: pos.x,
            physical_y: pos.y,
            physical_width: size.width,
            physical_height: size.height,
            scale,
            primary: is_primary,
            insets,
        });
    }
    crate::platform::decorate_displays(app, &mut out);

    // Keys must be unique even with two identical monitors.
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for d in out.iter_mut() {
        let n = seen.entry(d.key.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            d.key = format!("{} ({})", d.key, n);
            d.name = d.key.clone();
        }
        d.label = format!("wall-{}", &crate::util::hash_str(&[&d.key])[..10]);
    }
    out.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then(a.x.total_cmp(&b.x))
            .then(a.y.total_cmp(&b.y))
    });
    out
}
