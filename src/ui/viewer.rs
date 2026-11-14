//! The full-screen viewer: nothing but the painting. Moving the mouse reveals
//! it edge to edge, scrolling or pinching zooms, and a minimap shows where you are.

use super::plate;
use super::theme::{rgb, rgba};
use super::Ctx;
use crate::actions;
use crate::core::{self, PaintingView, Snapshot};
use crate::displays::DisplayInfo;
use crate::images;
use crate::settings::PlatePosition;
use rune::{
    col, el, row, text, Color, Cursor, Cx, Drag, El, Key, KeyEvent, Live, MagnifyEvent, Phase,
    Pixels, PointerEvent, Spring, TextStyle, Transition, Tx, WheelEvent,
};
use std::sync::Arc;
use std::time::Duration;

const BACKGROUND: u32 = 0x050505;
/// Space kept between the plate and the screen's edges.
const PLATE_MARGIN: f32 = 28.0;
const MINIMAP_WIDTH: f32 = 200.0;
const MINIMAP_LINGER: f64 = 1.4;

const HELP: &[(&str, &str)] = &[
    ("Move the mouse", "Reveal the painting edge to edge"),
    ("Scroll / pinch", "Zoom in and out"),
    (
        "Double-click · F",
        "Fit the whole painting / fill the screen",
    ),
    ("0", "Reset zoom"),
    ("← →", "Previous / next painting"),
    ("I", "Show or hide the info plate (drag it anywhere)"),
    ("W", "Use as desktop wallpaper"),
    ("O", "Open the museum page"),
    ("?", "This help"),
    ("Esc", "Close the viewer"),
];

pub struct ViewerUi {
    pub cx: Ctx,
    pub snap: Arc<Snapshot>,
    handle: Option<rune::AppHandle<ViewerUi>>,
    /// The viewer's own window and the display it covers, for going full screen.
    pub window: Option<rune::WindowHandle>,
    pub display: Option<DisplayInfo>,
    /// Decode on the spot instead of on a worker (headless renders).
    immediate: bool,
    /// Every painting that may be shown, for ← and →.
    library: Arc<Vec<PaintingView>>,
    painting: Option<PaintingView>,
    lowres: Option<Pixels>,
    hires: Option<Pixels>,
    /// The viewport in points.
    size: (f32, f32),
    /// Show the whole painting (true) or fill the screen (false) at zoom 1.
    fit: bool,
    zoom: f64,
    /// The pointer, 0..1 across the viewport.
    mouse: (f64, f64),
    /// Where the painting is drawn: its top-left, and its size relative to filling the screen.
    offset: Live<[f32; 2]>,
    scale: Live<f32>,
    /// The part of the painting on screen, on the minimap.
    mini: Live<[f32; 4]>,
    mini_opacity: Live<f32>,
    mini_until: f64,
    plate_visible: bool,
    drag: Option<PlatePosition>,
    dragging: bool,
    help: bool,
    toast: Option<(String, f64)>,
    clock: f64,
}

impl ViewerUi {
    pub fn new(
        cx: Ctx,
        snap: Arc<Snapshot>,
        handle: Option<rune::AppHandle<ViewerUi>>,
    ) -> ViewerUi {
        let plate_visible = snap.settings.viewer.show_plate;
        ViewerUi {
            cx,
            snap,
            handle,
            window: None,
            display: None,
            immediate: false,
            library: Arc::new(Vec::new()),
            painting: None,
            lowres: None,
            hires: None,
            size: (1512.0, 982.0),
            fit: false,
            zoom: 1.0,
            mouse: (0.5, 0.5),
            offset: Live::new([0.0, 0.0]),
            scale: Live::new(1.0),
            mini: Live::new([0.0, 0.0, MINIMAP_WIDTH, 0.0]),
            mini_opacity: Live::new(0.0),
            mini_until: 0.0,
            plate_visible,
            drag: None,
            dragging: false,
            help: false,
            toast: None,
            clock: 0.0,
        }
    }

    pub fn immediate(mut self) -> ViewerUi {
        self.immediate = true;
        self
    }

