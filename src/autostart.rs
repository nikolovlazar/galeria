//! Launch at login: a LaunchAgent on macOS, the Run registry key on Windows,
//! an XDG autostart entry on Linux.

/// Passed to Galeria when the system starts it at login, so it stays quiet
/// (no settings window when the art folder is not set up yet).
pub const FLAG: &str = "--autostart";

pub fn set(enabled: bool) -> Result<(), String> {
    // A sandboxed run (`GALERIA_HOME`) or a test must not change what the real install does at login.
    if cfg!(test) || std::env::var_os("GALERIA_HOME").is_some_and(|v| !v.is_empty()) {
        log::info!("launch at login left alone: GALERIA_HOME is set");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let auto = auto_launch::AutoLaunchBuilder::new()
        // The name Galeria has always registered under, so an existing login item is updated, not doubled.
        .set_app_name("Galeria")
        .set_app_path(&exe.to_string_lossy())
        .set_args(&[FLAG])
        .build()
        .map_err(|e| e.to_string())?;
    if enabled {
        auto.enable()
    } else {
        auto.disable()
    }
    .map_err(|e| e.to_string())
}
