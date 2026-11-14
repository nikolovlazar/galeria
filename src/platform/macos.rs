//! macOS: desktop-level NSWindow (the same technique Plash uses) and
//! NSWorkspace for the system wallpaper.

use super::SystemWallpaperOptions;
use crate::displays::{DisplayInfo, Insets};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationPresentationOptions, NSColor, NSScreen, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWorkspace, NSWorkspaceDesktopImageAllowClippingKey,
    NSWorkspaceDesktopImageFillColorKey, NSWorkspaceDesktopImageScalingKey,
};
use objc2_foundation::{NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use rune::raw_window_handle::RawWindowHandle;
use std::path::{Path, PathBuf};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

const KEY_DESKTOP: i32 = 2; // kCGDesktopWindowLevelKey
const KEY_DESKTOP_ICON: i32 = 18; // kCGDesktopIconWindowLevelKey
const NS_IMAGE_SCALE_PROPORTIONALLY_UP_OR_DOWN: usize = 3;

fn level(key: i32) -> isize {
    unsafe { CGWindowLevelForKey(key) as isize }
}

fn desktop_behavior() -> NSWindowCollectionBehavior {
    NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::Stationary
        | NSWindowCollectionBehavior::IgnoresCycle
        | NSWindowCollectionBehavior::FullScreenNone
}

fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm)
        .iter()
        .next()
        .map(|s| s.frame().size.height)
        .unwrap_or(0.0)
}

/// Convert a Cocoa (bottom-left origin) frame to global top-left coordinates.
fn top_left(frame: NSRect, ph: f64) -> (f64, f64) {
    (frame.origin.x, ph - frame.origin.y - frame.size.height)
}

fn find_screen(mtm: MainThreadMarker, d: &DisplayInfo) -> Option<Retained<NSScreen>> {
    let ph = primary_height(mtm);
    let mut best: Option<(f64, Retained<NSScreen>)> = None;
    for s in NSScreen::screens(mtm).iter() {
        let f = s.frame();
        let (x, y) = top_left(f, ph);
        let err = (x - d.x).abs()
            + (y - d.y).abs()
            + (f.size.width - d.width).abs()
            + (f.size.height - d.height).abs();
        if best.as_ref().is_none_or(|(e, _)| err < *e) {
            best = Some((err, s.clone()));
        }
    }
    best.filter(|(e, _)| *e < 8.0).map(|(_, s)| s)
}

/// Every screen, with its friendly name and the area the menu bar and Dock cover.
/// Must run on the main thread.
pub fn list_displays() -> Vec<DisplayInfo> {
    let Some(mtm) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let ph = primary_height(mtm);
    NSScreen::screens(mtm)
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let f = s.frame();
            let v = s.visibleFrame();
            let scale = s.backingScaleFactor().max(0.5);
            let (x, y) = top_left(f, ph);
            let name = s.localizedName().to_string();
            let name = if name.is_empty() {
                format!("Display {}", i + 1)
            } else {
                name
            };
            DisplayInfo {
                key: name.clone(),
                name,
                label: String::new(),
                x,
                y,
                width: f.size.width,
                height: f.size.height,
                physical_x: (x * scale).round() as i32,
                physical_y: (y * scale).round() as i32,
                physical_width: (f.size.width * scale).round() as u32,
                physical_height: (f.size.height * scale).round() as u32,
                scale,
                // The first screen is the one with the menu bar.
                primary: i == 0,
                insets: Insets {
                    top: ((f.origin.y + f.size.height) - (v.origin.y + v.size.height)).max(0.0),
                    bottom: (v.origin.y - f.origin.y).max(0.0),
                    left: (v.origin.x - f.origin.x).max(0.0),
                    right: ((f.origin.x + f.size.width) - (v.origin.x + v.size.width)).max(0.0),
                },
            }
        })
        .collect()
}

/// The display the pointer is over. Must run on the main thread.
pub fn display_under_pointer(displays: &[DisplayInfo]) -> Option<DisplayInfo> {
    let mtm = MainThreadMarker::new()?;
    let p = objc2_app_kit::NSEvent::mouseLocation();
    let (x, y) = (p.x, primary_height(mtm) - p.y);
    displays
        .iter()
        .find(|d| x >= d.x && x < d.x + d.width && y >= d.y && y < d.y + d.height)
        .cloned()
}

