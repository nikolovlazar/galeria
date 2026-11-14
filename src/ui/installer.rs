//! The picture behind the installer window (the disk image people download):
//! a gallery wall with two carved gilt frames, one for Galeria and one for the
//! Applications folder, and an arrow from the first to the second. Finder draws
//! the two icons and their names on top; this draws everything else.
//!
//! It is only ever rendered offscreen, by `packaging/macos/dmg.sh`:
//! `galeria ui --window installer --snap background.png`.

use crate::ui::fx::{self, Material};
use crate::ui::theme::{rgb, rgba};
use rune::{col, el, icon, row, text, Align, Color, Cx, El, Family, Icon, Shadow, TextStyle};
use std::f32::consts::PI;

/// The window's size in points, and where Finder puts the two icons (their centres).
pub const SIZE: (f32, f32) = (720.0, 460.0);
/// The picture is much larger than the window and composed in its top-left corner.
/// Finder does not always open the window at the size it is asked to (seen on
/// macOS 26), and where the picture ends it paints white: this way a larger window
/// shows more wall instead.
pub const CANVAS: (f32, f32) = (1600.0, 1000.0);
pub const ICON: f32 = 112.0;
pub const APP_AT: (f32, f32) = (190.0, 200.0);
pub const APPLICATIONS_AT: (f32, f32) = (530.0, 200.0);

/// How far the moulding reaches around the opening.
const RAIL: f32 = 34.0;
/// The opening around the icon: a little air above and beside it.
const OPEN_X: f32 = ICON / 2.0 + 8.0;
const OPEN_TOP: f32 = ICON / 2.0 + 8.0;
/// Finder writes the name just under the icon; the opening stops above it, so the
/// name lands on the frame's nameplate.
const OPEN_BOTTOM: f32 = ICON / 2.0 + 3.0;
const RAIL_Y: f32 = 30.0;

static ARROW: Icon = Icon {
    name: "galeria-installer-arrow",
    view: 48.0,
    body: r#"<path d="M3 24 C 12 20.5, 26 20.5, 41 24" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" fill="none"/><path d="M33.5 17.2 L43.6 24.1 L33.2 30.6" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" fill="none"/>"#,
};

pub struct InstallerUi;

fn pin_all(e: El) -> El {
    e.pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .a11y_hidden()
}

fn shadow(y: f32, blur: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y,
        blur,
        spread: 0.0,
        color,
        inset: false,
    }
}

fn inset(blur: f32, spread: f32, color: Color) -> Shadow {
    Shadow {
        x: 0.0,
        y: 0.0,
        blur,
        spread,
        color,
        inset: true,
    }
}

/// A carved bead: a small gilt dome catching the light from above.
fn bead(d: f32) -> El {
    el().size(d, d)
        .radius(d / 2.0)
        .radial_gradient(
            [0.36, 0.3],
            [d * 0.75, d * 0.75],
            [
                (0.0, rgb(0xfff3c4)),
                (0.35, rgb(0xe2b552)),
                (0.8, rgb(0x8a6220)),
                (1.0, rgb(0x4d340d)),
            ],
        )
        .box_shadow(shadow(0.6, 1.2, Color::BLACK.alpha(0.55)))
}

/// A run of beads `len` long, laid along a rail.
fn beads(len: f32, d: f32, vertical: bool) -> El {
    let n = (len / (d + 1.6)).floor().max(1.0) as usize;
    let line = if vertical { col() } else { row() };
    let line = if vertical {
        line.h(len).w(d)
    } else {
        line.w(len).h(d)
    };
    line.justify_between()
        .children((0..n).map(|_| bead(d)))
        .a11y_hidden()
}

/// A carved leaf: an oval boss with a darker heart, repeated along the inner edge.
fn leaf(w: f32, h: f32) -> El {
    el().size(w, h)
        .radius(w.min(h) / 2.0)
        .radial_gradient(
            [0.4, 0.28],
            [w * 0.8, h * 0.8],
            [
                (0.0, rgb(0xfbe6a6)),
                (0.45, rgb(0xcf9f3f)),
                (1.0, rgb(0x5d400f)),
            ],
        )
        .box_shadow(inset(1.5, 0.0, rgba(0x3a2606, 0.7)))
        .box_shadow(shadow(0.8, 1.4, Color::BLACK.alpha(0.5)))
}

fn leaves(len: f32, vertical: bool) -> El {
    let (w, h) = if vertical { (7.0, 10.0) } else { (10.0, 7.0) };
    let step = if vertical { h } else { w } + 1.5;
    let n = (len / step).floor().max(1.0) as usize;
    let line = if vertical {
        col().h(len).w(w)
    } else {
        row().w(len).h(h)
    };
    line.justify_between()
        .children((0..n).map(|_| leaf(w, h)))
        .a11y_hidden()
}

