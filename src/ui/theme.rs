//! Galeria's look: a quiet gallery wall, with gilt for what is selected.

use rune::Color;
use rune_kit::{Kit, PaletteFocus, PaletteSeed};

/// An sRGB colour from `0xRRGGBB`.
pub fn rgb(hex: u32) -> Color {
    let channel = |shift: u32| rune::color::srgb_decode(((hex >> shift) & 0xff) as f32 / 255.0);
    Color {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// `rgb` with an alpha.
pub fn rgba(hex: u32, a: f32) -> Color {
    rgb(hex).alpha(a)
}

/// Parse `#rgb`, `#rrggbb` or `#rrggbbaa`; anything else is `fallback`.
pub fn hex_or(s: &str, fallback: Color) -> Color {
    rune::color::hex(s.trim()).unwrap_or(fallback)
}

/// The colours Galeria's own elements use beyond what the kit's components need.
#[derive(Clone, Debug)]
pub struct Look {
    pub kit: Kit,
    /// The window itself.
    pub bg: Color,
    /// The kit as the sidebar's rows use it.
    pub side: Kit,
    /// Cards and grouped rows.
    pub card: Color,
    pub card_border: Color,
    pub separator: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    /// Gilt: the selected thing, primary buttons, links.
    pub gold: Color,
    /// Text on gilt.
    pub on_gold: Color,
    /// A wash of gilt behind a selected row.
    pub gold_wash: Color,
    pub gold_border: Color,
    pub ok: Color,
    pub ok_wash: Color,
    pub warn: Color,
    pub danger: Color,
    pub danger_wash: Color,
    /// Behind a painting's thumbnail before it has loaded.
    pub well: Color,
    pub hair: f32,
}

/// The look for the system's appearance. `scale` is device pixels per point (hairlines are one pixel).
pub fn look(dark: bool, scale: f32) -> Look {
    let (bg, fg, gold) = if dark {
        (rgb(0x181716), rgb(0xf1ede6), rgb(0xe3bf66))
    } else {
        (rgb(0xf4f1ea), rgb(0x1d1c1a), rgb(0x8a6417))
    };
    let mut kit = Kit::from_palette_focus(
        PaletteSeed {
            bg,
            fg,
            accent: gold,
        },
        PaletteFocus::Accent,
    );
    kit.radius.sm = 5.0;
    kit.radius.md = 7.0;
    kit.radius.lg = 10.0;
    kit.radius.xl = 14.0;
    let on_gold = if dark { rgb(0x1d1708) } else { rgb(0xfffaf0) };
    // The kit's "contrast" button is Galeria's primary button: gilt.
    kit.c.bg_contrast = gold;
    kit.c.text_contrast = on_gold;
    kit.c.icon_contrast = on_gold;
    kit.c.bg_icon_button_contrast = gold;
    kit.c.knob_on = Color::WHITE;
    // The sidebar marks the open page in gilt rather than grey.
    let mut side = kit.clone();
    side.c.pressed = gold.alpha(if dark { 0.16 } else { 0.14 });
    side.c.text_base = if dark { rgb(0xf0d491) } else { rgb(0x6b4c0f) };
    side.c.icon_base = gold;
    let hair = 1.0 / scale.max(1.0);
    if dark {
        Look {
            card: rgb(0x201f1d),
            card_border: Color::WHITE.alpha(0.075),
            separator: Color::WHITE.alpha(0.07),
            text: fg,
            muted: kit.c.text_muted,
            faint: kit.c.text_faint,
            gold,
            on_gold,
            gold_wash: gold.alpha(0.16),
            gold_border: gold.alpha(0.55),
            ok: rgb(0x9fd18b),
            ok_wash: rgb(0x9fd18b).alpha(0.16),
            warn: rgb(0xe8b25c),
            danger: rgb(0xf08a7a),
            danger_wash: rgb(0xf08a7a).alpha(0.14),
            well: rgb(0x0f0e0d),
            hair,
            bg,
            kit,
            side,
        }
    } else {
        Look {
            card: rgb(0xfcfaf5),
            card_border: Color::BLACK.alpha(0.09),
            separator: Color::BLACK.alpha(0.08),
            text: fg,
            muted: kit.c.text_muted,
            faint: kit.c.text_faint,
            gold,
            on_gold,
            gold_wash: gold.alpha(0.13),
            gold_border: gold.alpha(0.6),
            ok: rgb(0x3d7a2a),
            ok_wash: rgb(0x3d7a2a).alpha(0.12),
            warn: rgb(0x9a6408),
            danger: rgb(0xb43a2a),
            danger_wash: rgb(0xb43a2a).alpha(0.1),
            well: rgb(0xe2ddd1),
            hair,
            bg,
            kit,
            side,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_literals_are_srgb() {
        assert_eq!(rgb(0x000000), Color::BLACK);
        assert_eq!(rgb(0xffffff), Color::WHITE);
        // Mid grey is far darker in linear light than its code value suggests.
        let grey = rgb(0x808080);
        assert!((grey.r - 0.2158).abs() < 1e-3 && grey.r == grey.g && grey.g == grey.b);
        assert_eq!(rgb(0x1d1c1a), rune::color::hex("#1d1c1a").unwrap());
        assert_eq!(rgba(0xff0000, 0.5).a, 0.5);
    }

    #[test]
    fn hex_colours_fall_back() {
        assert_eq!(hex_or("nonsense", Color::WHITE), Color::WHITE);
        assert!(hex_or(" #1d1c1a ", Color::WHITE).r < 0.05);
    }
}