/// Run `f` with the window's NSWindow. Main thread only; `None` when the window is gone.
fn with_ns_window<R>(
    window: &rune::WindowHandle,
    f: impl FnOnce(&NSWindow, MainThreadMarker) -> R,
) -> Option<R> {
    let mtm = MainThreadMarker::new()?;
    window
        .with_raw_handle(|handle| {
            let RawWindowHandle::AppKit(h) = handle.as_raw() else {
                return None;
            };
            // SAFETY: winit hands out its live content view, and this runs on the main
            // thread while the handle is borrowed.
            let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
            let ns = view.window()?;
            Some(f(&ns, mtm))
        })
        .ok()
        .flatten()
}

fn frame_for(mtm: MainThreadMarker, d: &DisplayInfo) -> NSRect {
    match find_screen(mtm, d) {
        Some(s) => s.frame(),
        None => {
            let ph = primary_height(mtm);
            NSRect::new(
                NSPoint::new(d.x, ph - d.y - d.height),
                NSSize::new(d.width, d.height),
            )
        }
    }
}

/// How a wallpaper window is created: borderless, behind the desktop icons, on every Space.
pub fn desktop_window_options(d: &DisplayInfo) -> rune::ConfiguredWindowOptions {
    rune::WindowOptions {
        borderless: true,
        click_through: true,
        resizable: false,
        position: Some((d.physical_x, d.physical_y)),
        physical_size: Some((d.physical_width, d.physical_height)),
        min_size: Some((0.0, 0.0)),
        ..Default::default()
    }
    .macos(
        rune::MacWindowOptions::default()
            .shadow(false)
            .level(level(KEY_DESKTOP))
            .collection_behavior(desktop_behavior().0),
    )
}

/// Turn a freshly created window into the desktop of its display. Main thread only.
pub fn prepare_desktop_window(window: &rune::WindowHandle, d: &DisplayInfo) {
    with_ns_window(window, |ns, mtm| {
        ns.setLevel(level(KEY_DESKTOP));
        ns.setCollectionBehavior(desktop_behavior());
        ns.setIgnoresMouseEvents(true);
        ns.setHasShadow(false);
        ns.setCanHide(false);
        // Cover the whole screen, including the area under the menu bar.
        ns.setFrame_display(frame_for(mtm, d), true);
        ns.orderBack(None);
    });
}

/// Re-fit a wallpaper window to its display. Main thread only.
pub fn place_desktop_window(window: &rune::WindowHandle, d: &DisplayInfo) {
    with_ns_window(window, |ns, mtm| {
        let frame = frame_for(mtm, d);
        let now = ns.frame();
        let moved = (now.origin.x - frame.origin.x).abs()
            + (now.origin.y - frame.origin.y).abs()
            + (now.size.width - frame.size.width).abs()
            + (now.size.height - frame.size.height).abs();
        if moved > 0.5 {
            ns.setFrame_display(frame, true);
        }
    });
}

/// Arrange mode: lift the wallpaper above the desktop icons (below every app window) and let
/// it take the mouse; leaving drops it back behind everything. Main thread only.
pub fn set_interactive(window: &rune::WindowHandle, _d: &DisplayInfo, interactive: bool) {
    with_ns_window(window, |ns, mtm| {
        if interactive {
            // `+ 1`: above Finder's desktop icons but below every normal window.
            ns.setLevel(level(KEY_DESKTOP_ICON) + 1);
            ns.setIgnoresMouseEvents(false);
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
            ns.makeKeyAndOrderFront(None);
        } else {
            ns.setLevel(level(KEY_DESKTOP));
            ns.setIgnoresMouseEvents(true);
            ns.orderBack(None);
        }
    });
}

/// How the viewer window is created: a borderless window the size of its display.
pub fn viewer_window_options(d: &DisplayInfo) -> rune::ConfiguredWindowOptions {
    rune::WindowOptions {
        borderless: true,
        resizable: false,
        visible: false,
        position: Some((d.physical_x, d.physical_y)),
        physical_size: Some((d.physical_width, d.physical_height)),
        min_size: Some((0.0, 0.0)),
        ..Default::default()
    }
    .macos(rune::MacWindowOptions::default().shadow(false))
}

/// "Simple" full screen: instant, in the current Space, with the menu bar and Dock tucked
/// away while Galeria is in front. Main thread only.
pub fn enter_viewer(window: &rune::WindowHandle, d: &DisplayInfo) {
    with_ns_window(window, |ns, mtm| {
        let app = NSApplication::sharedApplication(mtm);
        app.setPresentationOptions(
            NSApplicationPresentationOptions::AutoHideDock
                | NSApplicationPresentationOptions::AutoHideMenuBar,
        );
        // Galeria is a menu bar app, and macOS often will not make it the active app when
        // it is asked from a menu or the command line. The presentation options above only
        // hide the menu bar for the active app, so the window also sits above it
        // (NSMainMenuWindowLevel is 24, status items 25) and covers it either way.
        ns.setLevel(26);
        ns.setFrame_display(frame_for(mtm, d), true);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        ns.makeKeyAndOrderFront(None);
    });
}

