//! Library: the art folder, what is wrong in it, and what Galeria caches.

use super::widgets::{self, Kind, Tone};
use super::{scroll_page, SettingsUi};
use crate::actions;
use crate::core;
use crate::library::Issue;
use crate::ui::theme::Look;
use rune::tok::Family;
use rune::{col, el, row, text, El, TextStyle};
use serde_json::json;

#[derive(Default)]
pub struct State {
    /// What the last action on this page did ("Freed 12 MB").
    pub notice: Option<String>,
}

const MANIFEST_EXAMPLE: &str = r#"{
  "items": [{
    "file": "van-gogh__wheat-field.jpg",      // relative to this folder
    "title": "Wheat Field with Cypresses",
    "artist": "Vincent van Gogh",
    "artist_bio": "Dutch, 1853–1890",
    "date": "1889",
    "medium": "oil on canvas",
    "dimensions": "73 × 93.4 cm",
    "description": "Painted in Saint-Rémy…",
    "museum": "The Metropolitan Museum of Art",
    "credit_line": "Purchase, The Annenberg Foundation Gift, 1993",
    "credit": "Image: The Met (CC0)",
    "license": "CC0",
    "source": "met", "source_id": "436535",
    "source_page": "https://www.metmuseum.org/art/collection/search/436535",
    "tags": ["landscape"]
  }]
}"#;

fn mono(size: f32) -> TextStyle {
    TextStyle::new(size, 400.0)
        .family(Family::SystemMono)
        .lh((size * 1.5).round())
}

fn issue_list(l: &Look, issues: &[&Issue], more: usize) -> El {
    let mut c = col().w_full().gap(6.0);
    for i in issues {
        c = c.child(
            row()
                .w_full()
                .gap(8.0)
                .items_start()
                .child(
                    text(i.file.clone())
                        .font(mono(11.5))
                        .color(l.text)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0)
                        .selectable(true),
                )
                .children((!i.problem.is_empty()).then(|| {
                    text(i.problem.clone())
                        .font(widgets::small())
                        .color(l.muted)
                        .wrap()
                        .w(260.0)
                })),
        );
    }
    if more > 0 {
        c = c.child(widgets::hint(l, format!("…and {more} more")));
    }
    widgets::block(c)
}

