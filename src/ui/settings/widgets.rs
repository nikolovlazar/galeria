//! The pieces every settings page is made of: sections of grouped rows, and the
//! controls in them. Controls come from rune-kit, themed by `theme::Look`; where
//! the system has a native control (pop-up menus) that is used instead.

use super::SettingsUi;
use crate::ui::theme::Look;
use rune::{
    col, el, row, text, Align, Color, Cursor, Drag, El, Phase, PopupMenu, TextEdit, TextStyle, Tx,
};
use rune_kit::{Button, ButtonVariant, Checkbox, ControlSize, SegmentedControl, Select, TextInput};

pub type S = SettingsUi;

pub fn title(_: &Look) -> TextStyle {
    TextStyle::new(17.0, 650.0).lh(22.0).tracking(-0.2)
}

pub fn body() -> TextStyle {
    TextStyle::new(13.0, 400.0).lh(18.0)
}

pub fn strong() -> TextStyle {
    TextStyle::new(13.0, 550.0).lh(18.0)
}

pub fn small() -> TextStyle {
    TextStyle::new(11.5, 400.0).lh(16.0)
}

/// Body text that wraps.
pub fn para(l: &Look, s: impl Into<String>) -> El {
    text(s.into()).font(body()).color(l.text).wrap().w_full()
}

pub fn muted(l: &Look, s: impl Into<String>) -> El {
    text(s.into()).font(body()).color(l.muted).wrap().w_full()
}

pub fn hint(l: &Look, s: impl Into<String>) -> El {
    text(s.into()).font(small()).color(l.muted).wrap().w_full()
}

/// A card: the grouped surface rows sit on.
pub fn card(l: &Look) -> El {
    col()
        .w_full()
        .fill(l.card)
        .radius(l.kit.radius.lg)
        .border(l.hair, l.card_border)
}

/// A titled group of rows on a card, with an optional line of explanation and
/// actions at the title's trailing edge.
pub fn section(
    l: &Look,
    key: &str,
    heading: &str,
    description: Option<String>,
    actions: Vec<El>,
    rows: Vec<El>,
) -> El {
    let mut head = col().grow(1.0).min_w(0.0).gap(2.0).child(
        text(heading.to_string())
            .key("heading")
            .role("heading")
            .level(2)
            .font(title(l))
            .color(l.text),
    );
    if let Some(d) = description {
        head = head.child(muted(l, d));
    }
    let mut top = row().w_full().items_end().gap(12.0).child(head);
    if !actions.is_empty() {
        top = top.child(row().gap(8.0).children(actions));
    }
    let mut group = card(l).key("card");
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        group = group.child(r);
        if i + 1 < n {
            group = group.child(el().h(l.hair).mx(16.0).fill(l.separator).a11y_hidden());
        }
    }
    col().key(key).w_full().gap(10.0).child(top).child(group)
}

/// One setting: what it is, an optional explanation, and its control.
pub fn setting(
    l: &Look,
    key: &str,
    label: impl Into<String>,
    explain: Option<String>,
    control: Option<El>,
) -> El {
    let label = label.into();
    let mut words = col().grow(1.0).min_w(0.0).gap(1.0).child(
        text(label.clone())
            .key("label")
            .font(body())
            .color(l.text)
            .wrap()
            .a11y_hidden(),
    );
    if let Some(h) = explain {
        words = words.child(hint(l, h));
    }
    let mut r = row()
        .key(key)
        .label(label)
        .w_full()
        .min_h(46.0)
        .px(16.0)
        .py(9.0)
        .gap(16.0)
        .center_y()
        .child(words);
    if let Some(c) = control {
        r = r.child(c);
    }
    r
}

/// Dim a row whose control cannot be used right now.
pub fn disabled(e: El, off: bool) -> El {
    if off {
        e.dim(0.45).inert()
    } else {
        e
    }
}

/// Free content inside a card (a gallery, a chart), padded like a row.
pub fn block(content: El) -> El {
    el().w_full().px(16.0).py(14.0).child(content)
}

pub fn toggle(l: &Look, on: bool, f: impl Fn(&mut S) + 'static) -> El {
    // The kit's switch paints a sheen on the track and the knob; this one is flat:
    // a plain track in the muted or accent colour with a plain knob.
    let c = &l.kit.c;
    let (track, knob) = if on {
        (c.bg_accent, c.knob_on)
    } else {
        (c.layer_04, c.knob)
    };
    let (tw, th, kw, kh, inset) = (36.0, 16.0, 21.0, 14.0, 1.0);
    let x = if on { tw - inset - kw } else { inset };
    let thumb = el()
        .key("thumb")
        .size(kw, kh)
        .absolute(x, inset)
        .radius(kh / 2.0)
        .fill(knob)
        .motion(l.kit.motion.control);
    let track = el()
        .key("control")
        .size(tw, th)
        .radius(th / 2.0)
        .fill(track)
        .hover(c.hover)
        .focus_ring(2.0, 1.0, c.border_focus)
        .motion(l.kit.motion.control)
        .a11y_hidden()
        .child(thumb);
    row()
        .role("switch")
        .checked(on)
        .focusable(true)
        .on_press(f)
        .child(track)
}