/// Put the menu bar and Dock back. Main thread only.
pub fn leave_viewer() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setPresentationOptions(NSApplicationPresentationOptions::Default);
    }
}

/// Must run on the main thread (see `AppHandle::on_main`).
pub fn set_system_wallpaper(
    d: &DisplayInfo,
    path: &Path,
    opts: SystemWallpaperOptions,
) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let screen = find_screen(mtm, d).ok_or_else(|| format!("screen for {} not found", d.name))?;
    let url = NSURL::from_file_path(path).ok_or("invalid wallpaper path")?;
    let (r, g, b) = opts.color;
    let scaling = NSNumber::new_usize(NS_IMAGE_SCALE_PROPORTIONALLY_UP_OR_DOWN);
    let clipping = NSNumber::new_bool(opts.fill);
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
    let keys: [&NSString; 3] = unsafe {
        [
            NSWorkspaceDesktopImageScalingKey,
            NSWorkspaceDesktopImageAllowClippingKey,
            NSWorkspaceDesktopImageFillColorKey,
        ]
    };
    let objects: [&AnyObject; 3] = [&scaling, &clipping, &color];
    let options = NSDictionary::from_slices(&keys, &objects);
    unsafe {
        NSWorkspace::sharedWorkspace()
            .setDesktopImageURL_forScreen_options_error(&url, &screen, &options)
    }
    .map_err(|e| e.localizedDescription().to_string())
}

/// Must run on the main thread.
pub fn current_system_wallpaper(d: &DisplayInfo) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let screen = find_screen(mtm, d)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    url.path().map(|p| PathBuf::from(p.to_string()))
}

/// Whether macOS is in Dark Mode (follows "Auto" too): the preference
/// `defaults read -g AppleInterfaceStyle` shows.
pub fn system_dark() -> Option<bool> {
    use objc2_foundation::NSUserDefaults;
    let defaults = NSUserDefaults::standardUserDefaults();
    let style = defaults.stringForKey(&NSString::from_str("AppleInterfaceStyle"));
    Some(style.is_some_and(|s| s.to_string().eq_ignore_ascii_case("dark")))
}

pub fn live_wallpaper_supported() -> bool {
    true
}

fn wallpaper_store() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/com.apple.wallpaper/Store/Index.plist"),
    )
}

/// macOS 14+ keeps every wallpaper choice (images, folders, dynamic, aerials,
/// per-Space settings) in one plist. Copy it before Galeria first changes
/// anything so "Restore" can put back exactly what was there.
pub fn backup_wallpaper_store(dest: &Path) -> bool {
    if dest.exists() {
        return true;
    }
    match wallpaper_store().filter(|p| p.exists()) {
        Some(src) => std::fs::copy(src, dest).is_ok(),
        None => false,
    }
}

pub fn restore_wallpaper_store(backup: &Path) -> Result<bool, String> {
    let Some(store) = wallpaper_store() else {
        return Ok(false);
    };
    if !backup.exists() {
        return Ok(false);
    }
    std::fs::copy(backup, &store).map_err(|e| e.to_string())?;
    // WallpaperAgent reloads the store when it restarts (launchd restarts it).
    let _ = std::process::Command::new("killall")
        .arg("WallpaperAgent")
        .status();
    Ok(true)
}

// ---------------------------------------------------------------------------
// Screen saver
// ---------------------------------------------------------------------------

/// The module macOS calls Photos: a slideshow of a folder's pictures.
const PHOTOS_SAVER: &str = "file:///System/Library/ExtensionKit/Extensions/iLifeSlideshows.appex";

pub fn screen_saver_supported() -> bool {
    true
}

