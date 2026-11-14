//! Materials. Where a plain gradient would only suggest gilt, brass or glass,
//! these are shaded: a polished frame reflects a studio, a glass plate bends
//! and frosts the painting behind it.
//!
//! An element asks for a material with an effect slot ([`slot`]). The
//! [`Materials`] compositor draws Rune's frame as usual and paints a rune-fx
//! layer at each slot, in painter order. Every material here is still, so a
//! window with them goes idle like any other.
//!
//! `GALERIA_FX=0` turns materials off: slots paint nothing and the elements'
//! own gradients show instead.

use rune::compose::{Compositor, DeviceKey, Frame};
use rune::render::{EffectPhase, EffectSite, KIND_EFFECT};
use rune::El;
use rune_fx::{
    rgb, FillAnchor, Fx, FxFrame, FxLayer, FxOptions, FxRenderer, FxSlot, GrainParams, LensParams,
    LiquidMetalParams, PlasticParams, Shape,
};
use std::sync::atomic::{AtomicBool, Ordering};

/// A material an effect slot can ask for. The slot's data carries its few numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Material {
    /// Polished gold. `border` > 0 draws only a moulding that wide (a picture frame).
    Gilt { border: f32 },
    /// A brass plate: warmer and flatter than gilt, with a narrow rolled edge.
    Brass,
    /// Lacquered wood or metal moulding: black, white or walnut.
    Lacquer { border: f32, finish: Finish },
    /// Glossy black acrylic.
    Acrylic,
    /// Glass that refracts at its rim and frosts what is behind it (0 clear, 1 heavily frosted).
    Glass { frost: f32, smoke: f32 },
    /// Paper grain over whatever is under it.
    Grain { amount: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    Black,
    White,
    Walnut,
}

const GILT: u32 = 1;
const BRASS: u32 = 2;
const LACQUER: u32 = 3;
const ACRYLIC: u32 = 4;
const GLASS: u32 = 5;
const GRAIN: u32 = 6;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether windows draw materials. False until a compositor is attached, and
/// when `GALERIA_FX=0`: elements then draw their plain look instead.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The compositor to attach to a window, or `None` when materials are switched off.
pub fn materials() -> Option<Materials> {
    if std::env::var("GALERIA_FX").is_ok_and(|v| v == "0" || v.eq_ignore_ascii_case("off")) {
        return None;
    }
    ENABLED.store(true, Ordering::Relaxed);
    Some(Materials::default())
}

/// An element that paints `material` over its own box (give it a size or pin it).
/// `zoom` is how large the thing is drawn relative to life size (a plate in a
/// settings preview is smaller), so rims and bevels shrink with it.
pub fn slot(material: Material, zoom: f32) -> El {
    let (mark, [a, b], backdrop) = match material {
        Material::Gilt { border } => (GILT, [border, 0.0], false),
        Material::Brass => (BRASS, [0.0; 2], false),
        Material::Lacquer { border, finish } => (LACQUER, [border, finish as u8 as f32], false),
        Material::Acrylic => (ACRYLIC, [0.0; 2], false),
        Material::Glass { frost, smoke } => (GLASS, [frost, smoke], true),
        Material::Grain { amount } => (GRAIN, [amount, 0.0], false),
    };
    let data = [a, b, 0.0, zoom.clamp(0.05, 8.0)];
    let e = rune::effect(mark).effect_data(data).inert().a11y_hidden();
    if backdrop {
        e.effect_backdrop(true)
    } else {
        e
    }
}