/// A corner boss: a rosette where two rails meet.
fn rosette(d: f32) -> El {
    el().size(d, d)
        .radius(d / 2.0)
        .radial_gradient(
            [0.4, 0.32],
            [d * 0.7, d * 0.7],
            [
                (0.0, rgb(0xfff0bd)),
                (0.3, rgb(0xe0b04a)),
                (0.62, rgb(0x8f6521)),
                (0.66, rgb(0xf0cf7a)),
                (1.0, rgb(0x6b4a14)),
            ],
        )
        .box_shadow(shadow(1.2, 2.5, Color::BLACK.alpha(0.6)))
        .child(
            el().size(d * 0.36, d * 0.36)
                .radius(d)
                .absolute(d * 0.32, d * 0.32)
                .radial_gradient(
                    [0.4, 0.3],
                    [d * 0.3, d * 0.3],
                    [(0.0, rgb(0xfff6d6)), (1.0, rgb(0x9a6d22))],
                ),
        )
}

/// A carved gilt frame around an icon whose centre is at `at`.
fn frame(at: (f32, f32)) -> El {
    let (w, h) = (2.0 * (OPEN_X + RAIL), OPEN_TOP + OPEN_BOTTOM + 2.0 * RAIL);
    let (x, y) = (at.0 - OPEN_X - RAIL, at.1 - OPEN_TOP - RAIL);
    let (ow, oh) = (2.0 * OPEN_X, OPEN_TOP + OPEN_BOTTOM);

    // The moulding: gold leaf over a rounded profile. Plain gradients under the
    // shader, for when materials are off.
    let mut f = el()
        .key("frame")
        .absolute(x, y)
        .size(w, h)
        .radius(3.0)
        .linear_gradient(
            135.0 * PI / 180.0,
            [
                (0.0, rgb(0xf6e2a1)),
                (0.2, rgb(0xc89b3c)),
                (0.4, rgb(0x8a6320)),
                (0.55, rgb(0xe3bf66)),
                (0.75, rgb(0x9b7027)),
                (1.0, rgb(0x7a531a)),
            ],
        )
        .box_shadow(shadow(3.0, 5.0, Color::BLACK.alpha(0.5)))
        .box_shadow(shadow(16.0, 30.0, Color::BLACK.alpha(0.6)));
    if fx::enabled() {
        f = f.child(pin_all(fx::slot(Material::Gilt { border: 0.0 }, 1.0)));
    }
    // A cove cut into the moulding, where the carving sits in shadow.
    let cove = 7.0;
    f = f.child(
        el().absolute(cove, cove)
            .size(w - 2.0 * cove, h - 2.0 * cove)
            .radius(2.0)
            .fill(rgba(0x3c2706, 0.5))
            .box_shadow(inset(5.0, 0.0, rgba(0x1e1202, 0.85)))
            .border(1.0, rgba(0xffe7a3, 0.35))
            .a11y_hidden(),
    );
    // Beads along the outer edge of the cove.
    let (bd, edge) = (4.6, cove + 2.2);
    let (bw, bh) = (w - 2.0 * edge - 16.0, h - 2.0 * edge - 16.0);
    f = f
        .child(beads(bw, bd, false).absolute(edge + 8.0, edge))
        .child(beads(bw, bd, false).absolute(edge + 8.0, h - edge - bd))
        .child(beads(bh, bd, true).absolute(edge, edge + 8.0))
        .child(beads(bh, bd, true).absolute(w - edge - bd, edge + 8.0));
    // Leaves along the inner edge, round the opening.
    let lip = RAIL - 12.5;
    f = f
        .child(leaves(ow - 6.0, false).absolute(RAIL + 3.0, lip))
        .child(leaves(oh - 6.0, true).absolute(lip, RAIL + 3.0))
        .child(leaves(oh - 6.0, true).absolute(w - lip - 7.0, RAIL + 3.0));
    // Rosettes at the corners.
    let r = 17.0;
    for (cx, cy) in [
        (cove - 1.0, cove - 1.0),
        (w - cove - r + 1.0, cove - 1.0),
        (cove - 1.0, h - cove - r + 1.0),
        (w - cove - r + 1.0, h - cove - r + 1.0),
    ] {
        f = f.child(rosette(r).absolute(cx, cy).a11y_hidden());
    }
    // The inner lip: a bright fillet, then the opening, lined in dark velvet.
    f = f.child(
        el().key("opening")
            .absolute(RAIL - 3.0, RAIL - 3.0)
            .size(ow + 6.0, oh + 6.0)
            .radius(2.0)
            .linear_gradient(
                160.0 * PI / 180.0,
                [
                    (0.0, rgb(0xfff0c0)),
                    (0.5, rgb(0xc7963a)),
                    (1.0, rgb(0x8c651f)),
                ],
            )
            .box_shadow(shadow(0.0, 2.0, Color::BLACK.alpha(0.6)))
            .a11y_hidden()
            .child(
                el().absolute(3.0, 3.0)
                    .size(ow, oh)
                    .radius(1.0)
                    .radial_gradient(
                        [0.5, 0.38],
                        [ow * 0.75, oh * 0.8],
                        [(0.0, rgb(0x2f4a3d)), (1.0, rgb(0x12211a))],
                    )
                    .box_shadow(inset(9.0, 1.0, Color::BLACK.alpha(0.75))),
            ),
    );
    // The nameplate on the bottom rail, where Finder writes the name.
    let (pw, ph) = (ow - 14.0, 19.0);
    let mut plate = el()
        .key("nameplate")
        .absolute((w - pw) / 2.0, RAIL + oh + (RAIL - ph) / 2.0)
        .size(pw, ph)
        .radius(2.0)
        .linear_gradient(
            PI,
            [
                (0.0, rgb(0xcfa650)),
                (0.5, rgb(0xb48a3a)),
                (1.0, rgb(0x94702a)),
            ],
        )
        .border(0.5, rgba(0x4a3208, 0.8))
        .box_shadow(shadow(1.0, 2.0, Color::BLACK.alpha(0.6)))
        .a11y_hidden();
    for px in [3.5, pw - 6.5] {
        plate = plate.child(
            el().absolute(px, ph / 2.0 - 1.5)
                .size(3.0, 3.0)
                .radius(1.5)
                .fill(rgb(0x7a5a1c))
                .box_shadow(shadow(0.4, 0.4, rgba(0xfff3c4, 0.7))),
        );
    }
    f.child(plate)
}

