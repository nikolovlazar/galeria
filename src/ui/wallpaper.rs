//! The desktop: one window per display, behind the icons. It hangs the
//! painting (filling the screen, panning across it, or framed on a wall),
//! crossfades to the next one and shows the info plate.

use super::fx::{self, Finish, Material};
use super::plate;
use super::stage::{self, Axis, Layout, Pan};
use super::theme::{hex_or, rgb, rgba};
use super::Ctx;
use crate::core::{self, DisplayView, LayoutKind, PaintingView, Snapshot};
use crate::images;
use crate::settings::{FrameStyle, Matte, PlatePosition, WallpaperSettings};
use rune::{
    col, el, row, text, Color, Cursor, Cx, Drag, El, Key, KeyEvent, Live, Phase, Pixels, Shadow,
    Spring, SpringPair, TextStyle, Transition, Tx,
};
use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Duration;

/// Behind everything, while the first painting decodes.
const BACKGROUND: u32 = 0x0e0e0e;

/// A painting as it hangs on this display.
struct Layer {
    /// Everything the hanging depends on; a different key means a new layer.
    key: String,
    /// Names the element, so the next layer fades in over this one.
    generation: u64,
    painting: PaintingView,
    pixels: Pixels,
    layout: Layout,
    wallpaper: WallpaperSettings,
    /// The wall behind a painting shown in full.
    wall: Color,
    /// A tiny copy of the painting, blown up and blurred across the wall.
    backdrop: Option<Pixels>,
    pan: Option<Pan>,
    /// The pan timeline's zero on the window's clock.
    pan_origin: f64,
    /// Where the pan stopped, while paused.
    pan_frozen: Option<f64>,
    offset: Live<[f32; 2]>,
    /// Seconds the layer takes to fade in.
    fade: f32,
}

impl Layer {
    fn pan_time(&self, clock: f64) -> f64 {
        self.pan_frozen.unwrap_or(clock - self.pan_origin)
    }
}

/// What a layer is loaded from, sent to a worker and back.
struct Request {
    key: String,
    painting: PaintingView,
    layout: Layout,
    wallpaper: WallpaperSettings,
    shown_at: u64,
    target: (u32, u32),
}

pub struct WallUi {
    pub cx: Ctx,
    pub snap: Arc<Snapshot>,
    pub display_key: String,
    /// Routes decoded paintings back to this window. `None` in tests.
    handle: Option<rune::AppHandle<WallUi>>,
    /// Decode on the spot instead of on a worker (headless renders).
    immediate: bool,
    generation: u64,
    shown: Option<Layer>,
    /// The previous painting, kept under the new one until it has faded in.
    leaving: Option<(Layer, f64)>,
    loading: Option<String>,
    clock: f64,
    /// When the next timer wake is due; none is asked for before that.
    wake_due: f64,
    /// The painting whose plate was last brought back into view.
    plate_for: Option<String>,
    plate_was_on: bool,
    /// Faded out by "fade out after".
    plate_asleep: bool,
    sleep_at: Option<f64>,
    /// The plate's position while it is being dragged (and until the setting follows).
    drag: Option<PlatePosition>,
    dragging: bool,
}

fn now_unix() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

impl WallUi {
    pub fn new(
        cx: Ctx,
        snap: Arc<Snapshot>,
        display_key: String,
        handle: Option<rune::AppHandle<WallUi>>,
    ) -> WallUi {
        let mut ui = WallUi {
            cx,
            snap,
            display_key,
            handle,
            immediate: false,
            generation: 0,
            shown: None,
            leaving: None,
            loading: None,
            clock: 0.0,
            wake_due: 0.0,
            plate_for: None,
            plate_was_on: false,
            plate_asleep: false,
            sleep_at: None,
            drag: None,
            dragging: false,
        };
        ui.sync();
        ui
    }

    /// Decode paintings on the calling thread, so the very first frame shows one.
    pub fn immediate(mut self) -> WallUi {
        self.immediate = true;
        self.loading = None;
        self.sync();
        self
    }

    pub fn display(&self) -> Option<&DisplayView> {
        self.snap
            .displays
            .iter()
            .find(|d| d.info.key == self.display_key)
            .or_else(|| self.snap.displays.first())
    }

