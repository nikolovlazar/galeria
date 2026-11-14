//! How a painting meets a screen: fill it, hang on the wall, or fill and pan.
//! Pure geometry and timing; the wallpaper window draws from it.

use crate::core::{layout_kind, LayoutKind};
use crate::settings::{FrameStyle, WallpaperSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub kind: LayoutKind,
    /// Painting size and position in points (for pan: its position at the start edge).
    pub w: f64,
    pub h: f64,
    pub x: f64,
    pub y: f64,
    /// Pan axis and distance in points.
    pub axis: Option<Axis>,
    pub travel: f64,
    /// Fraction of the long side that a cover fit would crop.
    pub crop: f64,
    /// Frame thickness in points (contain only).
    pub frame: f64,
}

/// Frame thickness as a fraction of the screen's short side.
fn frame_thickness(f: FrameStyle) -> f64 {
    match f {
        FrameStyle::None => 0.0,
        FrameStyle::Black | FrameStyle::White => 0.011,
        FrameStyle::Gilded => 0.03,
        FrameStyle::Walnut => 0.022,
    }
}

/// Place an `iw`×`ih` painting on a `vw`×`vh` screen. `top_inset` is the menu
/// bar: a painting hung on the wall is centred in what is left below it.
pub fn compute(
    iw: f64,
    ih: f64,
    vw: f64,
    vh: f64,
    w: &WallpaperSettings,
    top_inset: f64,
) -> Layout {
    if iw <= 0.0 || ih <= 0.0 || vw <= 0.0 || vh <= 0.0 {
        return Layout {
            kind: LayoutKind::Cover,
            w: vw.max(0.0),
            h: vh.max(0.0),
            x: 0.0,
            y: 0.0,
            axis: None,
            travel: 0.0,
            crop: 0.0,
            frame: 0.0,
        };
    }
    let cover = (vw / iw).max(vh / ih);
    let contain = (vw / iw).min(vh / ih);
    let crop = 1.0 - contain / cover;
    let kind = layout_kind(w, iw, ih, vw, vh);

    if kind == LayoutKind::Contain {
        let frame = (vw.min(vh) * frame_thickness(w.frame)).round();
        let area_h = vh - top_inset;
        let avail_w = vw * (1.0 - 2.0 * w.fit_margin) - 2.0 * frame;
        let avail_h = area_h * (1.0 - 2.0 * w.fit_margin) - 2.0 * frame;
        let s = (avail_w / iw).min(avail_h / ih).max(0.0);
        let (pw, ph) = (iw * s, ih * s);
        return Layout {
            kind,
            w: pw,
            h: ph,
            x: (vw - pw) / 2.0,
            y: top_inset + (area_h - ph) / 2.0,
            axis: None,
            travel: 0.0,
            crop,
            frame,
        };
    }

    let (pw, ph) = (iw * cover, ih * cover);
    let axis = if pw - vw > ph - vh { Axis::X } else { Axis::Y };
    if kind == LayoutKind::Pan {
        let travel = match axis {
            Axis::X => pw - vw,
            Axis::Y => ph - vh,
        };
        return Layout {
            kind,
            w: pw,
            h: ph,
            x: if axis == Axis::X {
                0.0
            } else {
                (vw - pw) / 2.0
            },
            y: if axis == Axis::Y {
                0.0
            } else {
                (vh - ph) / 2.0
            },
            axis: Some(axis),
            travel,
            crop,
            frame: 0.0,
        };
    }
    Layout {
        kind,
        w: pw,
        h: ph,
        x: (vw - pw) / 2.0,
        y: (vh - ph) / 2.0,
        axis: None,
        travel: 0.0,
        crop,
        frame: 0.0,
    }
}

/// CSS `ease-in-out` = cubic-bezier(0.42, 0, 0.58, 1).
fn ease_in_out(x: f64) -> f64 {
    let (x1, x2) = (0.42, 0.58);
    let bx = |s: f64| 3.0 * x1 * s * (1.0 - s).powi(2) + 3.0 * x2 * s * s * (1.0 - s) + s.powi(3);
    let by = |s: f64| 3.0 * s * s * (1.0 - s) + s.powi(3);
    let (mut lo, mut hi, mut s) = (0.0, 1.0, x);
    for _ in 0..24 {
        let v = bx(s) - x;
        if v.abs() < 1e-6 {
            break;
        }
        if v > 0.0 {
            hi = s;
        } else {
            lo = s;
        }
        s = (lo + hi) / 2.0;
    }
    by(s)
}