    pub fn apply(&mut self, snap: Arc<Snapshot>) {
        // Details looked up online arrive after the painting is already open.
        if let Some(p) = &self.painting {
            if let Some(fresh) = snap
                .displays
                .iter()
                .filter_map(|d| d.painting.as_ref())
                .find(|f| f.id == p.id)
            {
                self.painting = Some(fresh.clone());
            }
        }
        if !self.dragging {
            self.drag = None;
        }
        self.snap = snap;
        self.retarget(false);
    }

    /// Show `id` from `library` on `display`.
    pub fn open(
        &mut self,
        id: &str,
        library: Arc<Vec<PaintingView>>,
        display: Option<DisplayInfo>,
        tx: &mut Tx,
    ) {
        self.library = library;
        if let Some(d) = &display {
            self.size = (d.width as f32, d.height as f32);
        }
        self.display = display;
        self.plate_visible = self.snap.settings.viewer.show_plate;
        self.help = false;
        self.clock = tx.clock();
        self.load(id);
        self.say(
            "Move to explore · scroll to zoom · ← → browse · I info · ? help · Esc close",
            4.2,
            tx,
        );
    }

    /// Open whatever the first display shows, with the whole library to browse (headless renders).
    pub fn showing_current(mut self) -> ViewerUi {
        let core = self.cx.app.core();
        let library = Arc::new(core.library_view().paintings);
        let display = self.snap.displays.first().cloned();
        let id = display
            .as_ref()
            .and_then(|d| d.painting.as_ref().map(|p| p.id.clone()))
            .or_else(|| library.first().map(|p| p.id.clone()));
        if let Some(d) = &display {
            self.size = (d.info.width as f32, d.info.height as f32);
        }
        self.display = display.map(|d| d.info);
        self.library = library;
        if let Some(id) = id {
            self.load(&id);
        }
        self
    }

    fn load(&mut self, id: &str) {
        let Some(p) = self.library.iter().find(|p| p.id == id).cloned() else {
            return;
        };
        self.zoom = 1.0;
        self.lowres = Some(self.cx.art.thumb(&p.id, p.bytes, images::THUMB_EDGE));
        self.hires = None;
        // The whole scan, as large as one texture can be.
        let (tw, th) = images::clamp_to_texture(p.width, p.height);
        let id = p.id.clone();
        self.painting = Some(p);
        self.retarget(true);
        if self.immediate {
            let core = self.cx.app.core();
            if let Some(Ok(rgba)) = core
                .source_for(&id)
                .map(|src| images::render_pixels(&src, tw, th, &core.paths.cache))
            {
                let pixels = Pixels::new();
                pixels.set(rgba.width, rgba.height, rgba.data);
                self.hires = Some(pixels);
            }
            return;
        }
        let Some(handle) = self.handle.clone() else {
            return;
        };
        self.cx.art.render(&id.clone(), tw, th, move |result| {
            handle.update(move |ui: &mut ViewerUi, tx| {
                if ui.painting.as_ref().is_none_or(|p| p.id != id) {
                    return; // Moved on to another painting meanwhile.
                }
                match result {
                    Ok(pixels) => ui.hires = Some(pixels),
                    Err(e) => {
                        log::warn!("viewer: {id}: {e}");
                        ui.say("Could not load the full-resolution image", 2.2, tx);
                    }
                }
            });
        });
    }

    fn neighbour(&mut self, step: isize) {
        let list: Vec<&PaintingView> = self.library.iter().filter(|p| !p.excluded).collect();
        if list.is_empty() {
            return;
        }
        let current = self.painting.as_ref().map(|p| p.id.as_str());
        let i = list
            .iter()
            .position(|p| Some(p.id.as_str()) == current)
            .unwrap_or(0) as isize;
        let n = list.len() as isize;
        let next = list[((i + step).rem_euclid(n)) as usize].id.clone();
        self.load(&next);
    }

    fn say(&mut self, message: &str, seconds: f64, tx: &mut Tx) {
        self.toast = Some((message.to_string(), self.clock + seconds));
        tx.wake_after(Duration::from_secs_f64(seconds));
    }