    /// A new snapshot arrived.
    pub fn apply(&mut self, snap: Arc<Snapshot>, tx: &mut Tx) {
        self.snap = snap;
        if !self.dragging {
            self.drag = None;
        }
        self.clock = tx.clock();
        self.sync();
        self.refresh_plate();
        self.schedule(tx);
    }

    /// Start loading whatever this display should show, unless it already does.
    fn sync(&mut self) {
        let Some(d) = self.display().cloned() else {
            return self.clear();
        };
        let Some(p) = d.painting.clone() else {
            return self.clear();
        };
        let w = self.snap.settings.wallpaper.clone();
        let (vw, vh) = (d.info.width, d.info.height);
        let layout = stage::compute(
            p.width as f64,
            p.height as f64,
            vw,
            vh,
            &w,
            d.info.insets.top,
        );
        let key = format!(
            "{}|{}|{vw}x{vh}|{}|{:?}|{}|{}|{}|{}|{:?}|{}|{:?}|{}|{}|{}",
            p.id,
            p.bytes,
            d.info.insets.top,
            w.fit,
            w.crop_tolerance,
            w.pan,
            w.pan_speed,
            w.pan_pause,
            w.matte,
            w.matte_color,
            w.frame,
            w.fit_margin,
            w.shadow,
            w.spotlight
        );
        if let Some(l) = self.shown.as_mut().filter(|l| l.key == key) {
            // Same hanging; only the pause switch can have changed.
            self.loading = None;
            let t = l.pan_time(self.clock);
            match (w.pan_paused, l.pan_frozen) {
                (true, None) => l.pan_frozen = Some(t),
                (false, Some(t)) => {
                    l.pan_origin = self.clock - t;
                    l.pan_frozen = None;
                }
                _ => {}
            }
            l.wallpaper.pan_paused = w.pan_paused;
            return;
        }
        if self.loading.as_deref() == Some(key.as_str()) {
            return;
        }
        self.loading = Some(key.clone());

        // Exactly the pixels the painting covers on this display, and never more than it has.
        let scale = d.info.scale;
        let target =
            images::target_size(p.width, p.height, layout.w * scale, layout.h * scale, true);
        let request = Request {
            key,
            painting: p,
            layout,
            wallpaper: w,
            shown_at: d.shown_at,
            target,
        };
        if self.immediate {
            let core = self.cx.app.core();
            let (wall, result) = load(core, &request);
            let result = result.map(|rgba| {
                let pixels = Pixels::new();
                pixels.set(rgba.width, rgba.height, rgba.data);
                pixels
            });
            self.loaded(request, wall, result);
            return;
        }
        let Some(handle) = self.handle.clone() else {
            return;
        };
        let app = self.cx.app.clone();
        let (id, (tw, th)) = (request.painting.id.clone(), request.target);
        self.cx.art.render(&id, tw, th, move |result| {
            let wall = wall_color(app.core(), &request);
            handle.update(move |ui: &mut WallUi, tx| {
                ui.clock = tx.clock();
                ui.loaded(request, wall, result);
                ui.schedule(tx);
            });
        });
    }

    fn clear(&mut self) {
        self.loading = None;
        self.shown = None;
        self.leaving = None;
    }

