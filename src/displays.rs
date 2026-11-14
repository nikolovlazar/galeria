//! Display enumeration shared by every platform.

use crate::host::AppHandle;
use serde::Serialize;

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
    // Headless runs and tests bring their own display; otherwise ask the system,
    // which has to happen on the UI thread.
    let mut out = match app.fixed_displays() {
        Some(d) => d,
        None => app
            .on_main(crate::platform::list_displays)
            .unwrap_or_default(),
    };

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
