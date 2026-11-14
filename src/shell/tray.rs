//! The menu bar item, drawn by the system.
//!
//! macOS and Windows get the whole menu (submenus, check marks) through
//! `tray-icon`. Linux uses Rune's own tray, which shows a flat menu: the
//! everyday actions, with everything else in the settings window.

use crate::core::Snapshot;
use crate::host::{self, AppHandle};
use crate::menu::{self, Node};

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod native {
    use super::*;
    use std::cell::RefCell;
    use tray_icon::menu::{
        accelerator::Accelerator, CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem,
        PredefinedMenuItem, Submenu,
    };
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

    struct State {
        tray: TrayIcon,
        pending: Option<Snapshot>,
        guard: guard::MenuGuard,
    }

    thread_local! {
        /// The status item belongs to the UI thread.
        static TRAY: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    /// Replacing the menu while it is open cancels AppKit/Win32 menu tracking
    /// (`muda` calls `cancelTrackingWithoutAnimation` on drop). Hold updates
    /// while any menu is open and apply the newest snapshot once it closes.
    #[cfg(target_os = "macos")]
    mod guard {
        use objc2::rc::Retained;
        use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
        use objc2_app_kit::{
            NSEventTrackingRunLoopMode, NSMenuDidBeginTrackingNotification,
            NSMenuDidEndTrackingNotification,
        };
        use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject, NSRunLoop};
        use std::cell::Cell;
        use std::ffi::c_void;
        use std::ptr;

        thread_local! {
            static OPEN_MENUS: Cell<usize> = const { Cell::new(0) };
        }

        #[repr(C)]
        struct CFRunLoopSourceContext {
            version: isize,
            info: *mut c_void,
            retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
            release: Option<unsafe extern "C" fn(*const c_void)>,
            copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
            equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> u8>,
            hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
            schedule: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
            cancel: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
            perform: Option<unsafe extern "C" fn(*mut c_void)>,
        }

        #[link(name = "CoreFoundation", kind = "framework")]
        extern "C" {
            static kCFRunLoopDefaultMode: *const c_void;
            fn CFRunLoopGetMain() -> *mut c_void;
            fn CFRunLoopSourceCreate(
                allocator: *const c_void,
                order: isize,
                context: *const CFRunLoopSourceContext,
            ) -> *mut c_void;
            fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: *const c_void);
            fn CFRunLoopSourceSignal(source: *mut c_void);
            fn CFRunLoopSourceInvalidate(source: *mut c_void);
            fn CFRunLoopWakeUp(rl: *mut c_void);
            fn CFRelease(cf: *const c_void);
        }

        unsafe extern "C" fn on_default_mode(_: *mut c_void) {
            super::flush();
        }

        define_class!(
            #[unsafe(super = NSObject)]
            #[thread_kind = MainThreadOnly]
            #[name = "GaleriaMenuObserver"]
            struct MenuObserver;

            impl MenuObserver {
                #[unsafe(method(menuDidBeginTracking:))]
                fn did_begin(&self, _: &NSNotification) {
                    OPEN_MENUS.with(|n| n.set(n.get() + 1));
                }

                #[unsafe(method(menuDidEndTracking:))]
                fn did_end(&self, _: &NSNotification) {
                    let left = OPEN_MENUS.with(|n| {
                        let next = n.get().saturating_sub(1);
                        n.set(next);
                        next
                    });
                    if left == 0 {
                        super::signal_flush();
                    }
                }
            }
        );

        pub struct MenuGuard {
            observer: Retained<MenuObserver>,
            source: *mut c_void,
        }

        impl MenuGuard {
            pub fn new() -> Self {
                let mtm = MainThreadMarker::new().expect("tray must be installed on main thread");
                let observer: Retained<MenuObserver> =
                    unsafe { msg_send![mtm.alloc::<MenuObserver>(), init] };
                unsafe {
                    let center = NSNotificationCenter::defaultCenter();
                    center.addObserver_selector_name_object(
                        &observer,
                        sel!(menuDidBeginTracking:),
                        Some(NSMenuDidBeginTrackingNotification),
                        None,
                    );
                    center.addObserver_selector_name_object(
                        &observer,
                        sel!(menuDidEndTracking:),
                        Some(NSMenuDidEndTrackingNotification),
                        None,
                    );
                }
                let ctx = CFRunLoopSourceContext {
                    version: 0,
                    info: ptr::null_mut(),
                    retain: None,
                    release: None,
                    copy_description: None,
                    equal: None,
                    hash: None,
                    schedule: None,
                    cancel: None,
                    perform: Some(on_default_mode),
                };
                let source = unsafe {
                    let s = CFRunLoopSourceCreate(ptr::null(), 0, &ctx);
                    if !s.is_null() {
                        // Default mode only: never fires while AppKit is tracking an open menu.
                        CFRunLoopAddSource(CFRunLoopGetMain(), s, kCFRunLoopDefaultMode);
                    }
                    s
                };
                MenuGuard { observer, source }
            }

            pub fn is_open(&self) -> bool {
                if OPEN_MENUS.with(|n| n.get() > 0) {
                    return true;
                }
                let mode = NSRunLoop::currentRunLoop().currentMode();
                mode.is_some_and(|m| &*m == unsafe { NSEventTrackingRunLoopMode })
            }

            pub fn schedule(&self) {
                if !self.source.is_null() {
                    unsafe {
                        CFRunLoopSourceSignal(self.source);
                        CFRunLoopWakeUp(CFRunLoopGetMain());
                    }
                }
            }
        }

        impl Drop for MenuGuard {
            fn drop(&mut self) {
                unsafe {
                    NSNotificationCenter::defaultCenter().removeObserver(&self.observer);
                    if !self.source.is_null() {
                        CFRunLoopSourceInvalidate(self.source);
                        CFRelease(self.source);
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    mod guard {
        use std::cell::Cell;
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetGUIThreadInfo, KillTimer, SetTimer, GUITHREADINFO, GUI_INMENUMODE, GUI_POPUPMENUMODE,
        };

        unsafe extern "system" fn on_timer(_: HWND, _: u32, id: usize, _: u32) {
            if !in_menu() {
                let _ = unsafe { KillTimer(None, id) };
                super::clear_timer();
                super::flush();
            }
        }

        fn in_menu() -> bool {
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            let ok = unsafe { GetGUIThreadInfo(0, &mut info) }.is_ok();
            ok && (info.flags & (GUI_INMENUMODE | GUI_POPUPMENUMODE)).0 != 0
        }

        pub struct MenuGuard {
            timer: Cell<usize>,
        }

        impl MenuGuard {
            pub fn new() -> Self {
                MenuGuard {
                    timer: Cell::new(0),
                }
            }

            pub fn is_open(&self) -> bool {
                in_menu()
            }

            pub fn schedule(&self) {
                if !self.is_open() {
                    // Apply on the spot when no menu is open.
                    return;
                }
                if self.timer.get() == 0 {
                    let id = unsafe { SetTimer(None, 0, 100, Some(on_timer)) };
                    self.timer.set(id);
                }
            }

            pub fn clear_timer(&self) {
                self.timer.set(0);
            }
        }

        impl Drop for MenuGuard {
            fn drop(&mut self) {
                let id = self.timer.replace(0);
                if id != 0 {
                    let _ = unsafe { KillTimer(None, id) };
                }
            }
        }
    }

    /// The boxed items of one menu level. They must outlive the calls that append them.
    fn items(nodes: &[Node]) -> Vec<Box<dyn IsMenuItem>> {
        nodes
            .iter()
            .map(|n| -> Box<dyn IsMenuItem> {
                match n {
                    Node::Item {
                        id,
                        text,
                        enabled,
                        accelerator,
                    } => {
                        let accel = accelerator.and_then(|a| a.parse::<Accelerator>().ok());
                        Box::new(MenuItem::with_id(id.as_str(), text, *enabled, accel))
                    }
                    Node::Check { id, text, checked } => Box::new(CheckMenuItem::with_id(
                        id.as_str(),
                        text,
                        true,
                        *checked,
                        None,
                    )),
                    Node::Submenu { text, items: sub } => {
                        let menu = Submenu::new(text, true);
                        append(&items(sub), |i| menu.append(i));
                        Box::new(menu)
                    }
                    Node::Separator => Box::new(PredefinedMenuItem::separator()),
                }
            })
            .collect()
    }

    fn append(
        items: &[Box<dyn IsMenuItem>],
        mut add: impl FnMut(&dyn IsMenuItem) -> tray_icon::menu::Result<()>,
    ) {
        for item in items {
            if let Err(e) = add(item.as_ref()) {
                log::warn!("menu: {e}");
            }
        }
    }

    fn build(snap: &Snapshot) -> Menu {
        let menu = Menu::new();
        append(&items(&menu::build(snap)), |i| menu.append(i));
        menu
    }

    fn icon() -> Option<Icon> {
        let img = image::load_from_memory(include_bytes!("../../assets/icons/tray.png"))
            .ok()?
            .into_rgba8();
        let (w, h) = img.dimensions();
        Icon::from_rgba(img.into_raw(), w, h).ok()
    }

    pub fn install(app: &AppHandle, snap: &Snapshot) {
        let handler = app.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            // Menu items read files and talk to the system: never on the UI thread.
            let (app, id) = (handler.clone(), e.id.0.clone());
            host::spawn_blocking(move || menu::on_menu(&app, &id));
        }));
        let mut builder = TrayIconBuilder::new()
            .with_tooltip(menu::tooltip(snap))
            .with_menu(Box::new(build(snap)))
            .with_menu_on_left_click(true)
            // A template image: the system tints it for light and dark menu bars.
            .with_icon_as_template(true);
        if let Some(icon) = icon() {
            builder = builder.with_icon(icon);
        }
        match builder.build() {
            Ok(tray) => {
                log::debug!("menu bar item installed");
                let state = State {
                    tray,
                    pending: None,
                    guard: guard::MenuGuard::new(),
                };
                TRAY.with(|t| *t.borrow_mut() = Some(state));
            }
            Err(e) => log::error!("menu bar item: {e}"),
        }
    }

    #[cfg(target_os = "macos")]
    fn signal_flush() {
        TRAY.with(|t| {
            if let Some(state) = t.borrow().as_ref() {
                if state.pending.is_some() {
                    state.guard.schedule();
                }
            }
        });
    }

    #[cfg(target_os = "windows")]
    fn clear_timer() {
        TRAY.with(|t| {
            if let Some(state) = t.borrow().as_ref() {
                state.guard.clear_timer();
            }
        });
    }

    fn flush() {
        TRAY.with(|t| {
            let mut borrow = t.borrow_mut();
            let Some(state) = borrow.as_mut() else {
                return;
            };
            if state.guard.is_open() {
                return;
            }
            if let Some(snap) = state.pending.take() {
                state.tray.set_menu(Some(Box::new(build(&snap))));
                let _ = state.tray.set_tooltip(Some(menu::tooltip(&snap)));
            }
        });
    }

    pub fn update(snap: &Snapshot) {
        TRAY.with(|t| {
            if let Some(state) = t.borrow_mut().as_mut() {
                state.pending = Some(snap.clone());
                state.guard.schedule();
            }
        });
        #[cfg(target_os = "windows")]
        flush();
    }

    pub fn remove() {
        TRAY.with(|t| t.borrow_mut().take());
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod native {
    use super::*;
    use rune::tray::{Tray, TrayEvent, TrayIcon, TrayItem, TraySpec};
    use std::cell::RefCell;

    thread_local! {
        static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    }

    /// Rune's tray has no submenus or check marks: offer what is used every day and
    /// leave the rest to the settings window. Check marks become a leading tick.
    fn flat(snap: &Snapshot) -> Vec<TrayItem> {
        const KEEP: &[&str] = &[
            "next",
            "prev",
            "viewer",
            "pause-rotation",
            "plate-toggle",
            "settings",
            "quit",
        ];
        let mut out = Vec::new();
        for n in menu::build(snap) {
            match n {
                Node::Item {
                    id, text, enabled, ..
                } if id == "noop" || KEEP.contains(&id.as_str()) => {
                    out.push(TrayItem::action(id, text).enabled(enabled))
                }
                Node::Check { id, text, checked } if KEEP.contains(&id.as_str()) => out.push(
                    TrayItem::action(id, if checked { format!("✓ {text}") } else { text }),
                ),
                Node::Submenu { items, .. } => {
                    for sub in items {
                        if let Node::Check { id, text, checked } = sub {
                            if KEEP.contains(&id.as_str()) {
                                out.push(TrayItem::action(
                                    id,
                                    if checked { format!("✓ {text}") } else { text },
                                ));
                            }
                        }
                    }
                }
                Node::Separator if !matches!(out.last(), Some(TrayItem::Separator) | None) => {
                    out.push(TrayItem::Separator)
                }
                _ => {}
            }
        }
        out
    }

    fn icon() -> Option<TrayIcon> {
        let img = image::load_from_memory(include_bytes!("../../assets/icons/tray.png"))
            .ok()?
            .into_rgba8();
        let (w, h) = img.dimensions();
        Some(TrayIcon::rgba(w, h, img.into_raw()))
    }

    pub fn install(app: &AppHandle, snap: &Snapshot) {
        let Some(icon) = icon() else { return };
        let spec = TraySpec::new(crate::APP_ID, "Galeria", icon)
            .tooltip(menu::tooltip(snap))
            .menu(flat(snap));
        let handler = app.clone();
        let made = Tray::new(spec, move |event| {
            let app = handler.clone();
            let id = match event {
                TrayEvent::Menu(id) => id,
                TrayEvent::Activate { .. } => "settings".to_string(),
                _ => return,
            };
            host::spawn_blocking(move || menu::on_menu(&app, &id));
        });
        match made {
            Ok(tray) => TRAY.with(|t| *t.borrow_mut() = Some(tray)),
            Err(e) => log::warn!("system tray: {e}"),
        }
    }

    pub fn update(snap: &Snapshot) {
        TRAY.with(|t| {
            if let Some(tray) = t.borrow().as_ref() {
                tray.set_menu(flat(snap));
                tray.set_tooltip(menu::tooltip(snap));
            }
        });
    }

    pub fn remove() {
        TRAY.with(|t| t.borrow_mut().take());
    }
}

/// Put Galeria in the menu bar. UI thread only.
pub fn install(app: &AppHandle, snap: &Snapshot) {
    native::install(app, snap);
}

/// Rebuild the menu from a new snapshot. UI thread only.
pub fn update(snap: &Snapshot) {
    native::update(snap);
}

/// Take the item out of the menu bar. UI thread only.
pub fn remove() {
    native::remove();
}