    /// A painting finished decoding (or failed to).
    fn loaded(&mut self, r: Request, wall: Color, result: Result<Pixels, String>) {
        if self.loading.as_deref() != Some(r.key.as_str()) {
            return; // Something newer was asked for meanwhile.
        }
        self.loading = None;
        let pixels = match result {
            Ok(p) => p,
            Err(e) => {
                log::warn!("could not show {}: {e}", r.painting.id);
                let (id, display) = (r.painting.id.clone(), self.display_key.clone());
                self.cx
                    .run(move |app| core::report_bad_image(app, &id, Some(display)));
                return;
            }
        };
        let w = &r.wallpaper;
        let old = self.shown.take();
        let same = old.as_ref().is_some_and(|o| o.painting.id == r.painting.id);
        let pan = Pan::new(&r.layout, w.pan_speed, w.pan_pause);
        // A re-layout of the same painting keeps its place in the sweep; a fresh
        // painting starts from when it appeared, so a restart doesn't jump back.
        let elapsed = match (&old, &pan) {
            (Some(o), Some(new)) if same && o.pan.is_some() => {
                o.pan
                    .map(|p| p.progress(o.pan_time(self.clock)))
                    .unwrap_or(0.0)
                    * new.cycle_secs()
            }
            _ if r.shown_at > 0 && !self.immediate => (now_unix() - r.shown_at as f64).max(0.0),
            _ => 0.0,
        };
        let fade = match &old {
            Some(_) if same => 0.35,
            Some(_) => w.transition_seconds as f32,
            None => (w.transition_seconds as f32).min(0.8),
        };
        let backdrop = (r.layout.kind == LayoutKind::Contain && w.matte == Matte::Blur)
            .then(|| self.cx.art.thumb(&r.painting.id, r.painting.bytes, 96));
        self.generation += 1;
        let layer = Layer {
            key: r.key,
            generation: self.generation,
            pixels,
            layout: r.layout,
            wall,
            backdrop,
            pan,
            pan_origin: self.clock - elapsed,
            pan_frozen: w.pan_paused.then_some(elapsed),
            offset: Live::new([0.0, 0.0]),
            fade,
            wallpaper: r.wallpaper,
            painting: r.painting,
        };
        if let Some(o) = old {
            // Keep the old painting underneath until the new one is opaque.
            self.leaving = Some((o, self.clock + fade as f64 + 0.25));
        }
        self.shown = Some(layer);
        self.step_pan();
        self.refresh_plate();
    }

    /// Bring the plate back when a new painting appears or it is switched on, and
    /// start the "fade out after" timer.
    fn refresh_plate(&mut self) {
        let ps = &self.snap.settings.plate;
        let on = ps.visible || self.snap.arrange;
        let id = self.shown.as_ref().map(|l| l.painting.id.clone());
        let reveal = (id.is_some() && id != self.plate_for) || (on && !self.plate_was_on);
        self.plate_for = id;
        self.plate_was_on = on;
        if reveal {
            self.plate_asleep = false;
            self.sleep_at =
                (ps.auto_hide_seconds > 0).then_some(self.clock + ps.auto_hide_seconds as f64);
        }
        if self.snap.arrange || ps.auto_hide_seconds == 0 {
            self.plate_asleep = false;
            self.sleep_at = None;
        }
    }

    /// Move the panning painting to where the timeline says it is now.
    fn step_pan(&mut self) -> Option<f64> {
        let clock = self.clock;
        let scale = self.display().map(|d| d.info.scale).unwrap_or(2.0);
        let l = self.shown.as_ref()?;
        let (pan, axis) = (l.pan?, l.layout.axis?);
        let t = l.pan_time(clock);
        let v = stage::snap(pan.offset_at(t), scale) as f32;
        l.offset.set(match axis {
            Axis::X => [v, 0.0],
            Axis::Y => [0.0, v],
        });
        l.pan_frozen.is_none().then(|| pan.next_step(t, scale))
    }

    /// Ask for the next timer wake: the pan's next pixel, the old layer's removal,
    /// the plate's fade-out. One wake at a time, however often frames run.
    fn schedule(&mut self, tx: &mut Tx) {
        let mut next: Option<f64> = self.step_pan();
        let mut soonest = |at: f64, clock: f64| {
            let wait = (at - clock).max(0.0);
            next = Some(next.map_or(wait, |n: f64| n.min(wait)));
        };
        if let Some((_, at)) = &self.leaving {
            soonest(*at, self.clock);
        }
        if let Some(at) = self.sleep_at {
            soonest(at, self.clock);
        }
        let Some(wait) = next else { return };
        let due = self.clock + wait;
        // A wake already due no later than this one will reschedule when it fires.
        if self.wake_due > self.clock + 1e-4 && self.wake_due <= due + 1e-4 {
            return;
        }
        self.wake_due = due;
        tx.wake_after(Duration::from_secs_f64(wait.max(0.001)));
    }

