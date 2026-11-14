//! Galeria's windows, drawn with Rune: the desktop wallpaper, the full-screen
//! viewer and the settings window. Views read the core's [`Snapshot`]; handlers
//! hand work to the core on a worker thread and the next snapshot redraws.

pub mod art;
pub mod fx;
pub mod icons;
pub mod installer;
pub mod plate;
pub mod settings;
pub mod stage;
pub mod theme;
pub mod viewer;
pub mod wallpaper;

use crate::core;
use crate::host::{self, AppHandle};
use art::ArtStore;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// A named starting state for a window, for `galeria ui --scene <name>`.
pub type Scene<S> = (&'static str, fn(&mut S));

/// What every window's state carries: the core, and the images they draw.
#[derive(Clone)]
pub struct Ctx {
    pub app: AppHandle,
    pub art: Arc<ArtStore>,
    /// The system appearance the windows last reported (0 unknown, 1 light, 2 dark).
    dark: Arc<AtomicU8>,
    /// Rendering without a desktop (`galeria ui`): nothing is reported back to the core.
    pub headless: bool,
}

impl Ctx {
    pub fn new(app: AppHandle) -> Ctx {
        Ctx {
            art: Arc::new(ArtStore::new(app.clone())),
            app,
            dark: Arc::new(AtomicU8::new(0)),
            headless: false,
        }
    }

    /// For `galeria ui`: the same views, with no desktop behind them.
    pub fn headless(app: AppHandle) -> Ctx {
        Ctx {
            art: Arc::new(ArtStore::new(app.clone()).immediate()),
            dark: Arc::new(AtomicU8::new(0)),
            headless: true,
            app,
        }
    }

    /// Do core work (file I/O, network, waiting on the UI thread) off the UI thread.
    pub fn run(&self, f: impl FnOnce(&AppHandle) + Send + 'static) {
        let app = self.app.clone();
        host::spawn_blocking(move || f(&app));
    }

    /// Change settings. The windows redraw from the snapshot that follows.
    pub fn patch(&self, patch: serde_json::Value) {
        self.run(move |app| {
            if let Err(e) = core::patch_settings(app, &patch) {
                log::warn!("settings: {e}");
            }
        });
    }

    /// The windows learn of a Light/Dark switch the moment it happens (the core
    /// also asks the system every few seconds). Cheap enough to call from a view.
    pub fn report_dark(&self, dark: bool) {
        if self.headless {
            return;
        }
        let v = if dark { 2 } else { 1 };
        if self.dark.swap(v, Ordering::Relaxed) != v {
            self.run(move |app| {
                // Where the system can be asked directly, its answer wins.
                let dark = crate::platform::system_dark().unwrap_or(dark);
                core::set_dark(app, dark);
            });
        }
    }
}
