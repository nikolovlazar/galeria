//! macOS: desktop-level NSWindow (the same technique Plash uses) and
//! NSWorkspace for the system wallpaper.

use super::SystemWallpaperOptions;
use crate::displays::{DisplayInfo, Insets};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSColor, NSScreen, NSWindow, NSWindowCollectionBehavior, NSWorkspace,
    NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageFillColorKey, NSWorkspaceDesktopImageScalingKey,
};
use objc2_foundation::{NSDictionary, NSNumber, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, WebviewWindow};

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

fn primary_height(mtm: MainThreadMarker) -> f64 {
    NSScreen::screens(mtm).iter().next().map(|s| s.frame().size.height).unwrap_or(0.0)
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
        let err = (x - d.x).abs() + (y - d.y).abs() + (f.size.width - d.width).abs() + (f.size.height - d.height).abs();
        if best.as_ref().is_none_or(|(e, _)| err < *e) {
            best = Some((err, s.clone()));
        }
    }
    best.filter(|(e, _)| *e < 8.0).map(|(_, s)| s)
}

pub fn decorate_displays(app: &AppHandle, displays: &mut Vec<DisplayInfo>) {
    let input = displays.clone();
    let named = super::on_main(app, move || {
        let Some(mtm) = MainThreadMarker::new() else { return input };
        let mut out = input;
        for d in out.iter_mut() {
            if let Some(s) = find_screen(mtm, d) {
                let name = s.localizedName().to_string();
                if !name.is_empty() {
                    d.name = name.clone();
                    d.key = name;
                }
                let f = s.frame();
                let v = s.visibleFrame();
                d.insets = Insets {
                    top: ((f.origin.y + f.size.height) - (v.origin.y + v.size.height)).max(0.0),
                    bottom: (v.origin.y - f.origin.y).max(0.0),
                    left: (v.origin.x - f.origin.x).max(0.0),
                    right: ((f.origin.x + f.size.width) - (v.origin.x + v.size.width)).max(0.0),
                };
            }
        }
        out
    });
    if let Some(n) = named {
        *displays = n;
    }
}

fn ns_window(window: &WebviewWindow) -> Option<&NSWindow> {
    let ptr = window.ns_window().ok()?;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { &*(ptr as *const NSWindow) })
}

fn frame_for(mtm: MainThreadMarker, d: &DisplayInfo) -> NSRect {
    match find_screen(mtm, d) {
        Some(s) => s.frame(),
        None => {
            let ph = primary_height(mtm);
            NSRect::new(NSPoint::new(d.x, ph - d.y - d.height), NSSize::new(d.width, d.height))
        }
    }
}

pub fn prepare_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(ns) = ns_window(&w) else { return };
        ns.setLevel(level(KEY_DESKTOP));
        ns.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenNone,
        );
        ns.setIgnoresMouseEvents(true);
        ns.setHasShadow(false);
        ns.setCanHide(false);
        // Cover the whole screen, including the area under the menu bar.
        ns.setFrame_display(frame_for(mtm, &d), true);
        ns.orderBack(None);
    });
}

pub fn place_desktop_window(window: &WebviewWindow, d: &DisplayInfo) {
    let w = window.clone();
    let d = d.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if let Some(ns) = ns_window(&w) {
            ns.setFrame_display(frame_for(mtm, &d), true);
        }
    });
}

pub fn set_interactive(window: &WebviewWindow, _d: &DisplayInfo, interactive: bool) {
    let w = window.clone();
    let _ = window.run_on_main_thread(move || {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(ns) = ns_window(&w) else { return };
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

/// Must run on the main thread (see `platform::on_main`).
pub fn set_system_wallpaper(d: &DisplayInfo, path: &Path, opts: SystemWallpaperOptions) -> Result<(), String> {
    let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
    let screen = find_screen(mtm, d).ok_or_else(|| format!("screen for {} not found", d.name))?;
    let url = NSURL::from_file_path(path).ok_or("invalid wallpaper path")?;
    let (r, g, b) = opts.color;
    let scaling = NSNumber::new_usize(NS_IMAGE_SCALE_PROPORTIONALLY_UP_OR_DOWN);
    let clipping = NSNumber::new_bool(opts.fill);
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
    let keys: [&NSString; 3] = unsafe {
        [NSWorkspaceDesktopImageScalingKey, NSWorkspaceDesktopImageAllowClippingKey, NSWorkspaceDesktopImageFillColorKey]
    };
    let objects: [&AnyObject; 3] = [&scaling, &clipping, &color];
    let options = NSDictionary::from_slices(&keys, &objects);
    unsafe { NSWorkspace::sharedWorkspace().setDesktopImageURL_forScreen_options_error(&url, &screen, &options) }
        .map_err(|e| e.localizedDescription().to_string())
}

/// Must run on the main thread.
pub fn current_system_wallpaper(d: &DisplayInfo) -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let screen = find_screen(mtm, d)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    url.path().map(|p| PathBuf::from(p.to_string()))
}

pub fn live_wallpaper_supported() -> bool {
    true
}

fn wallpaper_store() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/com.apple.wallpaper/Store/Index.plist"))
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
    let Some(store) = wallpaper_store() else { return Ok(false) };
    if !backup.exists() {
        return Ok(false);
    }
    std::fs::copy(backup, &store).map_err(|e| e.to_string())?;
    // WallpaperAgent reloads the store when it restarts (launchd restarts it).
    let _ = std::process::Command::new("killall").arg("WallpaperAgent").status();
    Ok(true)
}

/// Render a window's web content to a PNG (WKWebView snapshot — no screen
/// recording permission needed). Used by `galeria --capture` for debugging.
pub fn capture_webview(window: &WebviewWindow, path: PathBuf) -> std::sync::mpsc::Receiver<Result<(), String>> {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_foundation::NSError;
    use std::sync::Mutex;

    let (tx, rx) = std::sync::mpsc::channel();
    let tx = std::sync::Arc::new(Mutex::new(Some(tx)));
    let tx2 = tx.clone();
    let res = window.with_webview(move |wv| {
        let webview = wv.inner() as *mut AnyObject;
        let send = move |r: Result<(), String>| {
            if let Some(t) = tx2.lock().unwrap().take() {
                let _ = t.send(r);
            }
        };
        let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            if image.is_null() {
                let msg = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
                send(Err(msg.unwrap_or_else(|| "snapshot failed".into())));
                return;
            }
            let image = unsafe { &*image };
            let result = (|| {
                let tiff = image.TIFFRepresentation().ok_or("no TIFF data")?;
                let rep = NSBitmapImageRep::imageRepWithData(&tiff).ok_or("no bitmap")?;
                let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }
                    .ok_or("PNG encoding failed")?;
                std::fs::write(&path, png.to_vec()).map_err(|e| e.to_string())
            })();
            send(result);
        });
        let nil: *const AnyObject = std::ptr::null();
        unsafe {
            let _: () = msg_send![webview, takeSnapshotWithConfiguration: nil, completionHandler: &*block];
        }
    });
    if let Err(e) = res {
        if let Some(t) = tx.lock().unwrap().take() {
            let _ = t.send(Err(e.to_string()));
        }
    }
    rx
}
