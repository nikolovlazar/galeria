//! Galeria on screen: Rune's event loop, the windows and the menu bar item
//! around the core.
//!
//! There are three kinds of window. The settings window is the app's main one:
//! it starts closed and the app keeps running without it. The viewer is a
//! second, borderless window. Each display gets a wallpaper window, opened and
//! closed as displays come and go ([`RuneShell::reconcile`]).

mod tray;

use crate::core::{self, Core, Paths, Snapshot};
use crate::displays::DisplayInfo;
use crate::download::Job;
use crate::host::{self, AppHandle, CatalogProgress, Shell};
use crate::ui::fx;
use crate::ui::settings::{self, SettingsUi};
use crate::ui::viewer::{self, ViewerUi};
use crate::ui::wallpaper::{self, WallUi};
use crate::ui::Ctx;
use crate::{autostart, cli, instance, platform};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// A window's first state is always built by the shell (`App::init`).
fn supplied<S>() -> S {
    unreachable!("the shell supplies this window's state")
}

fn settings_spec() -> rune::AppSpec<SettingsUi> {
    rune::AppSpec {
        min_size: Some((880.0, 580.0)),
        on_key: Some(settings::on_key),
        poll: Some(settings::poll),
        scenes: settings::SCENES,
        ..rune::AppSpec::new("Galeria", (1180.0, 800.0), supplied, settings::view)
    }
}

fn viewer_spec() -> rune::AppSpec<ViewerUi> {
    rune::AppSpec {
        min_size: Some((0.0, 0.0)),
        on_key: Some(viewer::on_key),
        poll: Some(viewer::poll),
        scenes: viewer::SCENES,
        ..rune::AppSpec::new("Galeria Viewer", (1512.0, 982.0), supplied, viewer::view)
    }
}

fn wall_spec() -> rune::AppSpec<WallUi> {
    rune::AppSpec {
        min_size: Some((0.0, 0.0)),
        on_key: Some(wallpaper::on_key),
        poll: Some(wallpaper::poll),
        scenes: wallpaper::SCENES,
        ..rune::AppSpec::new(
            "Galeria Wallpaper",
            (1512.0, 982.0),
            supplied,
            wallpaper::view,
        )
    }
}

/// Give a window the materials compositor (gilt, brass, glass), unless they are switched off.
fn shaded<S: 'static>(app: rune::App<S>) -> rune::App<S> {
    match fx::materials() {
        Some(materials) => app.compositor(materials),
        None => app,
    }
}

/// The settings window draws its own title area: content runs under a
/// transparent title bar, with the traffic lights over the sidebar.
const SETTINGS_CHROME: rune::Chrome = rune::Chrome {
    full_size_content: true,
    traffic_lights: Some((18.0, 20.0)),
    drag_regions: true,
};

struct Wall {
    handle: rune::AppHandle<WallUi>,
    window: rune::WindowHandle,
    display: DisplayInfo,
}

struct Inner {
    app: AppHandle,
    cx: Ctx,
    settings: rune::AppHandle<SettingsUi>,
    viewer: rune::AppHandle<ViewerUi>,
    viewer_window: rune::WindowHandle,
    /// Wallpaper windows by display label.
    walls: Mutex<HashMap<String, Wall>>,
    /// What every new window starts from.
    snap: Mutex<Arc<Snapshot>>,
    /// The socket or port file to remove on the way out.
    endpoint: Mutex<PathBuf>,
}

#[derive(Clone)]
pub struct RuneShell(Arc<Inner>);

impl RuneShell {
    fn main(&self, f: impl FnOnce(&RuneShell) + Send + 'static) {
        let this = self.clone();
        self.run_on_main(Box::new(move || f(&this)));
    }

    /// Open, move or close wallpaper windows so there is exactly one per display. UI thread.
    fn reconcile(&self) {
        let inner = &self.0;
        let (displays, live, arrange) = {
            let i = inner.app.core().lock();
            (i.displays.clone(), core::live(&i), i.arrange)
        };
        let wanted: Vec<DisplayInfo> = if live { displays } else { Vec::new() };
        let mut opened = false;
        let mut walls = inner.walls.lock().unwrap();
        walls.retain(|label, wall| {
            let keep = wanted.iter().any(|d| &d.label == label);
            if !keep {
                wall.window.close();
            }
            keep
        });
        for d in wanted {
            if let Some(wall) = walls.get_mut(&d.label) {
                if wall.display != d {
                    wall.display = d.clone();
                    platform::place_desktop_window(&wall.window, &d);
                }
                continue;
            }
            let snap = inner.snap.lock().unwrap().clone();
            let app = shaded(rune::App::new(wall_spec()));
            let handle = app.handle();
            let state = WallUi::new(inner.cx.clone(), snap, d.key.clone(), Some(handle.clone()));
            let created = d.clone();
            let spec = rune::WindowSpec::from_app(d.label.clone(), app.init(move || state))
                .configured_options(platform::desktop_window_options(&d))
                .on_created(move |window| {
                    log::debug!("wallpaper window created for {}", created.name);
                    platform::prepare_desktop_window(&window, &created);
                    if arrange {
                        platform::set_interactive(&window, &created, true);
                    }
                });
            let window = rune::open_window(spec);
            opened = true;
            log::info!("wallpaper window for {}", d.name);
            walls.insert(
                d.label.clone(),
                Wall {
                    handle,
                    window,
                    display: d,
                },
            );
        }
        if opened {
            // New windows start at the end of an event-loop turn: ask for one now rather
            // than waiting for whatever happens to wake the loop next.
            inner.settings.wake();
        }
    }
}