    fn area(&self) -> [f32; 4] {
        let Some(d) = self.display() else {
            return [0.0; 4];
        };
        let m = self.snap.settings.plate.margin as f32;
        let ins = &d.info.insets;
        let (vw, vh) = (d.info.width as f32, d.info.height as f32);
        let (l, t, r, b) = (
            ins.left as f32,
            ins.top as f32,
            ins.right as f32,
            ins.bottom as f32,
        );
        [
            l + m,
            t + m * 0.6,
            vw - l - r - 2.0 * m,
            vh - t - b - m * 1.6,
        ]
    }

    fn plate_position(&self) -> PlatePosition {
        let ps = &self.snap.settings.plate;
        self.drag
            .or_else(|| ps.positions.get(&self.display_key).copied())
            .unwrap_or(ps.default_position)
    }
}

/// The wall colour for a painting shown in full.
fn wall_color(core: &core::Core, r: &Request) -> Color {
    let w = &r.wallpaper;
    let configured = hex_or(&w.matte_color, rgb(0x1d1c1a));
    if r.layout.kind == LayoutKind::Contain && w.matte == Matte::Average {
        return core
            .colors_for(&r.painting.id)
            .map(|c| hex_or(&c.wall, configured))
            .unwrap_or(configured);
    }
    configured
}

fn load(core: &core::Core, r: &Request) -> (Color, Result<images::Rgba, String>) {
    let result = core
        .source_for(&r.painting.id)
        .ok_or_else(|| "unknown painting".to_string())
        .and_then(|src| images::render_pixels(&src, r.target.0, r.target.1, &core.paths.cache));
    (wall_color(core, r), result)
}

/// Change the snapshot in place and hang the painting again (headless scenes).
fn restyle(s: &mut WallUi, f: impl FnOnce(&mut Snapshot)) {
    let mut snap = (*s.snap).clone();
    f(&mut snap);
    s.snap = Arc::new(snap);
    s.sync();
    s.refresh_plate();
}

fn on_wall(s: &mut WallUi, frame: FrameStyle) {
    restyle(s, |n| {
        n.settings.wallpaper.fit = crate::settings::FitMode::Fit;
        n.settings.wallpaper.frame = frame;
    });
}

fn plate_style(s: &mut WallUi, id: &str) {
    restyle(s, |n| n.settings.plate.style = id.into());
}

/// Named states for `galeria ui --window wall --scene <name>`.
pub const SCENES: &[crate::ui::Scene<WallUi>] = &[
    ("plastic", |s| plate_style(s, "plastic")),
    ("glass", |s| plate_style(s, "glass")),
    ("smoked", |s| plate_style(s, "smoked")),
    ("brass", |s| plate_style(s, "brass")),
    ("paper", |s| plate_style(s, "paper")),
    ("acrylic", |s| plate_style(s, "acrylic")),
    ("minimal", |s| plate_style(s, "minimal")),
    ("fill", |s| {
        restyle(s, |n| {
            n.settings.wallpaper.fit = crate::settings::FitMode::Fill
        })
    }),
    ("pan", |s| {
        restyle(s, |n| {
            n.settings.wallpaper.fit = crate::settings::FitMode::Pan
        })
    }),
    ("wall", |s| on_wall(s, FrameStyle::None)),
    ("gilded", |s| on_wall(s, FrameStyle::Gilded)),
    ("walnut", |s| on_wall(s, FrameStyle::Walnut)),
    ("black", |s| on_wall(s, FrameStyle::Black)),
    ("white", |s| on_wall(s, FrameStyle::White)),
    ("blurred", |s| {
        restyle(s, |n| {
            n.settings.wallpaper.fit = crate::settings::FitMode::Fit;
            n.settings.wallpaper.matte = Matte::Blur;
        })
    }),
    ("qr", |s| {
        restyle(s, |n| {
            n.settings.plate.fields.qr = true;
            n.settings.plate.fields.license = true;
            n.settings.plate.fields.accession = true;
        })
    }),
    ("hidden", |s| {
        restyle(s, |n| n.settings.plate.visible = false)
    }),
    ("arrange", |s| restyle(s, |n| n.arrange = true)),
];