pub fn checkbox(l: &Look, on: bool, label: &str, f: impl Fn(&mut S) + 'static) -> El {
    Checkbox::new(on).label(label).on_toggle(f).build(&l.kit)
}

/// A row of mutually exclusive choices.
pub fn segmented(
    l: &Look,
    labels: &[&str],
    selected: Option<usize>,
    f: impl Fn(&mut S, usize) + Clone + 'static,
) -> El {
    SegmentedControl::new(labels, selected)
        .fit()
        .on_pick(move |s: &mut S, i, _: &mut Tx| f(s, i))
        .build(&l.kit)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Normal,
    /// The one thing this page is for: gilt.
    Primary,
    Danger,
    /// Text only.
    Quiet,
}

pub fn button(
    l: &Look,
    label: impl Into<String>,
    kind: Kind,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    button_if(l, label, kind, true, f)
}

pub fn button_if(
    l: &Look,
    label: impl Into<String>,
    kind: Kind,
    enabled: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let variant = match kind {
        Kind::Normal => ButtonVariant::Neutral,
        Kind::Primary => ButtonVariant::Contrast,
        Kind::Danger => ButtonVariant::Danger,
        Kind::Quiet => ButtonVariant::Ghost,
    };
    Button::new(label)
        .variant(variant)
        .control_size(ControlSize::Regular)
        .disabled(!enabled)
        .on_press_tx(f)
        .build(&l.kit)
}

/// Text that acts: opens a page, shows a file.
pub fn link(l: &Look, label: impl Into<String>, f: impl Fn(&mut S, &mut Tx) + 'static) -> El {
    let label = label.into();
    text(label.clone())
        .role("link")
        .label(label)
        .focusable(true)
        .font(body())
        .color(l.gold)
        .cursor(Cursor::Pointer)
        .on_press_tx(f)
}

/// A small label in a rounded box: a license, a state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Good,
    Bad,
    Gold,
}

pub fn chip(l: &Look, label: impl Into<String>, tone: Tone) -> El {
    let (ink, wash) = match tone {
        Tone::Plain => (l.muted, l.text.alpha(0.07)),
        Tone::Good => (l.ok, l.ok_wash),
        Tone::Bad => (l.danger, l.danger_wash),
        Tone::Gold => (l.gold, l.gold_wash),
    };
    row()
        .h(20.0)
        .px(8.0)
        .center()
        .radius(10.0)
        .fill(wash)
        .child(
            text(label.into())
                .font(TextStyle::new(11.0, 600.0).lh(14.0))
                .color(ink),
        )
}

/// A pill that is on or off: a genre, a museum, a playlist.
pub fn choice(
    l: &Look,
    key: impl Into<String>,
    label: impl Into<String>,
    count: Option<usize>,
    on: bool,
    f: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let (ink, wash, edge) = if on {
        (l.on_gold, l.gold, l.gold)
    } else {
        (l.text, Color::TRANSPARENT, l.card_border)
    };
    let mut r = row()
        .key(key)
        .role("checkbox")
        .label(label.clone())
        .checked(on)
        .focusable(true)
        .h(28.0)
        .px(12.0)
        .gap(6.0)
        .center()
        .radius(14.0)
        .fill(wash)
        .border(1.0, edge)
        .hover(if on {
            Color::BLACK.alpha(0.06)
        } else {
            l.kit.c.hover
        })
        .cursor(Cursor::Pointer)
        .on_press_tx(f)
        .child(
            text(label)
                .font(TextStyle::new(12.5, if on { 600.0 } else { 400.0 }).lh(16.0))
                .color(ink)
                .a11y_hidden(),
        );
    if let Some(n) = count {
        r = r.child(
            text(n.to_string())
                .font(TextStyle::new(11.0, 400.0).lh(14.0))
                .color(if on { l.on_gold.alpha(0.7) } else { l.faint })
                .a11y_hidden(),
        );
    }
    r
}

/// Chips that wrap onto as many lines as they need.
pub fn wrap(items: Vec<El>) -> El {
    row().w_full().flex_wrap().gaps(8.0, 8.0).children(items)
}