    /// (cover, contain): the scales at which the painting fills, and fits, the screen.
    fn scales(&self) -> (f64, f64) {
        let Some(p) = &self.painting else {
            return (1.0, 1.0);
        };
        let (vw, vh) = (self.size.0 as f64, self.size.1 as f64);
        let (pw, ph) = (p.width.max(1) as f64, p.height.max(1) as f64);
        ((vw / pw).max(vh / ph), (vw / pw).min(vh / ph))
    }

    /// (min zoom, max zoom, scale at zoom 1).
    fn zoom_limits(&self) -> (f64, f64, f64) {
        let (cover, contain) = self.scales();
        let base = if self.fit { contain } else { cover };
        let device = self
            .display
            .as_ref()
            .map(|d| d.scale)
            .unwrap_or(2.0)
            .max(0.5);
        // Up to two and a half screen pixels per pixel of the scan.
        (
            (contain / base).min(1.0),
            (2.5 / device / base).max(2.0),
            base,
        )
    }

    fn set_zoom(&mut self, zoom: f64) {
        let (min, max, _) = self.zoom_limits();
        self.zoom = zoom.clamp(min, max);
    }

    /// Whether the painting is larger than the screen, so there is somewhere to travel.
    fn overflowing(&self) -> bool {
        let Some(p) = &self.painting else {
            return false;
        };
        let s = self.zoom_limits().2 * self.zoom;
        p.width as f64 * s > self.size.0 as f64 + 1.0
            || p.height as f64 * s > self.size.1 as f64 + 1.0
    }

    /// Work out where the painting should be for the pointer and zoom, and send it
    /// there. `jump` skips the glide (a new painting, a resized window).
    fn retarget(&mut self, jump: bool) {
        let Some(p) = &self.painting else { return };
        let (cover, _) = self.scales();
        let s = self.zoom_limits().2 * self.zoom;
        let (w, h) = (p.width as f64 * s, p.height as f64 * s);
        let (vw, vh) = (self.size.0 as f64, self.size.1 as f64);
        let dz = self.snap.settings.viewer.edge_dead_zone.clamp(0.0, 0.45);
        let t = |m: f64| ((m - dz) / (1.0 - 2.0 * dz)).clamp(0.0, 1.0);
        let x = if w > vw {
            -(w - vw) * t(self.mouse.0)
        } else {
            (vw - w) / 2.0
        };
        let y = if h > vh {
            -(h - vh) * t(self.mouse.1)
        } else {
            (vh - h) / 2.0
        };

        let (mw, mh) = (
            MINIMAP_WIDTH as f64,
            MINIMAP_WIDTH as f64 * p.height as f64 / p.width.max(1) as f64,
        );
        let mini = [
            ((-x / w).clamp(0.0, 1.0) * mw) as f32,
            ((-y / h).clamp(0.0, 1.0) * mh) as f32,
            ((vw / w).clamp(0.0, 1.0) * mw) as f32,
            ((vh / h).clamp(0.0, 1.0) * mh) as f32,
        ];
        let (offset, scale) = ([x as f32, y as f32], (s / cover) as f32);
        if jump {
            // Fresh values start where they are set instead of gliding from the old painting's.
            self.offset = Live::new(offset);
            self.scale = Live::new(scale);
            self.mini = Live::new(mini);
        } else {
            self.offset.set(offset);
            self.scale.set(scale);
            self.mini.set(mini);
        }
    }

    /// Show the minimap for a moment.
    fn poke_minimap(&mut self, tx: &mut Tx) {
        if !self.snap.settings.viewer.minimap || !self.overflowing() {
            return;
        }
        self.mini_opacity.set(1.0);
        self.mini_until = tx.clock() + MINIMAP_LINGER;
        tx.wake_after(Duration::from_secs_f64(MINIMAP_LINGER));
    }