pub fn poll(s: &mut WallUi, clock: f64, tx: &mut Tx) -> bool {
    s.clock = clock;
    let mut changed = false;
    if s.leaving.as_ref().is_some_and(|(_, at)| clock >= *at) {
        s.leaving = None;
        changed = true;
    }
    if s.sleep_at.is_some_and(|at| clock >= at) {
        s.sleep_at = None;
        if !s.snap.arrange && !s.plate_asleep {
            s.plate_asleep = true;
            changed = true;
        }
    }
    s.schedule(tx);
    changed
}

pub fn on_key(s: &mut WallUi, ev: &KeyEvent, _: &mut Tx) -> bool {
    if s.snap.arrange && ev.key == Key::Escape {
        s.cx.run(|app| core::set_arrange(app, false));
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn css_angle(degrees: f32) -> f32 {
    degrees * PI / 180.0
}

fn outer(y: f32, blur: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y,
        blur,
        spread: 0.0,
        color,
        inset: false,
    }
}

fn inner(blur: f32, spread: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur,
        spread,
        color,
        inset: true,
    }
}

/// The moulding around a painting on the wall. `vmin` is a hundredth of the screen's short side.
fn frame(style: FrameStyle, shadow: bool, vmin: f32) -> El {
    let black = Color::BLACK;
    let e = el();
    match style {
        FrameStyle::None => {
            if shadow {
                e.box_shadow(outer(0.4 * vmin, 0.8 * vmin, black.alpha(0.35)))
                    .box_shadow(outer(2.5 * vmin, 6.0 * vmin, black.alpha(0.45)))
            } else {
                e
            }
        }
        FrameStyle::Black => e
            .linear_gradient(
                css_angle(145.0),
                [
                    (0.0, rgb(0x2b2b2b)),
                    (0.45, rgb(0x0a0a0a)),
                    (1.0, rgb(0x1c1c1c)),
                ],
            )
            .border(1.0, Color::WHITE.alpha(0.08))
            .box_shadow(outer(0.4 * vmin, 0.8 * vmin, black.alpha(0.35)))
            .box_shadow(outer(2.5 * vmin, 6.0 * vmin, black.alpha(0.45))),
        FrameStyle::White => e
            .linear_gradient(
                css_angle(145.0),
                [
                    (0.0, rgb(0xfbfaf6)),
                    (0.55, rgb(0xe6e3dc)),
                    (1.0, rgb(0xf3f1ec)),
                ],
            )
            .border(1.0, black.alpha(0.08))
            .box_shadow(outer(0.4 * vmin, 0.8 * vmin, black.alpha(0.3)))
            .box_shadow(outer(2.5 * vmin, 6.0 * vmin, black.alpha(0.4))),
        FrameStyle::Gilded => e
            .linear_gradient(
                css_angle(135.0),
                [
                    (0.0, rgb(0xf6e2a1)),
                    (0.18, rgb(0xc89b3c)),
                    (0.36, rgb(0x8a6320)),
                    (0.52, rgb(0xe3bf66)),
                    (0.7, rgb(0x9b7027)),
                    (0.86, rgb(0xf0d58a)),
                    (1.0, rgb(0x7a531a)),
                ],
            )
            .box_shadow(inner(0.0, 0.25 * vmin, rgba(0xffebaa, 0.5)))
            .box_shadow(inner(0.0, 0.7 * vmin, rgba(0x5a3c0a, 0.35)))
            .box_shadow(inner(1.4 * vmin, 0.0, rgba(0x281900, 0.6)))
            .box_shadow(outer(0.5 * vmin, 1.0 * vmin, black.alpha(0.45)))
            .box_shadow(outer(3.0 * vmin, 7.0 * vmin, black.alpha(0.5))),
        FrameStyle::Walnut => e
            .linear_gradient(
                css_angle(135.0),
                [
                    (0.0, rgb(0x5b3a22)),
                    (0.4, rgb(0x3a2312)),
                    (0.6, rgb(0x6a4528)),
                    (1.0, rgb(0x2e1b0d)),
                ],
            )
            .box_shadow(inner(0.0, 0.2 * vmin, rgba(0xffdcb4, 0.12)))
            .box_shadow(inner(1.2 * vmin, 0.0, black.alpha(0.55)))
            .box_shadow(outer(0.5 * vmin, 1.0 * vmin, black.alpha(0.45)))
            .box_shadow(outer(3.0 * vmin, 7.0 * vmin, black.alpha(0.5))),
    }
}