/// The rune-fx layer for a slot of `kind` with data `d`, shaped like the slot.
fn layer(kind: u32, d: [f32; 4], shape: Shape) -> Option<FxLayer> {
    let z = if d[3] > 0.0 { d[3] } else { 1.0 };
    let fx = match kind {
        GILT => {
            // Still (no flow): the reflection of a studio in leaf gold, pinned to the frame.
            let gold = LiquidMetalParams {
                tint: rgb(0xf0c463),
                light: rgb(0xfff1d0),
                flow: 0.0,
                bevel: (d[0] * 0.5).max(3.0),
                dome: 0.7,
                contrast: 1.08,
                iridescence: 0.0,
                ..LiquidMetalParams::gold()
            }
            .anchor(FillAnchor::Element);
            if d[0] > 0.0 {
                Fx::fill(gold.border(d[0]))
            } else {
                Fx::fill(gold)
            }
        }
        BRASS => Fx::fill(
            LiquidMetalParams {
                tint: rgb(0xd8b25c),
                light: rgb(0xfff0c8),
                flow: 0.0,
                bevel: 2.5 * z,
                // Nearly flat, so the whole face reflects the lit side of the room and
                // the engraving stays readable; only the rolled edge catches the rest.
                dome: 0.03,
                angle: -0.35,
                contrast: 0.7,
                iridescence: 0.0,
                ..LiquidMetalParams::gold()
            }
            .anchor(FillAnchor::Element),
        ),
        LACQUER => {
            let (color, roughness) = match d[1] as u32 {
                1 => (rgb(0xf1eee6), 0.55),
                2 => (rgb(0x4a2d18), 0.42),
                _ => (rgb(0x141414), 0.22),
            };
            let moulding = PlasticParams {
                color,
                roughness,
                dome: 0.6,
                bevel: (d[0] * 0.5).max(2.0),
                angle: -0.6,
            };
            if d[0] > 0.0 {
                Fx::fill(moulding.border(d[0]))
            } else {
                Fx::fill(moulding)
            }
        }
        ACRYLIC => Fx::fill(PlasticParams {
            color: rgb(0x070708),
            roughness: 0.2,
            dome: 0.05,
            bevel: 4.0 * z,
            angle: -0.7,
        }),
        GLASS => Fx::lens(LensParams {
            bevel: 11.0 * z,
            depth: 13.0 * z,
            dispersion: 0.1,
            magnification: 1.02,
            frost: d[0].clamp(0.0, 1.0),
            // The plate casts its own shadow as an element; the glass adds none.
            shadow: 0.0,
            rim: 1.3 * z.max(0.6),
            tint: rgb(0x0c0c10),
            tint_amount: d[1].clamp(0.0, 1.0),
            ..LensParams::default()
        }),
        GRAIN => Fx::grain(GrainParams {
            amount: d[0],
            fps: 0.0,
            feather: 0.0,
        }),
        _ => return None,
    };
    Some(fx.on(shape).at(0.0))
}

/// Draws a window's frame, with a material at every effect slot.
#[derive(Default)]
pub struct Materials {
    fx: Option<(DeviceKey, FxRenderer)>,
}

impl<S: 'static> Compositor<S> for Materials {
    fn render(&mut self, f: &mut Frame<'_, S>) {
        // Which instances are slots, and the layer each one paints.
        let mut at: Vec<u32> = Vec::new();
        let mut layers: Vec<FxLayer> = Vec::new();
        let mut placements: Vec<FxSlot> = Vec::new();
        for (n, i) in f.rt.instances.iter().enumerate() {
            if i.params[1] != KIND_EFFECT {
                continue;
            }
            let kind = f.rt.marks.get(n).copied().unwrap_or(0) & 0xff;
            // Device pixels per point at the slot (the display's scale and any zoom).
            let k = i.params[3].max(0.01);
            let [x, y, w, h] = i.rect.map(|v| v / k);
            let radius = i.radii.iter().fold(0.0f32, |a, v| a.max(v.abs())) / k;
            let Some(l) = layer(kind, i.uv, Shape::rounded(x, y, w, h, radius)) else {
                continue;
            };
            at.push(n as u32);
            layers.push(l);
            placements.push(FxSlot::default().scale(k).clip(i.clip, i.fade));
        }
        if layers.is_empty() {
            // Nothing shaded in this frame: Rune's own path, untouched.
            return f.draw_ui();
        }

        let (device, queue, size) = (f.device, f.queue, f.size);
        // A window can move to another GPU; what was made on the old one stays there.
        let key = f.device_key();
        if self.fx.as_ref().is_none_or(|(made_on, _)| *made_on != key) {
            self.fx = Some((
                key,
                FxRenderer::new(device, f.format, FxOptions { dither: true }),
            ));
        }
        let Some((_, fx)) = self.fx.as_mut() else {
            return f.draw_ui();
        };
        fx.set_srgb_blend(f.srgb_blend());
        // Slots inside a fading or blurred element draw into that element's own layer.
        fx.add_format(device, wgpu::TextureFormat::Rgba16Float);
        fx.prepare_with_slots(
            device,
            queue,
            &FxFrame {
                viewport: [size.0, size.1],
                scale: 1.0,
                layers: &layers,
                backdrop: None,
                sources: &[],
            },
            &placements,
        );
        let clear = f.rt.clear_color();
        f.ui.prepare(device, queue, &mut f.rt.text.atlas, &f.rt.instances, size);
        f.ui.draw_with_backdrop(
            f.encoder,
            f.target,
            clear,
            &mut |phase, site: &EffectSite<'_>| {
                let Some(k) = at.iter().position(|n| *n == site.index) else {
                    return;
                };
                match phase {
                    // Glass reads what was drawn before it, blurred for frost.
                    EffectPhase::Backdrop(encoder) => {
                        if let Some(backdrop) = site.backdrop {
                            fx.blur_backdrop(device, queue, encoder, backdrop, k);
                        }
                    }
                    EffectPhase::Draw(pass) => fx.draw_layer_in(pass, site.format, site.origin, k),
                }
            },
        );
    }
}