/// A picture light's glow: warm light falling down the wall and over the frame.
fn light(at: (f32, f32)) -> El {
    let top = at.1 - OPEN_TOP - RAIL;
    // Brightest just under the lamp, fading out well inside its own box.
    let (w, h) = (470.0, 560.0);
    el().absolute(at.0 - w / 2.0, top + 40.0 - h / 2.0)
        .size(w, h)
        .a11y_hidden()
        .radial_gradient(
            [0.5, 0.5],
            [w * 0.5, h * 0.5],
            [
                (0.0, rgba(0xffe7b8, 0.46)),
                (0.35, rgba(0xffdca0, 0.24)),
                (0.7, rgba(0xffd596, 0.07)),
                (1.0, rgba(0xffd596, 0.0)),
            ],
        )
}

/// The lamp itself: a brass tube on an arm over the top of the frame.
fn lamp(at: (f32, f32)) -> El {
    let top = at.1 - OPEN_TOP - RAIL;
    let (w, h) = (96.0, 9.0);
    let brass = [
        (0.0, rgb(0xfbe9b0)),
        (0.45, rgb(0xc99a3e)),
        (1.0, rgb(0x6b4a14)),
    ];
    el().a11y_hidden()
        // The arm, from the frame's top up to the tube.
        .child(
            el().absolute(at.0 - 2.0, top - 15.0)
                .size(4.0, 17.0)
                .linear_gradient(PI / 2.0, brass),
        )
        // The light under the tube.
        .child(
            el().absolute(at.0 - w / 2.0 + 4.0, top - 15.0)
                .size(w - 8.0, 12.0)
                .radius(6.0)
                .radial_gradient(
                    [0.5, 0.0],
                    [w * 0.5, 12.0],
                    [(0.0, rgba(0xfff6dc, 0.95)), (1.0, rgba(0xfff0c8, 0.0))],
                ),
        )
        .child(
            el().absolute(at.0 - w / 2.0, top - 22.0)
                .size(w, h)
                .radius(h / 2.0)
                .linear_gradient(PI, brass)
                .box_shadow(shadow(2.0, 4.0, Color::BLACK.alpha(0.5))),
        )
}