impl Shell for RuneShell {
    fn snapshot(&self, snap: Arc<Snapshot>) {
        let inner = &self.0;
        *inner.snap.lock().unwrap() = snap.clone();
        let s = snap.clone();
        inner.settings.update(move |ui, tx| ui.apply(s, tx));
        let s = snap.clone();
        inner.viewer.update(move |ui, _| ui.apply(s));
        for wall in inner.walls.lock().unwrap().values() {
            let s = snap.clone();
            wall.handle.update(move |ui, tx| ui.apply(s, tx));
        }
    }

    fn menu(&self, snap: &Snapshot) {
        let snap = snap.clone();
        self.main(move |_| tray::update(&snap));
    }

    fn downloads(&self, jobs: Vec<Job>) {
        self.0.settings.update(move |ui, _| ui.jobs = jobs);
    }

    fn catalog_progress(&self, progress: CatalogProgress) {
        self.0
            .settings
            .update(move |ui, _| ui.progress = Some(progress));
    }

    fn reconcile_windows(&self) {
        self.main(|shell| shell.reconcile());
    }

    fn arrange_changed(&self, on: bool) {
        self.main(move |shell| {
            for wall in shell.0.walls.lock().unwrap().values() {
                platform::set_interactive(&wall.window, &wall.display, on);
            }
        });
    }

    fn run_on_main(&self, f: Box<dyn FnOnce() + Send>) -> bool {
        // An update that changes nothing the settings window draws: it only borrows the UI thread.
        self.0.settings.update_if(move |_, _| {
            f();
            false
        });
        true
    }

    fn set_dock_icon(&self, show: bool) {
        // Keep the Dock and ⌘Tab icon while the settings window is open; only tuck it
        // away when closed and the user hasn't asked to keep it permanently.
        self.0
            .settings
            .set_dock_icon(show || self.0.settings.is_visible());
    }

    fn open_settings(&self, section: Option<String>) {
        self.0
            .settings
            .update(move |ui, tx| ui.navigate(section.as_deref(), tx));
        self.0.settings.set_dock_icon(true);
        self.0.settings.open();
    }

    fn open_viewer(&self, painting: Option<String>, display: Option<String>) {
        let this = self.clone();
        // Listing the library and finding the display both take a moment: off the UI thread.
        host::spawn_blocking(move || {
            let inner = &this.0;
            let core = inner.app.core();
            let displays = core.lock().displays.clone();
            let under_pointer = || {
                let all = displays.clone();
                inner
                    .app
                    .on_main(move || platform::display_under_pointer(&all))
                    .flatten()
            };
            let Some(d) = display
                .and_then(|k| displays.iter().find(|d| d.key == k).cloned())
                .or_else(under_pointer)
                .or_else(|| displays.first().cloned())
            else {
                return log::warn!("viewer: no display");
            };
            let id = {
                let i = core.lock();
                painting
                    .or_else(|| {
                        i.persisted
                            .assignments
                            .get(&d.key)
                            .map(|a| a.painting.clone())
                    })
                    .filter(|id| i.library.get(id).is_some())
                    .or_else(|| i.library.paintings.first().map(|p| p.id.clone()))
            };
            let Some(id) = id else {
                return log::warn!("viewer: the library is empty");
            };
            let library = Arc::new(core.library_view().paintings);
            core.lock().viewer_open = true;
            let window = inner.viewer_window.clone();
            inner.viewer.update(move |ui, tx| {
                ui.window = Some(window.clone());
                ui.open(&id, library, Some(d.clone()), tx);
                // Already on screen (another painting asked for): it only needs to come forward.
                platform::enter_viewer(&window, &d);
            });
            inner.viewer_window.open();
        });
    }

    fn close_viewer(&self) {
        self.0.viewer_window.close();
        self.main(|shell| {
            platform::leave_viewer();
            shell.0.app.core().lock().viewer_open = false;
        });
    }