/// One painting as it hangs, filling the window.
fn layer(l: &Layer, vw: f32, vh: f32) -> El {
    let Layout {
        kind,
        w,
        h,
        x,
        y,
        frame: f,
        ..
    } = l.layout;
    let (w, h, x, y, f) = (w as f32, h as f32, x as f32, y as f32, f as f32);
    let vmin = vw.min(vh) / 100.0;
    let fade = Spring::new(l.fade.max(0.01), 0.0);
    let mut e = el()
        .key(format!("layer-{}", l.generation))
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .transition(Transition::opacity().springs(SpringPair::new(fade, Spring::NONE)));
    let picture = el()
        .key("painting")
        .role("img")
        .label(l.painting.title.clone())
        .size(w, h)
        .pixels(&l.pixels);

    if kind != LayoutKind::Contain {
        // Filling the screen; a pan slides it along its long side.
        let mut art = picture.absolute(x, y);
        if l.pan.is_some() {
            art = art.live_offset(&l.offset);
        }
        return e.child(art);
    }

    // On the wall.
    let ws = &l.wallpaper;
    let mut wall = el()
        .key("wall")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(l.wall)
        .a11y_hidden();
    if let Some(backdrop) = &l.backdrop {
        // The painting itself, out of focus and dimmed, a little larger than the screen
        // so its soft edge stays off screen.
        wall = wall
            .child(
                el().absolute(-0.08 * vw, -0.08 * vh)
                    .size(1.16 * vw, 1.16 * vh)
                    .blur(28.0)
                    .child(
                        el().w_full()
                            .h_full()
                            .pixels_fit(backdrop, rune::SurfaceFit::Fill),
                    ),
            )
            .child(
                el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .fill(Color::BLACK.alpha(0.5)),
            );
    }
    e = e.child(wall);
    if ws.spotlight && ws.matte != Matte::Blur {
        // A gallery spot from above, and the room falling off into the corners.
        e = e
            .child(
                el().key("spot")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .a11y_hidden()
                    .radial_gradient(
                        [0.5, 0.38],
                        [0.7 * vw, 0.6 * vh],
                        [(0.0, rgba(0xfff4e1, 0.11)), (0.7, rgba(0xfff4e1, 0.0))],
                    ),
            )
            .child(
                el().key("vignette")
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .a11y_hidden()
                    .radial_gradient(
                        [0.5, 0.5],
                        [1.2 * vw, 1.0 * vh],
                        [
                            (0.0, Color::BLACK.alpha(0.0)),
                            (0.55, Color::BLACK.alpha(0.0)),
                            (1.0, Color::BLACK.alpha(0.38)),
                        ],
                    ),
            );
    }
    let mut canvas = picture.absolute(f, f);
    if ws.frame != FrameStyle::None {
        // The painting sits a little behind the frame's inner lip.
        canvas = canvas.box_shadow(outer(0.0, 0.6 * vmin, Color::BLACK.alpha(0.6)));
    }
    let mut hung = frame(ws.frame, ws.shadow, vmin)
        .key("frame")
        .absolute(x - f, y - f)
        .size(w + 2.0 * f, h + 2.0 * f);
    // The moulding itself, shaded: leaf gold, or lacquer.
    let moulding = match ws.frame {
        FrameStyle::None => None,
        FrameStyle::Gilded => Some(Material::Gilt { border: f }),
        FrameStyle::Black => Some(Material::Lacquer {
            border: f,
            finish: Finish::Black,
        }),
        FrameStyle::White => Some(Material::Lacquer {
            border: f,
            finish: Finish::White,
        }),
        FrameStyle::Walnut => Some(Material::Lacquer {
            border: f,
            finish: Finish::Walnut,
        }),
    };
    if let Some(m) = moulding.filter(|_| fx::enabled() && f > 0.0) {
        hung = hung.child(fx::slot(m, 1.0).pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0)));
    }
    e.child(hung.child(canvas))
}