/// A single-line text field bound to `edit`.
pub fn field(
    l: &Look,
    edit: &TextEdit,
    placeholder: &str,
    width: Option<f32>,
    access: impl for<'a> Fn(&'a mut S) -> &'a mut TextEdit + 'static,
    submit: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let input = TextInput::new(edit)
        .placeholder(placeholder)
        .label(placeholder)
        .width(width)
        .on_edit(access)
        .on_submit(submit)
        .build(&l.kit);
    // Without a width of its own, the field takes what its row gives it.
    if width.is_none() {
        input.w_full()
    } else {
        input
    }
}

/// One of a list, chosen from a menu. `options` are `(id, label)`.
///
/// macOS and Windows open the system's own pop-up menu over the control. Where
/// there is none, the kit's menu opens inside the window (`layer`).
// A control needs its look, the page's state and overlay layer, a key, its options, the
// chosen one, a width and a handler: none of them belong together in a struct.
#[allow(clippy::too_many_arguments)]
pub fn picker(
    l: &Look,
    s: &S,
    layer: &mut Vec<El>,
    key: &str,
    options: &[(String, String)],
    current: &str,
    width: f32,
    choose: impl Fn(&mut S, &str, &mut Tx) + Clone + 'static,
) -> El {
    let labels: Vec<&str> = options.iter().map(|(_, label)| label.as_str()).collect();
    let shown = options
        .iter()
        .find(|(id, _)| id == current)
        .map(|(_, label)| label.as_str());
    if !rune::popup_menu_supported() {
        let ids: Vec<(String, String)> = options.to_vec();
        let (open_key, toggle_key) = (key.to_string(), key.to_string());
        return Select::new(key, &labels)
            .current(shown)
            .width(width)
            .label(key)
            .open(s.open_select.as_deref() == Some(key))
            .on_toggle(move |s: &mut S| {
                s.open_select = if s.open_select.as_deref() == Some(toggle_key.as_str()) {
                    None
                } else {
                    Some(toggle_key.clone())
                }
            })
            .on_close(move |s: &mut S| {
                if s.open_select.as_deref() == Some(open_key.as_str()) {
                    s.open_select = None;
                }
            })
            .on_choose(move |s: &mut S, label: &str, tx: &mut Tx| {
                s.open_select = None;
                if let Some((id, _)) = ids.iter().find(|(_, l)| l == label) {
                    choose(s, id, tx);
                }
            })
            .build(&l.kit, layer);
    }
    let (menu_options, selected, anchor) = (options.to_vec(), current.to_string(), key.to_string());
    row()
        .key(key)
        .role("combobox")
        .label(shown.unwrap_or(key).to_string())
        .focusable(true)
        .w(width)
        .h(28.0)
        .pl(10.0)
        .pr(7.0)
        .gap(6.0)
        .center_y()
        .radius(l.kit.radius.md)
        .fill(l.kit.c.bg_button_neutral)
        .border(l.hair, l.card_border)
        .hover(l.kit.c.hover)
        .on_press_tx(move |_: &mut S, tx: &mut Tx| {
            let menu = PopupMenu::picker(
                menu_options
                    .iter()
                    .map(|(id, label)| (id.clone(), label.clone())),
                &selected,
            );
            let choose = choose.clone();
            tx.popup_menu(
                menu,
                anchor.as_str(),
                move |s: &mut S, picked: Option<&str>, tx: &mut Tx| {
                    if let Some(id) = picked {
                        choose(s, id, tx);
                    }
                },
            );
        })
        .child(
            text(shown.unwrap_or("").to_string())
                .font(body())
                .color(l.text)
                .truncate()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden(),
        )
        .child(
            rune::icon(&crate::ui::icons::CHEVRONS, 14.0)
                .color(l.muted)
                .a11y_hidden(),
        )
}

