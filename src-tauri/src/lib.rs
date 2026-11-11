mod cli;
mod commands;
mod core;
mod displays;
mod enrich;
mod images;
mod library;
mod platform;
mod settings;
mod styles;
mod tray;
mod util;
mod wall;

use tauri::{Manager, RunEvent};
use tauri_plugin_autostart::MacosLauncher;

pub const AUTOSTART_FLAG: &str = "--autostart";

pub fn quit(app: &tauri::AppHandle) {
    let core = app.state::<core::Core>();
    let restore = {
        let mut i = core.lock();
        i.quitting = true;
        i.settings.general.restore_wallpaper_on_quit
    };
    if restore {
        core::restore_original_wallpaper(app);
    }
    app.exit(0);
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("galeria_lib=info,warn")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // `galeria --next` etc. control the running instance; a plain
            // second launch opens the settings.
            if !cli::handle(app, &args) {
                wall::spawn_open_settings(app, None);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![AUTOSTART_FLAG])))
        .register_asynchronous_uri_scheme_protocol("galeria", images::handle)
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::get_library,
            commands::patch_settings,
            commands::next_painting,
            commands::previous_painting,
            commands::show_painting,
            commands::open_viewer,
            commands::open_settings,
            commands::close_viewer,
            commands::set_plate_position,
            commands::set_arrange,
            commands::set_library_path,
            commands::rescan_library,
            commands::set_excluded,
            commands::set_favorite,
            commands::open_url,
            commands::reveal_painting,
            commands::painting_colors,
            commands::report_image_error,
            commands::open_styles_folder,
            commands::create_custom_style,
            commands::open_style_file,
            commands::restore_original_wallpaper,
            commands::refresh_enrichment,
            commands::clear_image_cache,
            commands::frontend_log,
            commands::quit,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let paths = core::Paths::resolve(&handle)?;
            log::info!("config: {}", paths.config.display());
            app.manage(core::Core::new(paths));

            let show_in_dock = app.state::<core::Core>().lock().settings.general.show_in_dock;
            core::apply_dock_policy(&handle, show_in_dock);

            core::start(&handle);

            let args: Vec<String> = std::env::args().collect();
            let autostarted = args.iter().any(|a| a == AUTOSTART_FLAG);
            let needs_setup = app.state::<core::Core>().lock().settings.library_path.is_none();
            if needs_setup && !autostarted {
                wall::spawn_open_settings(&handle, Some("library".into()));
            }
            cli::handle(&handle, &args);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Galeria")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, code, .. } = event {
                // Closing the settings/viewer window must not quit the menu bar app.
                let quitting = app.state::<core::Core>().lock().quitting;
                if code.is_none() && !quitting {
                    api.prevent_exit();
                }
            }
        });
}