/// The cord a frame hangs by, from the picture rail down to its top.
fn cords(at: (f32, f32)) -> El {
    let top = at.1 - OPEN_TOP - RAIL;
    let mut e = el().a11y_hidden();
    for dx in [-OPEN_X * 0.55, OPEN_X * 0.55] {
        e = e.child(
            el().absolute(at.0 + dx - 0.6, RAIL_Y + 3.0)
                .size(1.2, top - RAIL_Y - 2.0)
                .fill(rgba(0xd9b560, 0.55))
                .box_shadow(shadow(0.0, 1.5, Color::BLACK.alpha(0.5))),
        );
    }
    e
}

pub fn view(_: &InstallerUi, _: &mut Cx<InstallerUi>) -> El {
    let (w, h) = CANVAS;
    let gold = rgb(0xe6c170);
    // The wall: deep green, as in an old picture gallery, darker toward the corners.
    let mut wall = el().key("wall").size(w, h).fill(rgb(0x183026));
    // The picture rail, and the wainscot below.
    // High enough to stay in view when Finder shows its status bar.
    let dado = SIZE.1 - 88.0;
    wall = wall
        .child(
            el().absolute(0.0, RAIL_Y)
                .size(w, 5.0)
                .a11y_hidden()
                .linear_gradient(
                    PI,
                    [
                        (0.0, rgb(0xf0d58a)),
                        (0.5, rgb(0xb98f3d)),
                        (1.0, rgb(0x6b4a14)),
                    ],
                )
                .box_shadow(shadow(3.0, 6.0, Color::BLACK.alpha(0.5))),
        )
        .child(
            el().absolute(0.0, dado)
                .size(w, h - dado)
                .a11y_hidden()
                .linear_gradient(PI, [(0.0, rgb(0x2a1c12)), (1.0, rgb(0x150d08))])
                .child(
                    el().absolute(0.0, 0.0)
                        .size(w, 4.0)
                        .linear_gradient(PI, [(0.0, rgb(0x6a4a2c)), (1.0, rgb(0x2a1c12))])
                        .box_shadow(shadow(-2.0, 6.0, Color::BLACK.alpha(0.45))),
                ),
        );
    for at in [APP_AT, APPLICATIONS_AT] {
        wall = wall.child(light(at));
    }
    for at in [APP_AT, APPLICATIONS_AT] {
        wall = wall.child(cords(at)).child(frame(at)).child(lamp(at));
    }
    // The arrow, and what to do.
    let (ax, aw) = (
        APP_AT.0 + OPEN_X + RAIL,
        APPLICATIONS_AT.0 - APP_AT.0 - 2.0 * (OPEN_X + RAIL),
    );
    let serif = Family::Named("Iowan Old Style, Palatino, Baskerville, Georgia");
    wall = wall.child(
        col()
            .key("arrow")
            .absolute(ax, APP_AT.1 - 52.0)
            .size(aw, 104.0)
            .center()
            .gap(0.0)
            .child(
                // The arrow over its own shadow on the wall.
                el().size(76.0, 76.0)
                    .child(
                        icon(&ARROW, 76.0)
                            .color(Color::BLACK.alpha(0.55))
                            .absolute(0.0, 2.0),
                    )
                    .child(icon(&ARROW, 76.0).color(gold).absolute(0.0, 0.0)),
            )
            .child(
                text("Drag to install")
                    .font(TextStyle::new(14.0, 500.0).family(serif).italic().lh(18.0))
                    .color(rgba(0xf3dfae, 0.92))
                    .align(Align::Center)
                    .mt(-16.0),
            ),
    );
    // A caption on the wainscot, like a gallery's wall text.
    wall.child(
        el().absolute(0.0, dado + 20.0).w(SIZE.0).child(
            text("GALERIA  ·  A MUSEUM ON YOUR DESKTOP")
                .font(
                    TextStyle::new(10.0, 600.0)
                        .family(serif)
                        .tracking(2.2)
                        .lh(14.0),
                )
                .color(rgba(0xe6c170, 0.6))
                .align(Align::Center)
                .w_full(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Finder places the icons from packaging/macos/dmg_settings.py; the frames are drawn here.
    #[test]
    fn the_installer_settings_put_the_icons_in_the_frames() {
        let settings = include_str!("../../packaging/macos/dmg_settings.py");
        for line in [
            format!("WIDTH, HEIGHT = {}, {}", SIZE.0, SIZE.1),
            format!("ICON = {ICON}"),
            format!("APP_AT = ({}, {})", APP_AT.0, APP_AT.1),
            format!(
                "APPLICATIONS_AT = ({}, {})",
                APPLICATIONS_AT.0, APPLICATIONS_AT.1
            ),
        ] {
            assert!(
                settings.contains(&line),
                "dmg_settings.py should say: {line}"
            );
        }
    }
}