    fn choose_library_folder(&self) {
        self.0.settings.update(|ui, tx| ui.choose_folder(tx));
        self.0.settings.set_dock_icon(true);
        self.0.settings.open();
    }

    fn quit(&self) {
        instance::release(&self.0.endpoint.lock().unwrap());
        self.main(|shell| {
            tray::remove();
            shell.0.settings.quit();
        });
    }
}

/// Show Galeria in the Dock and ⌘Tab switcher whenever the settings window is open.
fn settings_visibility(ui: &mut SettingsUi, visible: bool, _: &mut rune::Tx) {
    if visible {
        if let Some(handle) = &ui.handle {
            handle.set_dock_icon(true);
        }
    }
}

/// When the settings window closes, return to menu-bar-only mode unless "Show in Dock" is on.
fn settings_closed(ui: &mut SettingsUi, _: &mut rune::Tx) {
    let keep = ui.cx.app.core().lock().settings.general.show_in_dock;
    if !keep {
        if let Some(handle) = &ui.handle {
            handle.set_dock_icon(false);
        }
    }
}

/// The viewer came on screen: cover its display and tuck the menu bar and Dock away.
/// (Going off screen is reported in passing while the window is being shown, so
/// leaving full screen waits for the window to actually close.)
fn viewer_visibility(ui: &mut ViewerUi, visible: bool, _: &mut rune::Tx) {
    if !visible {
        return;
    }
    if let (Some(window), Some(display)) = (&ui.window, &ui.display) {
        platform::enter_viewer(window, display);
        ui.cx.app.core().lock().viewer_open = true;
    }
}

/// The viewer closed: put the menu bar and Dock back, and let the core look at the displays again.
fn viewer_closed(ui: &mut ViewerUi, _: &mut rune::Tx) {
    platform::leave_viewer();
    ui.cx.app.core().lock().viewer_open = false;
}

/// Run Galeria. Returns when it quits.
pub fn run(app: AppHandle, args: Vec<String>, listener: instance::Listener) {
    // Displays first: the viewer window is made for one, and paintings are assigned per display.
    core::refresh_displays(&app, true);
    let cx = Ctx::new(app.clone());
    let snap = Arc::new(app.core().snapshot());
    log::debug!("starting the event loop");
    let (show_in_dock, primary) = {
        let i = app.core().lock();
        (i.settings.general.show_in_dock, i.displays.first().cloned())
    };

    let mut main = shaded(rune::App::new(settings_spec()))
        // Galeria's own flags are not Rune's.
        .args(Vec::new())
        .windowless()
        .keep_running()
        .standard_menus()
        .chrome(SETTINGS_CHROME)
        .on_visibility(settings_visibility)
        .on_close(settings_closed);
    if !show_in_dock {
        main = main.accessory();
    }
    let settings = main.handle();
    let mut main = main.init({
        let (cx, snap, handle) = (cx.clone(), snap.clone(), settings.clone());
        move || SettingsUi::new(cx, snap).with_handle(handle)
    });

    let viewer_app = shaded(rune::App::new(viewer_spec())).on_close(viewer_closed);
    let viewer = viewer_app.handle();
    let viewer_app = viewer_app.init({
        let (cx, snap, handle) = (cx.clone(), snap.clone(), viewer.clone());
        move || ViewerUi::new(cx, snap, Some(handle))
    });
    let viewer_options = match &primary {
        Some(d) => platform::viewer_window_options(d),
        None => rune::WindowOptions {
            borderless: true,
            visible: false,
            ..Default::default()
        }
        .into(),
    };
    let (_, viewer_window) = main.window(
        rune::WindowSpec::from_app("viewer", viewer_app)
            .configured_options(viewer_options)
            .on_visibility(viewer_visibility),
    );

    let shell = RuneShell(Arc::new(Inner {
        app: app.clone(),
        cx,
        settings: settings.clone(),
        viewer,
        viewer_window,
        walls: Mutex::new(HashMap::new()),
        snap: Mutex::new(snap.clone()),
        endpoint: Mutex::new(listener.endpoint().to_path_buf()),
    }));
    app.attach(Arc::new(shell.clone()));

    // Everything below waits for the event loop: the menu bar item belongs to the UI
    // thread, and the core's start reads files and asks the system about its displays.
    let boot = shell.clone();
    settings.update_if(move |_, _| {
        log::debug!("event loop running");
        tray::install(&boot.0.app, &snap);
        let shell = boot.clone();
        host::spawn_blocking(move || {
            let app = &shell.0.app;
            core::start(app);
            let autostarted = args.iter().any(|a| a == autostart::FLAG);
            let needs_setup = app.core().lock().settings.library_path.is_none();
            if needs_setup && !autostarted {
                // The settings window opens on the first run's welcome.
                shell.open_settings(None);
            }
            cli::handle(app, &args);
            let (app, shell) = (app.clone(), shell.clone());
            listener.serve(move |args| {
                // `galeria --next` etc. control this instance; a plain second launch opens the settings.
                if !cli::handle(&app, &args) {
                    shell.open_settings(None);
                }
            });
        });
        false
    });
    main.run();
    instance::release(&shell.0.endpoint.lock().unwrap());
}