    /// How quickly the painting follows the pointer, from the "motion smoothing" setting.
    fn glide(&self) -> Option<Spring> {
        let smoothing = self.snap.settings.viewer.smoothing.clamp(0.0, 0.97);
        if smoothing < 0.05 {
            return None;
        }
        // The setting is the share of the distance left after one 60 Hz frame.
        let time_constant = -(1.0 / 60.0) / smoothing.ln();
        Some(Spring::new(
            (4.0 * time_constant).clamp(0.03, 1.6) as f32,
            0.0,
        ))
    }

    fn area(&self) -> [f32; 4] {
        let m = PLATE_MARGIN;
        [m, m, self.size.0 - 2.0 * m, self.size.1 - 2.0 * m]
    }

    fn close(&mut self) {
        self.cx.run(|app| {
            if let Some(shell) = app.shell() {
                shell.close_viewer();
            }
        });
    }
}

/// Named states for `galeria ui --window viewer --scene <name>`.
pub const SCENES: &[crate::ui::Scene<ViewerUi>] = &[
    ("help", |s| s.help = true),
    ("fit", |s| {
        s.fit = true;
        s.retarget(true);
    }),
    ("left", |s| {
        s.mouse = (0.0, 0.5);
        s.retarget(true);
    }),
    ("right", |s| {
        s.mouse = (1.0, 0.5);
        s.retarget(true);
        s.mini_opacity = Live::new(1.0);
    }),
    ("zoomed", |s| {
        s.set_zoom(s.zoom * 2.5);
        s.mouse = (0.35, 0.4);
        s.retarget(true);
        s.mini_opacity = Live::new(1.0);
    }),
    ("no-plate", |s| s.plate_visible = false),
    ("toast", |s| {
        s.toast = Some(("Set as desktop wallpaper".into(), f64::MAX))
    }),
];

pub fn poll(s: &mut ViewerUi, clock: f64, _: &mut Tx) -> bool {
    s.clock = clock;
    if s.mini_until > 0.0 && clock >= s.mini_until {
        s.mini_until = 0.0;
        s.mini_opacity.set(0.0);
    }
    if s.toast.as_ref().is_some_and(|(_, until)| clock >= *until) {
        s.toast = None;
        return true;
    }
    false
}

