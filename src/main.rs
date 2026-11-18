// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod autostart;
mod cli;
mod core;
mod displays;
mod download;
mod enrich;
mod genres;
mod host;
mod images;
mod instance;
mod library;
mod licensing;
mod manifest;
mod menu;
mod open;
mod platform;
mod settings;
mod shell;
mod sources;
mod styles;
mod tone;
mod ui;
mod update;
mod util;

/// The bundle identifier: names the settings, data and cache folders.
pub const APP_ID: &str = "com.nikolovlazar.galeria";

const USAGE: &str = "Galeria: museum-style painting wallpapers.

usage: galeria [flags]      start Galeria, or control the running one
       galeria ui [...]     render its windows without opening them (development)
       galeria update --check|--apply <zip>  check or apply an update (development)

  --next, --prev            change the painting on every display
  --show <id>               show one painting now, by its manifest id
  --viewer                  open the full-screen viewer
  --settings [section]      general, now, browse, discover, sources, rotation, wallpaper, plate, library
  --toggle-plate            show or hide the info plate
  --toggle-panning          pause or resume panning
  --arrange, --done         enter or leave plate-arranging mode
  --menu <id>               trigger a menu item, e.g. style:glass, fit:pan, interval:60
  --theme light|dark|auto   pretend the system is in that appearance
  --check-update            check for updates
  --install-update          install the available update and restart
  --debug                   log what is showing and why
  --quit

GALERIA_HOME=<folder> keeps settings, state and caches there instead of the system folders.
GALERIA_LIBRARY=<folder> sets the art folder on first launch.";

fn main() {
    host::mark_main_thread();
    let args: Vec<String> = std::env::args().collect();
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h")
        && args.get(1).map(String::as_str) != Some("ui")
    {
        println!("{USAGE}");
        return;
    }
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("galeria=info,warn"),
    )
    .format_timestamp_millis()
    .init();

    // `galeria ui …` draws the windows headless; it has nothing to do with a running instance.
    if args.get(1).map(String::as_str) == Some("ui") {
        if let Err(e) = shell::tooling(args[2..].to_vec()) {
            eprintln!("galeria ui: {e}");
            std::process::exit(1);
        }
        return;
    }
    // Updating is independent of the running instance and its command socket.
    if args.get(1).map(String::as_str) == Some("update") {
        if let Err(e) = update::cli(&args[2..]) {
            eprintln!("galeria update: {e}");
            std::process::exit(1);
        }
        return;
    }

    let paths = match core::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Galeria could not find where to keep its settings: {e}");
            std::process::exit(1);
        }
    };
    // `galeria --next` and the like control the instance that is already running.
    let listener = match instance::claim(&paths.data, &args) {
        instance::Claim::Forwarded => return,
        instance::Claim::First(listener) => listener,
    };
    log::info!("config: {}", paths.config.display());
    // Start on the GPU while the app launches (macOS; a no-op elsewhere).
    rune::prewarm_gpu();
    let app = host::AppHandle::new(core::Core::new(paths));
    shell::run(app, args, listener);
}