/// A slider that follows the pointer while it is held and commits when it is let go.
pub fn slider(
    l: &Look,
    s: &S,
    key: &'static str,
    value: f64,
    (min, max, step): (f64, f64, f64),
    format: fn(f64) -> String,
    commit: impl Fn(&mut S, f64) + Clone + 'static,
) -> El {
    let shown = s
        .sliding
        .as_ref()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| *v)
        .unwrap_or(value)
        .clamp(min, max);
    let (w, h, k, bar) = (190.0f32, 22.0f32, 18.0f32, 4.0f32);
    let travel = w - k;
    let frac = (((shown - min) / (max - min).max(1e-9)) as f32).clamp(0.0, 1.0);
    let at = move |x: f32| {
        let f = ((x - k / 2.0) / travel).clamp(0.0, 1.0) as f64;
        let v = min + f * (max - min);
        ((v / step).round() * step).clamp(min, max)
    };
    let (drag_commit, step_commit) = (commit.clone(), commit);
    row()
        .gap(12.0)
        .center_y()
        .child(
            el().key(key)
                .role("slider")
                .label(key)
                .range_value(shown, min, max)
                .range_step(step)
                .a11y_value(format(shown))
                .focusable(true)
                .size(w, h)
                .cursor(Cursor::Pointer)
                .on_step(move |s: &mut S, steps, _| {
                    let v = (shown + steps as f64 * step).clamp(min, max);
                    s.sliding = Some((key.to_string(), v));
                    step_commit(s, v);
                })
                .on_drag(move |s: &mut S, d: &mut Drag, tx| match d.phase {
                    Phase::Down | Phase::Move => {
                        s.sliding = Some((key.to_string(), at(d.x)));
                        s.slider_held = true;
                        tx.instant();
                    }
                    Phase::Up => {
                        s.slider_held = false;
                        if d.cancelled() {
                            s.sliding = None;
                        } else {
                            let v = at(d.x);
                            // Keep showing the new value until the setting itself catches up.
                            s.sliding = Some((key.to_string(), v));
                            drag_commit(s, v);
                        }
                    }
                })
                .child(
                    el().size(w, bar)
                        .absolute(0.0, (h - bar) / 2.0)
                        .radius(bar / 2.0)
                        .fill(l.text.alpha(0.14))
                        .a11y_hidden(),
                )
                .child(
                    el().size(k / 2.0 + frac * travel, bar)
                        .absolute(0.0, (h - bar) / 2.0)
                        .radius(bar / 2.0)
                        .fill(l.gold)
                        .a11y_hidden(),
                )
                .child(
                    el().size(k, k)
                        .absolute(frac * travel, (h - k) / 2.0)
                        .radius(k / 2.0)
                        .fill(Color::WHITE)
                        .shadow(1.0, 3.0, Color::BLACK.alpha(0.3))
                        .border(l.hair, Color::BLACK.alpha(0.12))
                        .a11y_hidden(),
                ),
        )
        .child(
            text(format(shown))
                .w(64.0)
                .font(TextStyle::new(12.0, 400.0).features("'tnum' 1"))
                .color(l.muted)
                .align(Align::End)
                .a11y_hidden(),
        )
}

/// A number with what it counts under it.
pub fn stat(l: &Look, n: usize, what: &str, tone: Tone) -> El {
    let ink = match tone {
        Tone::Good => l.ok,
        Tone::Bad => l.warn,
        _ => l.text,
    };
    col()
        .grow(1.0)
        .min_w(92.0)
        .py(12.0)
        .px(14.0)
        .gap(2.0)
        .radius(l.kit.radius.md)
        .fill(l.text.alpha(0.04))
        .child(
            text(n.to_string())
                .font(TextStyle::new(20.0, 650.0).lh(24.0).features("'tnum' 1"))
                .color(ink),
        )
        .child(text(what.to_string()).font(small()).color(l.muted).wrap())
}

/// A message across the page: something went wrong, or needs saying.
pub fn notice(l: &Look, message: impl Into<String>, bad: bool) -> El {
    let (ink, wash) = if bad {
        (l.danger, l.danger_wash)
    } else {
        (l.text, l.gold_wash)
    };
    el().w_full()
        .px(14.0)
        .py(10.0)
        .radius(l.kit.radius.md)
        .fill(wash)
        .child(text(message.into()).font(body()).color(ink).wrap().w_full())
}

/// `Name   value` pairs, the value wrapping.
pub fn facts(l: &Look, rows: &[(&str, String)]) -> El {
    let mut c = col().w_full().gap(6.0);
    for (name, value) in rows.iter().filter(|(_, v)| !v.is_empty()) {
        c = c.child(
            row()
                .w_full()
                .gap(12.0)
                .items_start()
                .child(text(name.to_string()).w(92.0).font(body()).color(l.muted))
                .child(
                    text(value.clone())
                        .font(body())
                        .color(l.text)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0)
                        .selectable(true),
                ),
        );
    }
    c
}

pub fn bytes(n: u64) -> String {
    if n > 1_000_000_000 {
        format!("{:.1} GB", n as f64 / 1e9)
    } else if n > 1_000_000 {
        format!("{:.1} MB", n as f64 / 1e6)
    } else {
        format!("{} KB", (n as f64 / 1e3).round() as u64)
    }
}

/// `1234567` as `1,234,567`.
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_like_numbers() {
        assert_eq!(bytes(1_500), "2 KB");
        assert_eq!(bytes(5_809_302), "5.8 MB");
        assert_eq!(bytes(2_100_000_000), "2.1 GB");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(plural(1, "painting", "paintings"), "1 painting");
        assert_eq!(plural(2, "painting", "paintings"), "2 paintings");
    }
}