fn pill(key: &str, label: &str, primary: bool, f: impl Fn(&mut WallUi, &mut Tx) + 'static) -> El {
    let white = Color::WHITE;
    let (fill, ink, weight) = if primary {
        (white, rgb(0x111111), 600.0)
    } else {
        (white.alpha(0.14), white, 400.0)
    };
    row()
        .key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .h(30.0)
        .px(16.0)
        .center()
        .radius(15.0)
        .fill(fill)
        .hover(if primary {
            Color::BLACK.alpha(0.08)
        } else {
            white.alpha(0.08)
        })
        .cursor(Cursor::Pointer)
        .on_press_tx(f)
        .child(
            text(label)
                .font(TextStyle::new(13.0, weight))
                .color(ink)
                .a11y_hidden(),
        )
}

/// The panel shown while the plate is being arranged.
fn arrange_hud(s: &WallUi) -> El {
    let white = Color::WHITE;
    let visible = s.snap.settings.plate.visible;
    let key = s.display_key.clone();
    col()
        .key("arrange")
        .role("dialog")
        .label("Arrange the info plate")
        .min_w(360.0)
        .px(26.0)
        .pt(22.0)
        .pb(18.0)
        .radius(22.0)
        .items_center()
        .backdrop_blur(30.0)
        .fill(rgba(0x161618, 0.55))
        .border(1.0, white.alpha(0.1))
        .shadow(20.0, 60.0, Color::BLACK.alpha(0.4))
        .transition(Transition::scale(0.96))
        .child(
            text("Arrange the info plate")
                .font(TextStyle::new(16.0, 600.0))
                .color(white),
        )
        .child(
            text("Drag it anywhere. It snaps to edges and centre lines.")
                .font(TextStyle::new(13.0, 400.0))
                .color(white.alpha(0.7))
                .mt(4.0),
        )
        .child(
            row()
                .mt(16.0)
                .gap(8.0)
                .child(pill(
                    "toggle",
                    if visible { "Hide plate" } else { "Show plate" },
                    false,
                    move |s, _| {
                        s.cx.patch(serde_json::json!({ "plate": { "visible": !visible } }))
                    },
                ))
                .child(pill("reset", "Reset", false, move |s, _| {
                    let d = s.snap.settings.plate.default_position;
                    let key = key.clone();
                    s.drag = Some(d);
                    s.cx.run(move |app| {
                        core::set_plate_position(app, "wallpaper", Some(key), d.x, d.y)
                    });
                }))
                .child(pill("done", "Done", true, |s, _| {
                    s.cx.run(|app| core::set_arrange(app, false))
                })),
        )
}