pub fn page(s: &SettingsUi, l: &Look, _: &mut Vec<El>) -> El {
    let snap = &s.snap;
    let lib = &snap.library;
    let mut sections = Vec::new();

    // Art folder --------------------------------------------------------------
    let has_root = lib.root.is_some();
    let mut folder_actions = row().gap(8.0).child(widgets::button(
        l,
        if has_root { "Change…" } else { "Choose…" },
        if has_root {
            Kind::Normal
        } else {
            Kind::Primary
        },
        |s, tx| s.choose_folder(tx),
    ));
    if has_root {
        folder_actions = folder_actions
            .child(widgets::button(l, "Open Folder", Kind::Normal, |s, _| {
                if let Some(root) = s.snap.library.root.clone() {
                    crate::open::path(&root);
                }
            }))
            .child(widgets::button(l, "Rescan", Kind::Normal, |s, _| {
                s.cx.run(core::reload_library)
            }));
    }
    let mut folder = vec![widgets::setting(
        l,
        "folder",
        lib.root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "No folder chosen".into()),
        lib.generated
            .as_ref()
            .map(|g| format!("Manifest generated {g}")),
        Some(folder_actions),
    )];
    if let Some(e) = &lib.error {
        folder.push(widgets::block(widgets::notice(l, e.clone(), true)));
    }
    if has_root && !lib.manifest_found {
        // A folder Galeria has not used before: offer to start its index.
        folder.push(widgets::setting(
            l,
            "create-manifest",
            "Start a manifest here",
            Some("Galeria lists its paintings in manifest.json. This creates an empty one; downloads from Discover are added to it.".into()),
            Some(widgets::button(l, "Create manifest.json", Kind::Primary, |s, _| {
                if let Some(root) = s.snap.library.root.clone() {
                    s.cx.run(move |app| {
                        if let Err(e) = actions::create_manifest(app, &root) {
                            log::warn!("manifest: {e}");
                        }
                    });
                }
            })),
        ));
    }
    let warn = |n: usize| if n > 0 { Tone::Bad } else { Tone::Plain };
    folder.push(widgets::block(
        col()
            .w_full()
            .gap(12.0)
            .child(widgets::wrap(vec![
                widgets::stat(l, lib.entries, "manifest entries", Tone::Plain),
                widgets::stat(l, lib.valid, "ready", Tone::Good),
                widgets::stat(l, lib.missing, "missing files", warn(lib.missing)),
                widgets::stat(l, lib.invalid, "invalid", warn(lib.invalid)),
                widgets::stat(l, lib.duplicates, "duplicates", warn(lib.duplicates)),
                widgets::stat(l, lib.unindexed, "files not in manifest", Tone::Plain),
                widgets::stat(l, lib.excluded, "hidden by you", Tone::Plain),
            ]))
            .child(widgets::hint(
                l,
                "The folder is watched: when the manifest changes or files finish downloading, the library updates by itself. Galeria writes to manifest.json only to add a painting you download in Discover (or to remove missing entries when you ask), and keeps a backup each time.",
            )),
    ));
    sections.push(widgets::section(
        l,
        "folder",
        "Art Folder",
        Some("Galeria never bundles or copies paintings. It reads them from this folder.".into()),
        vec![],
        folder,
    ));

    // Issues ------------------------------------------------------------------
    if let Some(view) = &s.library {
        let issues: Vec<&Issue> = view
            .missing
            .iter()
            .chain(&view.invalid)
            .chain(&view.duplicates)
            .collect();
        if !issues.is_empty() {
            let mut actions = Vec::new();
            if !view.missing.is_empty() {
                actions.push(
                    widgets::button(
                        l,
                        format!("Remove {} Missing from Manifest", view.missing.len()),
                        Kind::Normal,
                        |s, _| {
                            let handle = s.handle.clone();
                            s.cx.run(move |app| {
                                let said = match actions::manifest_cleanup(app) {
                                    Ok(n) => format!("Removed {}", widgets::plural(n, "manifest entry", "manifest entries")),
                                    Err(e) => e,
                                };
                                if let Some(handle) = handle {
                                    handle.update(move |ui: &mut SettingsUi, _| ui.library_page.notice = Some(said));
                                }
                            });
                        },
                    )
                    .help("Removes the entries of deleted files from manifest.json. A backup of the manifest is kept."),
                );
            }
            sections.push(widgets::section(
                l,
                "issues",
                "Issues",
                None,
                actions,
                vec![issue_list(l, &issues, 0)],
            ));
        }
        if !view.unindexed.is_empty() {
            let files: Vec<Issue> = view
                .unindexed
                .iter()
                .map(|f| Issue {
                    file: f.clone(),
                    problem: String::new(),
                })
                .collect();
            let refs: Vec<&Issue> = files.iter().collect();
            sections.push(widgets::section(
                l,
                "unindexed",
                "Files Not in the Manifest",
                Some("These images are in the folder but won't be shown until the manifest lists them.".into()),
                vec![],
                vec![issue_list(l, &refs, lib.unindexed.saturating_sub(view.unindexed.len()))],
            ));
        }
    }

    // Museum information ----------------------------------------------------------
    let enrich = snap.settings.general.enrich_online;
    sections.push(widgets::section(
        l,
        "information",
        "Museum Information",
        Some("Fills in descriptions, artist nationality and dates, credit lines and English titles from the Cleveland Museum of Art and Rijksmuseum APIs, Wikidata and Wikipedia. Cached on this computer, not written to your manifest.".into()),
        vec![],
        vec![
            widgets::setting(
                l,
                "enrich",
                "Look up information online",
                None,
                Some(widgets::toggle(l, enrich, move |s| {
                    s.cx.patch(json!({ "general": { "enrichOnline": !enrich } }))
                })),
            ),
            widgets::setting(
                l,
                "progress",
                "Progress",
                lib.enrich_busy.then(|| "Working through the library in the background…".to_string()),
                Some(text(format!("{} of {} looked up", lib.enriched, lib.valid)).font(widgets::body()).color(l.muted)),
            ),
            widgets::setting(
                l,
                "again",
                "Start over",
                Some("Forget everything fetched and look it up again.".into()),
                Some(widgets::button(l, "Refresh All", Kind::Normal, |s, _| {
                    s.cx.run(|app| app.core().refresh_enrichment(None))
                })),
            ),
        ],
    ));

    // Cache ---------------------------------------------------------------------
    sections.push(widgets::section(
        l,
        "cache",
        "Cache",
        None,
        vec![],
        vec![widgets::setting(
            l,
            "images",
            "Thumbnails and display-sized renditions",
            Some(
                s.library_page
                    .notice
                    .clone()
                    .unwrap_or_else(|| "Rebuilt automatically when needed.".into()),
            ),
            Some(widgets::button(l, "Clear Cache", Kind::Normal, |s, _| {
                let handle = s.handle.clone();
                s.cx.art.clear();
                s.cx.run(move |app| {
                    let freed = actions::clear_image_cache(app.core());
                    if let Some(handle) = handle {
                        handle.update(move |ui: &mut SettingsUi, _| {
                            ui.library_page.notice =
                                Some(format!("Freed {}", widgets::bytes(freed)));
                        });
                    }
                });
            })),
        )],
    ));

    // Manifest format --------------------------------------------------------------
    sections.push(widgets::section(
        l,
        "format",
        "Manifest Format",
        None,
        vec![],
        vec![widgets::block(
            col()
                .w_full()
                .gap(10.0)
                .child(widgets::hint(
                    l,
                    "manifest.json is an array (or { \"items\": [...] }) of entries. Only \"file\" is required; everything else appears on the plate when present.",
                ))
                .child(
                    el().w_full().padding(14.0).radius(l.kit.radius.md).fill(l.well).child(
                        text(MANIFEST_EXAMPLE).font(mono(11.5)).color(l.text.alpha(0.86)).selectable(true),
                    ),
                ),
        )],
    ));

    scroll_page("library", sections)
}
