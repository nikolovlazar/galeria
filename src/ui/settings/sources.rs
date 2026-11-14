//! Museums: which ones take part, and the licensing rules Galeria downloads by.

use super::widgets::{self, Kind, Tone};
use super::{scroll_page, SettingsUi, Tab};
use crate::actions;
use crate::licensing;
use crate::ui::theme::Look;
use rune::tok::Family;
use rune::{col, el, row, text, El, TextStyle};
use serde_json::json;

fn open(l: &Look, label: &str, url: &str) -> El {
    let url = url.to_string();
    widgets::link(l, format!("{label} ↗"), move |_, _| {
        let _ = actions::open_url(&url);
    })
}

pub fn page(s: &SettingsUi, l: &Look, layer: &mut Vec<El>) -> El {
    let snap = &s.snap;
    let rules = licensing::rules();
    let disabled = &snap.settings.sources.disabled_museums;
    let off = snap.museums.iter().filter(|m| !m.enabled).count();
    let shown: usize = snap
        .museums
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.count)
        .sum();
    let license_label = |id: &Option<String>| {
        id.as_ref().map(|id| {
            rules
                .licenses
                .iter()
                .find(|x| &x.id == id)
                .map(|x| x.label.clone())
                .unwrap_or_else(|| id.clone())
        })
    };

    // Museums in the library ----------------------------------------------------------
    let mut museums = Vec::new();
    for m in &snap.museums {
        let (name, on, list) = (m.name.clone(), m.enabled, disabled.clone());
        let mut says = row()
            .w_full()
            .gap(8.0)
            .items_start()
            .child(if m.policy.direct {
                widgets::chip(
                    l,
                    m.policy
                        .license
                        .clone()
                        .unwrap_or_else(|| "Open access".into()),
                    Tone::Good,
                )
            } else {
                widgets::chip(
                    l,
                    m.policy
                        .license
                        .clone()
                        .unwrap_or_else(|| "No open license".into()),
                    Tone::Plain,
                )
            });
        says = says.child(
            widgets::hint(
                l,
                if m.policy.direct {
                    format!(
                        "{} programme: Galeria can download from this museum directly.",
                        m.policy.program.clone().unwrap_or_default()
                    )
                } else {
                    m.policy.summary.clone()
                },
            )
            .grow(1.0)
            .min_w(0.0)
            .pt(2.0),
        );
        let mut words = col()
            .grow(1.0)
            .min_w(0.0)
            .gap(4.0)
            .child(
                row()
                    .gap(6.0)
                    .items_end()
                    .child(text(m.name.clone()).font(widgets::strong()).color(l.text))
                    .child(
                        text(format!(
                            "· {}",
                            widgets::plural(m.count, "painting", "paintings")
                        ))
                        .font(widgets::body())
                        .color(l.muted),
                    ),
            )
            .child(says);
        if let Some(url) = &m.policy.policy_url {
            words = words.child(open(l, "Its terms", url));
        }
        let mut line = row()
            .key(format!("museum-{}", m.name))
            .w_full()
            .px(16.0)
            .py(12.0)
            .gap(14.0)
            .items_start()
            .child(el().pt(1.0).child(widgets::toggle(l, on, move |s| {
                let next: Vec<String> = if on {
                    list.iter()
                        .cloned()
                        .chain(std::iter::once(name.clone()))
                        .collect()
                } else {
                    list.iter().filter(|x| **x != name).cloned().collect()
                };
                s.cx.patch(json!({ "sources": { "disabledMuseums": next } }))
            })))
            .child(words);
        if m.name != "Collection not recorded" {
            let (source, museum, direct) =
                (m.policy.source.clone(), m.name.clone(), m.policy.direct);
            line = line.child(widgets::button(
                l,
                if m.policy.direct {
                    "Find More…"
                } else {
                    "On Commons…"
                },
                Kind::Normal,
                move |s, _| match (&source, direct) {
                    (Some(source), true) => s.discover_from(source, ""),
                    _ => s.discover_from("commons", &museum),
                },
            ));
        }
        let museum_name = m.name.clone();
        line = line.child(widgets::button(
            l,
            "Delete…",
            Kind::Danger,
            move |s, _| {
                s.confirm_delete_museum = Some(museum_name.clone());
            },
        ));
        let mut entry = col().w_full().child(if on { line } else { line.dim(0.6) });
        if s.confirm_delete_museum.as_deref() == Some(m.name.as_str()) {
            let museum_name = m.name.clone();
            let what = widgets::plural(m.count, "painting", "paintings");
            let label = format!(
                "Delete {}",
                widgets::plural(m.count, "Painting", "Paintings")
            );
            entry = entry.child(
                row()
                    .key("confirm-delete-museum")
                    .w_full()
                    .px(16.0)
                    .pb(12.0)
                    .child(
                        row()
                            .w_full()
                            .px(12.0)
                            .py(10.0)
                            .gap(12.0)
                            .center_y()
                            .flex_wrap()
                            .radius(l.kit.radius.md)
                            .fill(l.danger_wash)
                            .border(l.hair, l.danger.alpha(0.45))
                            .child(
                                text(format!(
                                    "Delete all {what} from {}? Their image files and manifest entries will be permanently removed.",
                                    m.name
                                ))
                                .font(widgets::small())
                                .color(l.text)
                                .wrap()
                                .grow(1.0)
                                .min_w(200.0),
                            )
                            .child(
                                widgets::button(l, label, Kind::Danger, move |s, _| {
                                    s.confirm_delete_museum = None;
                                    let museum = museum_name.clone();
                                    s.cx.run(move |app| {
                                        if let Err(e) = actions::delete_museum(app, &museum) {
                                            log::warn!("delete museum: {e}");
                                        }
                                    });
                                })
                                .key("confirm-delete-museum-yes"),
                            )
                            .child(
                                widgets::button(l, "Cancel", Kind::Normal, |s, _| {
                                    s.confirm_delete_museum = None;
                                })
                                .key("confirm-delete-museum-no"),
                            ),
                    ),
            );
        }
        museums.push(entry);
    }
    if snap.museums.is_empty() {
        museums.push(widgets::block(widgets::muted(
            l,
            "No museums yet: the manifest names none.",
        )));
    }
    let mut sections = vec![widgets::section(
        l,
        "museums",
        "Museums in Your Library",
        Some(format!(
            "Switch a museum off to leave its paintings out of the rotation. Nothing is deleted. {shown} of {} paintings are from museums that are on.",
            snap.library.valid
        )),
        if off > 0 {
            vec![widgets::button(l, "Turn All On", Kind::Normal, |s, _| {
                s.cx.patch(json!({ "sources": { "disabledMuseums": [] } }))
            })]
        } else {
            vec![]
        },
        museums,
    )];

    // Downloads -------------------------------------------------------------------------
    let sizes: Vec<(String, String)> = [4000, 6000, 8000, 12000, 16000]
        .iter()
        .map(|n: &u32| {
            (
                n.to_string(),
                format!("{} px", widgets::thousands(*n as u64)),
            )
        })
        .collect();
    sections.push(widgets::section(
        l,
        "downloads",
        "Downloads",
        Some("How Galeria saves what you pick in Discover.".into()),
        vec![],
        vec![
            widgets::setting(
                l,
                "max-size",
                "Largest size to keep",
                Some("Bigger originals are scaled down to this long edge. Files are saved as JPEG next to your other paintings and added to manifest.json with their license.".into()),
                Some(widgets::picker(
                    l,
                    s,
                    layer,
                    "max-long-edge",
                    &sizes,
                    &snap.settings.download.max_long_edge.to_string(),
                    130.0,
                    |s, id, _| s.cx.patch(json!({ "download": { "maxLongEdge": id.parse::<u32>().unwrap_or(8000) } })),
                )),
            ),
            widgets::setting(
                l,
                "find",
                "Find paintings",
                Some("Browse thumbnails from each museum and download the ones you like.".into()),
                Some(widgets::button(l, "Open Discover", Kind::Primary, |s, _| s.go(Tab::Discover))),
            ),
        ],
    ));

    // The rules ---------------------------------------------------------------------------
    let license_row = |chip: El, says: String| {
        row()
            .w_full()
            .gap(10.0)
            .items_start()
            .child(el().w(150.0).child(chip))
            .child(widgets::hint(l, says).grow(1.0).min_w(0.0).pt(2.0))
    };
    let mut accepted = col()
        .w_full()
        .gap(8.0)
        .child(text("Accepted").font(widgets::strong()).color(l.text));
    for x in rules.licenses.iter().filter(|x| x.allowed) {
        let says = if x.obligations.is_empty() {
            "No conditions.".to_string()
        } else {
            x.obligations.join(" ")
        };
        accepted = accepted.child(license_row(
            widgets::chip(l, x.label.clone(), Tone::Good),
            says,
        ));
    }
    let mut refused = col()
        .w_full()
        .gap(8.0)
        .child(text("Refused").font(widgets::strong()).color(l.text));
    for x in rules.licenses.iter().filter(|x| !x.allowed) {
        refused = refused.child(license_row(
            widgets::chip(l, x.label.clone(), Tone::Bad),
            x.reason.clone().unwrap_or_default(),
        ));
    }
    refused = refused.child(license_row(
        widgets::chip(l, "Anything else", Tone::Bad),
        "A license Galeria does not recognise, or none stated, is refused.".into(),
    ));
    sections.push(widgets::section(
        l,
        "rules",
        "What Galeria Is Allowed to Download",
        Some(format!(
            "Rules version {}, reviewed {}. They are a file compiled into the app (resources/licensing.json).",
            rules.version, rules.reviewed
        )),
        vec![],
        vec![widgets::block(
            col()
                .w_full()
                .gap(16.0)
                .child(widgets::para(l, rules.policy.clone()))
                .child(accepted)
                .child(refused)
                .child(widgets::hint(l, rules.disclaimer.clone())),
        )],
    ));

    // Open-access sources -----------------------------------------------------------------
    let mut cards = Vec::new();
    for src in &rules.sources {
        let id = src.id.clone();
        let mut facts: Vec<(&str, String)> = vec![("Proof checked", src.evidence.clone())];
        if let Some(asked) = &src.attribution.requested {
            facts.push((
                "Credit",
                format!(
                    "{}{asked}",
                    if src.attribution.required == Some(true) {
                        "Required: "
                    } else {
                        "Requested, not required: "
                    }
                ),
            ));
        }
        facts.push(("Images", src.images.clone()));
        if let Some(note) = &src.etiquette.note {
            facts.push(("Manners", note.clone()));
        }
        let mut body = col()
            .w_full()
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(text(src.name.clone()).font(widgets::strong()).color(l.text))
                    .child(widgets::chip(
                        l,
                        license_label(&src.grants).unwrap_or_else(|| "Per file".into()),
                        Tone::Good,
                    ))
                    .child(
                        text(src.program.clone())
                            .font(widgets::small())
                            .color(l.muted)
                            .grow(1.0)
                            .min_w(0.0)
                            .truncate(),
                    )
                    .child(widgets::button(
                        l,
                        "Browse…",
                        Kind::Normal,
                        move |s, _| s.discover_from(&id, ""),
                    )),
            )
            .child(widgets::para(l, src.summary.clone()));
        if !src.quote.is_empty() {
            body = body.child(
                row()
                    .w_full()
                    .gap(10.0)
                    .child(el().w(2.0).self_stretch().fill(l.gold.alpha(0.6)))
                    .child(
                        text(format!("“{}”", src.quote))
                            .font(widgets::body().italic())
                            .color(l.text.alpha(0.8))
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        let mut list = col().w_full().gap(6.0);
        for (name, value) in facts {
            let mono = name == "Proof checked";
            let style = if mono {
                TextStyle::new(11.5, 400.0)
                    .family(Family::SystemMono)
                    .lh(17.0)
            } else {
                widgets::body()
            };
            list = list.child(
                row()
                    .w_full()
                    .gap(12.0)
                    .items_start()
                    .child(text(name).w(92.0).font(widgets::body()).color(l.muted))
                    .child(
                        text(value)
                            .font(style)
                            .color(l.text)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0)
                            .selectable(true),
                    ),
            );
        }
        body = body.child(list);
        let mut links = row().gap(14.0).child(open(l, "Policy", &src.policy_url));
        if !src.data_url.is_empty() {
            links = links.child(open(l, "Data & API", &src.data_url));
        }
        cards.push(widgets::block(body.child(links)).key(format!("source-{}", src.id)));
    }
    sections.push(widgets::section(
        l,
        "open-access",
        "Open-Access Sources",
        Some("The programmes Galeria downloads from, and the field of each museum's own record that proves a work is open.".into()),
        vec![],
        cards,
    ));

    // Museums without a programme -----------------------------------------------------------
    let mut closed = Vec::new();
    for m in &rules.museums {
        let mut words = col().w_full().gap(4.0).child(
            row()
                .gap(8.0)
                .center_y()
                .child(text(m.name.clone()).font(widgets::strong()).color(l.text))
                .children(
                    m.license
                        .as_ref()
                        .map(|x| widgets::chip(l, x.clone(), Tone::Plain)),
                ),
        );
        words = words.child(widgets::hint(l, m.summary.clone()));
        if let Some(url) = &m.policy_url {
            words = words.child(open(l, "Its terms", url));
        }
        closed.push(el().w_full().px(16.0).py(12.0).child(words));
    }
    closed.push(
        el().w_full().px(16.0).py(12.0).child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    text("Any other institution")
                        .font(widgets::strong())
                        .color(l.text),
                )
                .child(widgets::hint(l, rules.fallback.summary.clone())),
        ),
    );
    sections.push(widgets::section(
        l,
        "closed",
        "Museums Without an Open-Access Programme",
        Some("Galeria never downloads from these. Their works are offered only as Wikimedia Commons files whose own license passes the rules above.".into()),
        vec![],
        closed,
    ));

    scroll_page("sources", sections)
}
