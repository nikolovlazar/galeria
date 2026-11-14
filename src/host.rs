//! The handle the core works through, and the seam between it and the windows.
//!
//! The core (library, rotation, downloads, enrichment) knows nothing about how
//! Galeria is drawn. It holds an [`AppHandle`], reaches its state through
//! [`AppHandle::core`], and tells the [`Shell`] when something the windows or
//! the menu bar show has changed. The Rune shell in `shell/` implements
//! [`Shell`]; tests run the core without one.

use crate::core::{Core, Snapshot};
use crate::displays::DisplayInfo;
use crate::download::Job;
use serde::Serialize;
use std::future::Future;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use tokio::task::JoinHandle;

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("galeria-worker")
        .build()
        .expect("tokio runtime")
});

/// Run a future on Galeria's worker threads.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    RUNTIME.spawn(future)
}

/// Run blocking work (file I/O, decoding) off the UI thread.
pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    RUNTIME.spawn_blocking(f)
}

/// Drive a future to completion on the calling thread (tests and headless renders; never the UI thread of a live window).
pub fn block_on<F: Future>(future: F) -> F::Output {
    RUNTIME.block_on(future)
}

/// Progress of a catalogue download (the National Gallery's open dataset).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProgress {
    pub source: String,
    pub stage: String,
    pub received: u64,
    pub total: u64,
}

/// What the core asks of whatever shows Galeria on screen.
///
/// Every method may be called from any thread and must not block on the core.
pub trait Shell: Send + Sync + 'static {
    /// The state every window draws from changed.
    fn snapshot(&self, snap: Arc<Snapshot>);
    /// Something the menu bar item shows changed.
    fn menu(&self, snap: &Snapshot);
    /// The download queue changed.
    fn downloads(&self, jobs: Vec<Job>);
    fn catalog_progress(&self, progress: CatalogProgress);
    /// Make sure there is exactly one wallpaper window per display (or none when live mode is off).
    fn reconcile_windows(&self);
    /// Arrange mode lifts the wallpaper windows above the desktop icons so the plate can be dragged.
    fn arrange_changed(&self, on: bool);
    /// Queue `f` for the UI thread. False when there is no event loop to run it.
    fn run_on_main(&self, f: Box<dyn FnOnce() + Send>) -> bool;
    fn set_dock_icon(&self, show: bool);
    fn open_settings(&self, section: Option<String>);
    fn open_viewer(&self, painting: Option<String>, display: Option<String>);
    fn close_viewer(&self);
    /// Ask for the art folder with the system's folder picker.
    fn choose_library_folder(&self);
    fn quit(&self);
}

struct Inner {
    core: Core,
    shell: OnceLock<Arc<dyn Shell>>,
    /// Displays to report instead of asking the system (headless runs and tests).
    fixed_displays: Mutex<Option<Vec<DisplayInfo>>>,
}

#[derive(Clone)]
pub struct AppHandle(Arc<Inner>);

impl AppHandle {
    pub fn new(core: Core) -> AppHandle {
        AppHandle(Arc::new(Inner {
            core,
            shell: OnceLock::new(),
            fixed_displays: Mutex::new(None),
        }))
    }

    pub fn core(&self) -> &Core {
        &self.0.core
    }

    /// Connect the windows. Only the first shell is kept.
    pub fn attach(&self, shell: Arc<dyn Shell>) {
        let _ = self.0.shell.set(shell);
    }

    pub fn shell(&self) -> Option<&Arc<dyn Shell>> {
        self.0.shell.get()
    }

    pub fn set_fixed_displays(&self, displays: Vec<DisplayInfo>) {
        *self.0.fixed_displays.lock().unwrap() = Some(displays);
    }

    pub fn fixed_displays(&self) -> Option<Vec<DisplayInfo>> {
        self.0.fixed_displays.lock().unwrap().clone()
    }

    /// Run `f` on the UI thread and wait for its result. Never call this while
    /// holding the core lock.
    pub fn on_main<T: Send + 'static>(&self, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        if is_main_thread() {
            return Some(f());
        }
        let shell = self.shell()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let queued = shell.run_on_main(Box::new(move || {
            let _ = tx.send(f());
        }));
        if !queued {
            return None;
        }
        rx.recv_timeout(std::time::Duration::from_secs(10)).ok()
    }
}

static MAIN_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Call once, first thing in `main`.
pub fn mark_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

pub fn is_main_thread() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2::MainThreadMarker::new().is_some()
    }
    #[cfg(not(target_os = "macos"))]
    {
        MAIN_THREAD.get() == Some(&std::thread::current().id())
    }
}