/// Slow edge-to-edge ping-pong pan: rest, glide, rest, glide back.
///
/// The timeline is a function of wall-clock time since the painting appeared,
/// so restarting Galeria doesn't send the painting back to the edge. The window
/// moves the painting one device pixel at a time and sleeps in between: a few
/// cheap frames per second, and none at all while resting or paused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pan {
    pub travel: f64,
    pub travel_secs: f64,
    pub pause_secs: f64,
}

impl Pan {
    pub fn new(layout: &Layout, speed: f64, pause_secs: f64) -> Option<Pan> {
        if layout.kind != LayoutKind::Pan || layout.axis.is_none() || layout.travel < 1.0 {
            return None;
        }
        Some(Pan {
            travel: layout.travel,
            travel_secs: layout.travel / speed.max(0.1),
            pause_secs: pause_secs.max(0.0),
        })
    }

    pub fn cycle_secs(&self) -> f64 {
        2.0 * (self.travel_secs + self.pause_secs)
    }

    /// Offset in points (≤ 0) along the pan axis, `t` seconds into the timeline.
    pub fn offset_at(&self, t: f64) -> f64 {
        let cycle = self.cycle_secs();
        let c = t.rem_euclid(cycle);
        let (p, m) = (self.pause_secs, self.travel_secs);
        let end = -self.travel;
        if c < p {
            0.0
        } else if c < p + m {
            end * ease_in_out((c - p) / m)
        } else if c < 2.0 * p + m {
            end
        } else {
            end * (1.0 - ease_in_out((c - 2.0 * p - m) / m))
        }
    }

    /// Fraction (0..1) through the cycle, for carrying progress across re-layouts.
    pub fn progress(&self, t: f64) -> f64 {
        let cycle = self.cycle_secs();
        t.rem_euclid(cycle) / cycle
    }

    /// Seconds until the painting should next move, at most one device pixel
    /// later (and no more often than 30 times a second).
    pub fn next_step(&self, t: f64, scale: f64) -> f64 {
        const MIN_STEP: f64 = 0.033;
        let probe = 0.2;
        let v = (self.offset_at(t + probe) - self.offset_at(t)).abs() / probe; // points per second
        if v > 0.0 {
            return (1.0 / (v * scale.max(0.5))).clamp(MIN_STEP, 1.0);
        }
        // Resting at an edge: wake up when the glide starts.
        let c = t.rem_euclid(self.cycle_secs());
        let (p, m) = (self.pause_secs, self.travel_secs);
        let rest_end = if c < p {
            p
        } else if c >= p + m && c < 2.0 * p + m {
            2.0 * p + m
        } else {
            c + 1.0
        };
        (rest_end - c).clamp(0.016, 1.0)
    }
}