// ---------------------------------------------------------------------------
// `galeria ui …`: the windows without a desktop
// ---------------------------------------------------------------------------

/// Where `galeria ui` keeps its state: `GALERIA_HOME` when set, otherwise a
/// private folder seeded with a copy of the real settings. It never writes to
/// the installed app's settings or state.
fn tooling_home() -> Result<PathBuf, String> {
    if let Some(home) = std::env::var_os("GALERIA_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::temp_dir().join("galeria-ui");
    let config = home.join("config");
    std::fs::create_dir_all(&config).map_err(|e| e.to_string())?;
    let copy = config.join("settings.json");
    if !copy.exists() {
        if let Some(real) =
            rune::dirs::AppDirs::new(crate::APP_ID).map(|d| d.config().join("settings.json"))
        {
            let _ = std::fs::copy(real, &copy);
        }
    }
    Ok(home)
}

/// Render the windows headless with Rune's tooling: `galeria ui --window wall
/// --snap out.png`, `--tree`, `--script`, `--list-windows`, `--list-scenes`.
/// The same views, state and images as the app, on one virtual display.
pub fn tooling(args: Vec<String>) -> Result<(), String> {
    let app = AppHandle::new(Core::new(Paths::in_dir(&tooling_home()?)?));
    // Nothing here may reach the real desktop.
    app.core().lock().settings.wallpaper.sync_system = false;
    // The virtual display is the size and density the render asks for.
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let (width, height) = value("--size")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse::<f64>().ok()?, h.parse::<f64>().ok()?))
        })
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .unwrap_or((1512.0, 982.0));
    let scale = value("--scale")
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .unwrap_or(2.0);
    let cx = Ctx::headless(app.clone());
    rune::run_windows_with_args(rune::RunOptions::default(), args, move |_| {
        let display = DisplayInfo {
            key: "Display".into(),
            name: "Display".into(),
            label: String::new(),
            x: 0.0,
            y: 0.0,
            width,
            height,
            physical_x: 0,
            physical_y: 0,
            physical_width: (width * scale).round() as u32,
            physical_height: (height * scale).round() as u32,
            scale,
            primary: true,
            // A menu bar, as on a real desktop.
            insets: crate::displays::Insets {
                top: 25.0,
                ..Default::default()
            },
        };
        let size = (display.width as f32, display.height as f32);
        app.set_fixed_displays(vec![display]);
        core::refresh_displays(&app, true);
        app.core().lock().dark = Some(true);
        core::reload_library(&app);
        // `GALERIA_UI_PAINTING=<id>` picks what hangs on the virtual display.
        if let Ok(id) = std::env::var("GALERIA_UI_PAINTING") {
            if let Err(e) = core::show(&app, &id, None) {
                eprintln!("GALERIA_UI_PAINTING={id}: {e}");
            }
        }
        let snap = Arc::new(app.core().snapshot());
        let key = snap
            .displays
            .first()
            .map(|d| d.info.key.clone())
            .unwrap_or_default();

        let wall = shaded(rune::App::new(rune::AppSpec {
            size,
            ..wall_spec()
        }));
        let wall = rune::WindowSpec::from_app("wall", wall)
            .state(WallUi::new(cx.clone(), snap.clone(), key.clone(), None).immediate());
        let view = shaded(rune::App::new(rune::AppSpec {
            size,
            ..viewer_spec()
        }));
        let view = rune::WindowSpec::from_app("viewer", view).state(
            ViewerUi::new(cx.clone(), snap.clone(), None)
                .immediate()
                .showing_current(),
        );
        let prefs = rune::WindowSpec::from_app("settings", shaded(rune::App::new(settings_spec())))
            .chrome(SETTINGS_CHROME)
            .state(SettingsUi::new(cx, snap).immediate());
        // The picture behind the installer window (packaging/macos/dmg.sh renders it).
        let installer = shaded(rune::App::new(rune::AppSpec {
            min_size: Some((0.0, 0.0)),
            ..rune::AppSpec::new(
                "Galeria Installer",
                crate::ui::installer::CANVAS,
                || crate::ui::installer::InstallerUi,
                crate::ui::installer::view,
            )
        }));
        let installer = rune::WindowSpec::from_app("installer", installer);
        vec![prefs.boxed(), wall.boxed(), view.boxed(), installer.boxed()]
    });
    Ok(())
}