pub fn on_key(s: &mut ViewerUi, ev: &KeyEvent, tx: &mut Tx) -> bool {
    s.clock = tx.clock();
    if ev.mods.cmd || ev.mods.ctrl {
        return false;
    }
    let typed = match &ev.key {
        Key::Text(t) => t.to_lowercase(),
        _ => String::new(),
    };
    match (&ev.key, typed.as_str()) {
        (Key::Escape, _) => {
            if s.help {
                s.help = false;
            } else {
                s.close();
            }
        }
        (Key::Right | Key::Down, _) => s.neighbour(1),
        (Key::Left | Key::Up, _) => s.neighbour(-1),
        (_, "i") => {
            s.plate_visible = !s.plate_visible;
            s.cx.patch(serde_json::json!({ "viewer": { "showPlate": s.plate_visible } }));
        }
        (_, "f") => {
            s.fit = !s.fit;
            s.zoom = 1.0;
            s.retarget(false);
        }
        (_, "0") => {
            s.zoom = 1.0;
            s.retarget(false);
        }
        (_, "+" | "=") => {
            s.set_zoom(s.zoom * 1.25);
            s.retarget(false);
            s.poke_minimap(tx);
        }
        (_, "-" | "_") => {
            s.set_zoom(s.zoom / 1.25);
            s.retarget(false);
            s.poke_minimap(tx);
        }
        (_, "w") => {
            if let Some(p) = &s.painting {
                let (id, display) = (p.id.clone(), s.display.as_ref().map(|d| d.key.clone()));
                s.cx.run(move |app| {
                    if let Err(e) = core::show(app, &id, display) {
                        log::warn!("viewer: {e}");
                    }
                });
                s.say("Set as desktop wallpaper", 2.2, tx);
            }
        }
        (_, "o") => {
            if let Some(url) = s.painting.as_ref().and_then(|p| p.source_page.clone()) {
                let _ = actions::open_url(&url);
            }
        }
        (_, "?" | "h") => s.help = !s.help,
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn glass(e: El) -> El {
    e.backdrop_blur(24.0)
        .fill(rgba(0x141416, 0.55))
        .border(1.0, Color::WHITE.alpha(0.12))
}

fn help_panel() -> El {
    let white = Color::WHITE;
    let mut keys = col().gap(6.0).items_start();
    let mut what = col().gap(6.0).items_start();
    for (key, does) in HELP {
        keys = keys.child(
            text(*key)
                .font(TextStyle::new(13.0, 600.0).lh(20.0))
                .color(white.alpha(0.9)),
        );
        what = what.child(
            text(*does)
                .font(TextStyle::new(13.0, 400.0).lh(20.0))
                .color(white.alpha(0.72)),
        );
    }
    glass(
        col()
            .key("help")
            .role("dialog")
            .label("Viewer shortcuts")
            .px(30.0)
            .py(24.0)
            .radius(22.0)
            .shadow(30.0, 80.0, Color::BLACK.alpha(0.5))
            .transition(Transition::scale(0.97)),
    )
    .child(
        text("Viewer")
            .font(TextStyle::new(17.0, 600.0))
            .color(white)
            .mb(12.0),
    )
    .child(row().gap(22.0).child(keys).child(what))
}

pub fn view(s: &ViewerUi, cx: &mut Cx<ViewerUi>) -> El {
    s.cx.report_dark(cx.system_dark());
    let (vw, vh) = s.size;
    let mut root = el()
        .key("viewer")
        .w_full()
        .h_full()
        .fill(rgb(BACKGROUND))
        .on_layout(|s: &mut ViewerUi, [_, _, w, h]| {
            if w > 0.0 && h > 0.0 && (s.size.0 - w).abs() + (s.size.1 - h).abs() > 0.5 {
                s.size = (w, h);
                s.retarget(true);
            }
        })
        .on_pointer_move_local(|s: &mut ViewerUi, e: PointerEvent, tx| {
            if s.dragging {
                return;
            }
            s.mouse = (
                (e.x / s.size.0.max(1.0)) as f64,
                (e.y / s.size.1.max(1.0)) as f64,
            );
            s.retarget(false);
            s.poke_minimap(tx);
            tx.live_only();
        })
        .on_wheel(|s: &mut ViewerUi, e: WheelEvent, tx| {
            s.set_zoom(s.zoom * (e.dy as f64 * 0.0022).exp());
            s.retarget(false);
            s.poke_minimap(tx);
            tx.live_only();
            true
        })
        .on_magnify(|s: &mut ViewerUi, e: MagnifyEvent, tx| {
            s.set_zoom(s.zoom * (1.0 + e.delta as f64));
            s.retarget(false);
            s.poke_minimap(tx);
            tx.live_only();
            true
        })
        .on_double_press(|s: &mut ViewerUi| {
            s.fit = !s.fit;
            s.zoom = 1.0;
            s.retarget(false);
        });
    if s.snap.settings.viewer.hide_cursor && !s.help {
        root = root.cursor(Cursor::Hidden);
    }
    let Some(p) = &s.painting else { return root };

    // The painting, laid out filling the screen and then moved and scaled as one piece.
    let (cover, _) = s.scales();
    let (cw, ch) = (
        (p.width as f64 * cover) as f32,
        (p.height as f64 * cover) as f32,
    );
    let glide = s.glide();
    let mut canvas = el()
        .key("canvas")
        .absolute(0.0, 0.0)
        .size(cw, ch)
        .origin(0.0, 0.0);
    canvas = match glide {
        Some(spring) => canvas
            .live_offset(s.offset.smooth(spring))
            .live_scale(s.scale.smooth(spring)),
        None => canvas.live_offset(&s.offset).live_scale(&s.scale),
    };
    if let Some(low) = &s.lowres {
        canvas = canvas.child(
            el().key("lowres")
                .size(cw, ch)
                .absolute(0.0, 0.0)
                .pixels(low)
                .a11y_hidden(),
        );
    }
    if let Some(hi) = &s.hires {
        canvas = canvas.child(
            el().key(format!("hires-{}", p.id))
                .role("img")
                .label(p.title.clone())
                .size(cw, ch)
                .absolute(0.0, 0.0)
                .pixels(hi)
                .transition(
                    Transition::opacity()
                        .springs(rune::SpringPair::new(Spring::new(0.6, 0.0), Spring::NONE)),
                ),
        );
    }
    root = root.child(canvas);

    // The info plate, where it was last left.
    let vs = &s.snap.settings.viewer;
    let ps = &s.snap.settings.plate;
    let style = plate::resolve(&ps.style, &s.snap.custom_styles);
    let pos = s.drag.unwrap_or(vs.plate_position);
    let label = plate::render(p, ps, &style, &s.cx.art, 1.0)
        .cursor(if s.dragging {
            Cursor::Grabbing
        } else {
            Cursor::Grab
        })
        .on_drag(|s: &mut ViewerUi, g: &mut Drag, tx| {
            let [px, py] = g.window_point();
            let at = plate::position_at(px - g.start_x, py - g.start_y, [g.w, g.h], s.area());
            match g.phase {
                Phase::Down => s.dragging = true,
                Phase::Move => {
                    s.drag = Some(at);
                    tx.instant();
                }
                Phase::Up => {
                    s.dragging = false;
                    if g.cancelled() {
                        s.drag = None;
                    } else if s.drag.is_some() {
                        s.drag = Some(at);
                        s.cx.run(move |app| {
                            core::set_plate_position(app, "viewer", None, at.x, at.y)
                        });
                    }
                }
            }
        });
    let mut placed = plate::place(label, pos, s.area())
        .key("plate-layer")
        .opacity(if s.plate_visible { 1.0 } else { 0.0 })
        .opacity_motion(rune::Curve::ease_in_out(0.35));
    if !s.plate_visible {
        placed = placed.inert();
    }
    root = root.child(placed);

    // Where you are on the whole painting.
    if vs.minimap {
        let mh = MINIMAP_WIDTH * p.height as f32 / p.width.max(1) as f32;
        let mut map = el()
            .key("minimap")
            .absolute((vw - MINIMAP_WIDTH) / 2.0, vh - 28.0 - mh)
            .size(MINIMAP_WIDTH, mh)
            .radius(6.0)
            .clip_rounded(6.0)
            .fill(rgb(0x111111))
            .border(1.0, Color::WHITE.alpha(0.25))
            .shadow(10.0, 30.0, Color::BLACK.alpha(0.5))
            .live_opacity(s.mini_opacity.smooth(Spring::new(0.4, 0.0)))
            .inert()
            .a11y_hidden();
        if let Some(low) = &s.lowres {
            map = map.child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .pixels(low),
            );
        }
        map = map
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(Color::BLACK.alpha(0.5)),
            )
            .child({
                let frame = el()
                    .key("minimap-view")
                    .absolute(0.0, 0.0)
                    .radius(2.0)
                    .fill(Color::WHITE.alpha(0.22))
                    .border(1.5, Color::WHITE);
                match glide {
                    Some(spring) => frame.live_rect(s.mini.smooth(spring)),
                    None => frame.live_rect(&s.mini),
                }
            });
        root = root.child(map);
    }

    if let Some((message, _)) = &s.toast {
        root = root.child(
            el().key("toast-layer")
                .pin(Some(0.0), Some(34.0), Some(0.0), None)
                .center()
                .inert()
                .child(
                    glass(
                        row()
                            .key("toast")
                            .role("status")
                            .h(34.0)
                            .px(18.0)
                            .center()
                            .radius(17.0),
                    )
                    .transition(Transition::offset(0.0, -8.0).combined(Transition::opacity()))
                    .child(
                        text(message.clone())
                            .font(TextStyle::new(13.0, 400.0))
                            .color(Color::WHITE),
                    ),
                ),
        );
    }
    if s.help {
        root = root.child(
            el().key("help-layer")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .on_press(|s: &mut ViewerUi| s.help = false)
                .child(help_panel()),
        );
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui(dir: &std::path::Path, painting: (u32, u32)) -> ViewerUi {
        let app = crate::host::AppHandle::new(core::Core::new(core::Paths::in_dir(dir).unwrap()));
        let snap = Arc::new(app.core().snapshot());
        let mut s = ViewerUi::new(Ctx::new(app), snap, None);
        let mut p = PaintingView::sample();
        (p.width, p.height) = painting;
        s.size = (1500.0, 1000.0);
        s.painting = Some(p);
        s
    }

    #[test]
    fn the_mouse_travels_a_wide_painting_edge_to_edge() {
        let dir = std::env::temp_dir().join(format!("galeria-view-{}", fastrand::u64(..)));
        // A 3:1 panorama on a 3:2 screen fills the height and overflows sideways.
        let mut s = ui(&dir, (6000, 2000));
        let (cover, contain) = s.scales();
        assert!((cover - 0.5).abs() < 1e-9 && (contain - 0.25).abs() < 1e-9);
        assert!(s.overflowing());

        s.mouse = (0.0, 0.5);
        s.retarget(true);
        assert_eq!(
            s.offset.get(),
            [0.0, 0.0],
            "left edge of the painting at the left of the screen"
        );
        s.mouse = (1.0, 0.5);
        s.retarget(false);
        assert_eq!(s.offset.get(), [-1500.0, 0.0], "right edge at the right");
        // The dead zone: the last 8% of the screen is already the edge.
        s.mouse = (0.95, 0.5);
        s.retarget(false);
        assert_eq!(s.offset.get(), [-1500.0, 0.0]);
        s.mouse = (0.5, 0.5);
        s.retarget(false);
        assert_eq!(s.offset.get(), [-750.0, 0.0]);
        assert_eq!(s.scale.get(), 1.0);
        // The minimap shows the middle half of the painting.
        let [mx, _, mw, mh] = s.mini.get();
        assert!(
            (mx - 50.0).abs() < 1e-3 && (mw - 100.0).abs() < 1e-3,
            "{mx} {mw}"
        );
        assert!((mh - MINIMAP_WIDTH / 3.0).abs() < 1e-3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn fitting_shows_the_whole_painting_centred() {
        let dir = std::env::temp_dir().join(format!("galeria-view-{}", fastrand::u64(..)));
        let mut s = ui(&dir, (6000, 2000));
        s.fit = true;
        s.retarget(true);
        assert!(!s.overflowing());
        // 1500 × 500 in a 1500 × 1000 screen: centred vertically, half the fill scale.
        assert_eq!(s.offset.get(), [0.0, 250.0]);
        assert_eq!(s.scale.get(), 0.5);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn zoom_stays_between_the_whole_painting_and_past_one_to_one() {
        let dir = std::env::temp_dir().join(format!("galeria-view-{}", fastrand::u64(..)));
        let mut s = ui(&dir, (6000, 2000));
        let (min, max, base) = s.zoom_limits();
        assert!(
            (min - 0.5).abs() < 1e-9,
            "zooming out stops at the whole painting"
        );
        // 2.5 screen pixels per scan pixel on a 2x display is 1.25 points per pixel.
        assert!((max * base - 1.25).abs() < 1e-9);
        s.set_zoom(1000.0);
        assert_eq!(s.zoom, max);
        s.set_zoom(0.0);
        assert_eq!(s.zoom, min);
        // A small scan can still be doubled.
        let small = ui(&dir, (900, 600));
        assert_eq!(small.zoom_limits().1, 2.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn smoothing_maps_to_a_glide() {
        let dir = std::env::temp_dir().join(format!("galeria-view-{}", fastrand::u64(..)));
        let mut s = ui(&dir, (6000, 2000));
        let set = |s: &mut ViewerUi, v: f64| {
            let mut snap = (*s.snap).clone();
            snap.settings.viewer.smoothing = v;
            s.snap = Arc::new(snap);
        };
        set(&mut s, 0.0);
        assert!(s.glide().is_none(), "direct");
        set(&mut s, 0.6);
        let smooth = s.glide().unwrap().duration;
        set(&mut s, 0.95);
        let floaty = s.glide().unwrap().duration;
        assert!(smooth > 0.08 && smooth < 0.2, "{smooth}");
        assert!(floaty > 1.0, "{floaty}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