/// Snap an offset to whole device pixels.
pub fn snap(v: f64, scale: f64) -> f64 {
    let s = scale.max(0.5);
    (v * s).round() / s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_painting_near_the_screens_shape_fills_it() {
        let w = WallpaperSettings::default();
        let l = compute(4096.0, 2642.0, 2560.0, 1707.0, &w, 25.0);
        assert_eq!(l.kind, LayoutKind::Cover);
        assert!(l.w >= 2560.0 - 1e-6 && l.h >= 1707.0 - 1e-6);
        // Centred: what is cropped is split between both sides.
        assert!(close(l.x, (2560.0 - l.w) / 2.0) && close(l.y, (1707.0 - l.h) / 2.0));
        assert!(l.axis.is_none() && l.frame == 0.0);
    }

    #[test]
    fn a_panorama_pans_along_its_long_side() {
        let w = WallpaperSettings::default();
        let l = compute(6209.0, 2100.0, 2560.0, 1707.0, &w, 25.0);
        assert_eq!(l.kind, LayoutKind::Pan);
        assert_eq!(l.axis, Some(Axis::X));
        assert!(close(l.h, 1707.0) && close(l.x, 0.0) && close(l.y, 0.0));
        assert!(close(l.travel, l.w - 2560.0) && l.travel > 2000.0);
        // A tall work pans down instead.
        let tall = compute(2000.0, 6000.0, 2560.0, 1707.0, &w, 0.0);
        assert_eq!(tall.axis, Some(Axis::Y));
        assert!(close(tall.w, 2560.0) && close(tall.travel, tall.h - 1707.0));
    }

    #[test]
    fn a_fitted_painting_hangs_below_the_menu_bar_inside_its_frame() {
        let w = WallpaperSettings {
            fit: crate::settings::FitMode::Fit,
            frame: FrameStyle::Gilded,
            ..Default::default()
        };
        let (vw, vh, top) = (2560.0, 1600.0, 40.0);
        let l = compute(4000.0, 3000.0, vw, vh, &w, top);
        assert_eq!(l.kind, LayoutKind::Contain);
        assert_eq!(l.frame, (1600.0f64 * 0.03).round());
        assert!(
            close(l.w / l.h, 4000.0 / 3000.0),
            "keeps the painting's shape"
        );
        // Inside the margin, with room for the frame, centred in the area under the menu bar.
        assert!(l.h + 2.0 * l.frame <= (vh - top) * (1.0 - 2.0 * w.fit_margin) + 1e-6);
        assert!(close(l.x, (vw - l.w) / 2.0));
        assert!(
            close(l.y - top, vh - (l.y + l.h)),
            "equal space above and below"
        );
    }

    #[test]
    fn pan_rests_glides_and_comes_back() {
        let w = WallpaperSettings::default();
        let l = compute(6209.0, 2100.0, 2560.0, 1707.0, &w, 0.0);
        let pan = Pan::new(&l, 5.0, 20.0).unwrap();
        let (p, m) = (pan.pause_secs, pan.travel_secs);
        assert!(close(m, l.travel / 5.0));
        assert_eq!(pan.offset_at(0.0), 0.0);
        assert_eq!(pan.offset_at(p - 0.01), 0.0, "rests at the start edge");
        assert!(
            close(pan.offset_at(p + m / 2.0), -l.travel / 2.0),
            "halfway at half time"
        );
        assert!(
            close(pan.offset_at(p + m + 1.0), -l.travel),
            "rests at the far edge"
        );
        assert!(
            close(pan.offset_at(2.0 * p + 1.5 * m), -l.travel / 2.0),
            "and glides back"
        );
        assert!(close(pan.offset_at(pan.cycle_secs()), 0.0));
        assert!(close(
            pan.offset_at(pan.cycle_secs() * 3.0 + p + m / 2.0),
            -l.travel / 2.0
        ));
        // It eases: slower near the edges than in the middle.
        let near_edge = (pan.offset_at(p + 1.0) - pan.offset_at(p)).abs();
        let middle = (pan.offset_at(p + m / 2.0 + 1.0) - pan.offset_at(p + m / 2.0)).abs();
        assert!(near_edge < middle);
    }

    #[test]
    fn pan_sleeps_between_pixels_and_through_rests() {
        let w = WallpaperSettings::default();
        let l = compute(6209.0, 2100.0, 2560.0, 1707.0, &w, 0.0);
        let pan = Pan::new(&l, 5.0, 20.0).unwrap();
        // Resting: sleep until the glide starts (capped at a second so pausing stays responsive).
        assert!(close(pan.next_step(19.5, 2.0), 0.5));
        assert!(close(pan.next_step(0.0, 2.0), 1.0));
        // Mid-glide at about 7 points a second on a 2x display: one device pixel is ~70 ms.
        let step = pan.next_step(pan.pause_secs + pan.travel_secs / 2.0, 2.0);
        assert!(step > 0.033 && step < 0.15, "{step}");
        // Never more often than 30 times a second, however fast it moves.
        let fast = Pan::new(&l, 60.0, 0.0).unwrap();
        assert!(close(fast.next_step(fast.travel_secs / 2.0, 2.0), 0.033));
        assert_eq!(snap(-10.26, 2.0), -10.5);
    }

    #[test]
    fn nothing_to_pan_means_no_pan() {
        let w = WallpaperSettings::default();
        let cover = compute(4096.0, 2642.0, 2560.0, 1707.0, &w, 0.0);
        assert!(Pan::new(&cover, 5.0, 20.0).is_none());
        // Degenerate sizes don't divide by zero.
        let none = compute(0.0, 0.0, 2560.0, 1707.0, &w, 0.0);
        assert_eq!(none.kind, LayoutKind::Cover);
    }
}