/// Make the screen saver the Photos slideshow, showing `folder`. Touches the same
/// settings System Settings → Screen Saver does.
pub fn set_screen_saver(folder: &Path) -> Result<(), String> {
    let path = folder.to_str().ok_or("the folder's name is not text")?;
    // Which folder the slideshow shows.
    let defaults = |args: &[&str]| -> Result<(), String> {
        let out = std::process::Command::new("defaults")
            .args(["-currentHost", "write", "com.apple.ScreenSaverPhotoChooser"])
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    defaults(&["SelectedSource", "-int", "4"])?;
    defaults(&["SelectedFolderPath", "-string", path])?;
    defaults(&[
        "CustomFolderDict",
        "-dict",
        "identifier",
        path,
        "name",
        "Galeria Screen Saver",
    ])?;
    // Which screen saver runs: the Photos slideshow, in the wallpaper store.
    if let Some(store) = wallpaper_store().filter(|p| p.exists()) {
        let mut value = plist::Value::from_file(&store).map_err(|e| e.to_string())?;
        if choose_photos_saver(&mut value) {
            value.to_file_binary(&store).map_err(|e| e.to_string())?;
            // WallpaperAgent reloads the store when it restarts (launchd restarts it).
            let _ = std::process::Command::new("killall")
                .arg("WallpaperAgent")
                .status();
        }
    }
    Ok(())
}

/// Switch every screen saver choice in the wallpaper store to the Photos slideshow.
/// Returns whether anything changed.
fn choose_photos_saver(value: &mut plist::Value) -> bool {
    use plist::{Dictionary, Value};

    fn module_of(config: &[u8]) -> Option<String> {
        let v = Value::from_reader(std::io::Cursor::new(config)).ok()?;
        let url = v
            .as_dictionary()?
            .get("module")?
            .as_dictionary()?
            .get("relative")?;
        url.as_string().map(str::to_string)
    }

    fn walk(v: &mut Value, changed: &mut bool) {
        match v {
            Value::Dictionary(d) => {
                let is_saver = d
                    .get("Provider")
                    .and_then(Value::as_string)
                    .is_some_and(|p| p == "com.apple.wallpaper.choice.screen-saver");
                if is_saver {
                    let current = d
                        .get("Configuration")
                        .and_then(Value::as_data)
                        .and_then(module_of);
                    if current.as_deref() != Some(PHOTOS_SAVER) {
                        let mut module = Dictionary::new();
                        module.insert("relative".into(), Value::String(PHOTOS_SAVER.into()));
                        let mut config = Dictionary::new();
                        config.insert("module".into(), Value::Dictionary(module));
                        let mut bytes = Vec::new();
                        if Value::Dictionary(config)
                            .to_writer_binary(&mut bytes)
                            .is_ok()
                        {
                            d.insert("Configuration".into(), Value::Data(bytes));
                            *changed = true;
                        }
                    }
                }
                for (_, child) in d.iter_mut() {
                    walk(child, changed);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|c| walk(c, changed)),
            _ => {}
        }
    }

    let mut changed = false;
    walk(value, &mut changed);
    changed
}

#[cfg(test)]
mod saver_tests {
    use super::*;
    use plist::{Dictionary, Value};

    fn choice(module: &str) -> Value {
        let mut m = Dictionary::new();
        m.insert("relative".into(), Value::String(module.into()));
        let mut c = Dictionary::new();
        c.insert("module".into(), Value::Dictionary(m));
        let mut bytes = Vec::new();
        Value::Dictionary(c).to_writer_binary(&mut bytes).unwrap();
        let mut d = Dictionary::new();
        d.insert(
            "Provider".into(),
            Value::String("com.apple.wallpaper.choice.screen-saver".into()),
        );
        d.insert("Configuration".into(), Value::Data(bytes));
        Value::Dictionary(d)
    }

    fn module(v: &Value) -> String {
        let data = v
            .as_dictionary()
            .unwrap()
            .get("Configuration")
            .unwrap()
            .as_data()
            .unwrap();
        let v = Value::from_reader(std::io::Cursor::new(data)).unwrap();
        v.as_dictionary()
            .unwrap()
            .get("module")
            .unwrap()
            .as_dictionary()
            .unwrap()
            .get("relative")
            .unwrap()
            .as_string()
            .unwrap()
            .to_string()
    }

    #[test]
    fn other_savers_become_photos_and_photos_is_left_alone() {
        let aerial = "file:///System/Library/ExtensionKit/Extensions/Aerials.appex";
        let mut store = Value::Array(vec![choice(aerial), choice(PHOTOS_SAVER)]);
        let desktop = {
            let mut d = Dictionary::new();
            d.insert(
                "Provider".into(),
                Value::String("com.apple.wallpaper.choice.image".into()),
            );
            Value::Dictionary(d)
        };
        store.as_array_mut().unwrap().push(desktop.clone());
        assert!(choose_photos_saver(&mut store));
        let items = store.as_array().unwrap();
        assert_eq!(module(&items[0]), PHOTOS_SAVER);
        assert_eq!(module(&items[1]), PHOTOS_SAVER);
        assert_eq!(items[2], desktop, "the desktop wallpaper is not touched");
        assert!(
            !choose_photos_saver(&mut store),
            "already Photos: nothing to do"
        );
    }
}