pub fn view(s: &WallUi, cx: &mut Cx<WallUi>) -> El {
    // This window is always open, so it is the one that notices Light/Dark Mode switching.
    s.cx.report_dark(cx.system_dark());
    let mut root = el().key("desktop").w_full().h_full().fill(rgb(BACKGROUND));
    let Some(d) = s.display() else { return root };
    let (vw, vh) = (d.info.width as f32, d.info.height as f32);
    if let Some((old, _)) = &s.leaving {
        root = root.child(layer(old, vw, vh));
    }
    if let Some(l) = &s.shown {
        root = root.child(layer(l, vw, vh));
    }

    let arranging = s.snap.arrange;
    if arranging {
        root = root.child(
            el().key("dim")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .fill(Color::BLACK.alpha(0.18))
                .transition(Transition::opacity())
                .a11y_hidden(),
        );
    }

    // The plate describes what is actually on screen: while the next painting is
    // still decoding, the current one keeps its label.
    let ps = &s.snap.settings.plate;
    let area = s.area();
    let pos = s.plate_position();
    if let Some(l) = &s.shown {
        let on = ps.visible || arranging;
        let style = plate::resolve(&ps.style, &s.snap.custom_styles);
        let mut label = plate::render(&l.painting, ps, &style, &s.cx.art, 1.0);
        if arranging {
            let display = s.display_key.clone();
            label = el()
                .key("handle")
                .padding(8.0)
                .radius(6.0)
                .border(1.5, Color::WHITE.alpha(0.85))
                .border_dash(6.0, 4.0)
                .cursor(Cursor::Grab)
                .on_drag(move |s: &mut WallUi, g: &mut Drag, tx| {
                    let area = s.area();
                    // The dashed outline is 8 points outside the plate itself.
                    let [px, py] = g.window_point();
                    let at = plate::position_at(
                        px - g.start_x + 8.0,
                        py - g.start_y + 8.0,
                        [g.w - 16.0, g.h - 16.0],
                        area,
                    );
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
                                let key = display.clone();
                                s.cx.run(move |app| {
                                    core::set_plate_position(
                                        app,
                                        "wallpaper",
                                        Some(key),
                                        at.x,
                                        at.y,
                                    )
                                });
                            }
                        }
                    }
                })
                .child(label);
        }
        let opacity = if !on || (s.plate_asleep && !arranging) {
            0.0
        } else if arranging && !ps.visible {
            0.45
        } else {
            1.0
        };
        // The outline adds 8 points on every side; keep the plate itself where it was.
        let area = if arranging {
            [area[0] - 8.0, area[1] - 8.0, area[2] + 16.0, area[3] + 16.0]
        } else {
            area
        };
        let mut placed = plate::place(label, pos, area)
            .key("plate-layer")
            .opacity(opacity)
            .opacity_motion(rune::Curve::ease_in_out(if arranging { 0.2 } else { 0.9 }));
        if opacity == 0.0 {
            placed = placed.inert();
        }
        root = root.child(placed);
    }

    if arranging {
        // Guides along the centre lines while the plate is snapped to them.
        let gold = rgba(0xffd666, 0.9);
        if s.dragging && s.drag.is_some_and(|p| p.x == 0.5) {
            root = root.child(
                el().key("guide-x")
                    .absolute(vw / 2.0, 0.0)
                    .size(1.0, vh)
                    .fill(gold)
                    .a11y_hidden(),
            );
        }
        if s.dragging && s.drag.is_some_and(|p| p.y == 0.5) {
            root = root.child(
                el().key("guide-y")
                    .absolute(0.0, vh / 2.0)
                    .size(vw, 1.0)
                    .fill(gold)
                    .a11y_hidden(),
            );
        }
        root = root.child(
            el().key("arrange-layer")
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(arrange_hud(s)),
        );
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A core with one display showing nothing: enough to drive the plate logic.
    fn ui(dir: &std::path::Path) -> WallUi {
        let app = crate::host::AppHandle::new(core::Core::new(core::Paths::in_dir(dir).unwrap()));
        let snap = Arc::new(app.core().snapshot());
        WallUi::new(Ctx::new(app), snap, "Test".into(), None)
    }

    #[test]
    fn nothing_to_show_draws_an_empty_desktop() {
        let dir = std::env::temp_dir().join(format!("galeria-wall-{}", fastrand::u64(..)));
        let s = ui(&dir);
        assert!(s.shown.is_none() && s.loading.is_none());
        let mut rt = rune::Runtime::new(s, view, (800.0, 500.0), rune::Env::new(true, 2.0));
        rt.frame(0.0);
        assert!(rt.find("desktop").is_some());
        assert!(rt.find("plate").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_plate_fades_out_after_its_timer_and_returns_with_a_new_painting() {
        let dir = std::env::temp_dir().join(format!("galeria-wall-{}", fastrand::u64(..)));
        let mut s = ui(&dir);
        let mut snap = (*s.snap).clone();
        snap.settings.plate.auto_hide_seconds = 10;
        s.snap = Arc::new(snap);
        s.plate_for = Some("a".into());
        s.plate_was_on = false;
        s.clock = 5.0;
        s.refresh_plate();
        assert_eq!(s.sleep_at, Some(15.0), "switched on: timer starts");
        assert!(!s.plate_asleep);
        s.plate_asleep = true;
        s.sleep_at = None;
        s.clock = 40.0;
        s.refresh_plate();
        assert!(s.plate_asleep, "nothing new: stays faded out");
        // Arrange mode always shows it.
        let mut snap = (*s.snap).clone();
        snap.arrange = true;
        s.snap = Arc::new(snap);
        s.refresh_plate();
        assert!(!s.plate_asleep && s.sleep_at.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
