//! The settings window, driven the way a person drives it: real clicks and keys
//! on the real view, through Rune's headless runtime.

use super::*;
use rune::{Env, Mods, Runtime};
use std::time::{Duration, Instant};

struct Bench {
    rt: Runtime<SettingsUi>,
    app: crate::host::AppHandle,
    dir: std::path::PathBuf,
}

impl Drop for Bench {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// A settings window over an empty, unconfigured core in a folder of its own.
fn bench(dark: bool, size: (f32, f32)) -> Bench {
    let dir = std::env::temp_dir().join(format!("galeria-settings-{}", fastrand::u64(..)));
    let app = crate::host::AppHandle::new(crate::core::Core::new(
        crate::core::Paths::in_dir(&dir).unwrap(),
    ));
    let snap = Arc::new(app.core().snapshot());
    let mut ui = SettingsUi::new(Ctx::headless(app.clone()), snap).immediate();
    // These tests are about the app itself; the first run has its own below.
    ui.onboarding = None;
    let rt = Runtime::new(ui, view, size, Env::new(dark, 2.0));
    let mut bench = Bench { rt, app, dir };
    bench.settle();
    bench
}

impl Bench {
    fn go(&mut self, tab: Tab) {
        self.rt.update(|s, _| s.go(tab));
        self.settle();
    }

    /// Let a page finish arriving: it fades in, and nothing transparent takes a press.
    fn settle(&mut self) {
        for _ in 0..60 {
            self.rt.frame(1.0 / 60.0);
        }
    }

    /// The centre of the first element matching `pick` below the element keyed `within`.
    fn inside(&self, within: &str, pick: impl Fn(&rune::ElementInfo) -> bool) -> (f32, f32) {
        let all = self.rt.elements();
        let root = all
            .iter()
            .find(|e| e.key.as_deref() == Some(within))
            .unwrap_or_else(|| panic!("no element keyed {within:?}"));
        let prefix = format!("{}/", root.path);
        let hit = all
            .iter()
            .find(|e| e.path.starts_with(&prefix) && pick(e))
            .unwrap_or_else(|| {
                let under: Vec<_> = all
                    .iter()
                    .filter(|e| e.path.starts_with(&prefix))
                    .map(|e| (&e.path, e.role, &e.key))
                    .collect();
                panic!("nothing matched inside {within:?}: {under:#?}")
            });
        let at = (
            hit.rect[0] + hit.rect[2] * 0.5,
            hit.rect[1] + hit.rect[3] * 0.5,
        );
        // Nothing else may be lying over it: a press there has to reach this control.
        let top = self
            .rt
            .element_at(at.0, at.1)
            .map(|p| p.path().to_string())
            .unwrap_or_default();
        assert!(
            top == hit.path || top.starts_with(&format!("{}/", hit.path)),
            "{} is covered by {top}",
            hit.path
        );
        at
    }

    fn click(&mut self, (x, y): (f32, f32)) {
        self.rt.pointer_move(x, y);
        self.rt.pointer_down(x, y);
        self.rt.pointer_up(x, y);
        self.rt.frame(0.0);
    }

    /// Settings are written off the UI thread; wait for the core to show the change.
    fn until(&self, what: &str, done: impl Fn(&crate::settings::Settings) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if done(&self.app.core().lock().settings) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the core never saw: {what}");
    }

    /// Hand the window the core's latest snapshot, as the shell does.
    fn sync(&mut self) {
        let snap = Arc::new(self.app.core().snapshot());
        self.rt.update(|s, tx| s.apply(snap, tx));
        self.settle();
    }
}

#[test]
fn every_page_lays_out_at_every_size_in_both_appearances() {
    for dark in [true, false] {
        for size in [(700.0, 480.0), (1080.0, 720.0), (1800.0, 1200.0)] {
            let mut b = bench(dark, size);
            for (tab, key, title, _) in Tab::ALL {
                b.go(tab);
                let (x, y) =
                    b.rt.find("page")
                        .unwrap_or_else(|| panic!("{key}: no page at {size:?}"));
                assert!(
                    x > 0.0 && x < size.0 && y > 0.0 && y < size.1,
                    "{key}: page centre {x},{y} outside {size:?}"
                );
                assert!(
                    b.rt.elements()
                        .iter()
                        .any(|e| e.text.as_deref() == Some(title)),
                    "{key}: the toolbar does not name the page"
                );
            }
        }
    }
}

#[test]
fn the_sidebar_and_command_digits_switch_pages() {
    let mut b = bench(true, (1080.0, 720.0));
    assert!(
        b.rt.state.tab == Tab::General,
        "opens on General, the first page"
    );
    for (tab, key, _, _) in Tab::ALL.into_iter().rev() {
        let at =
            b.rt.find(key)
                .unwrap_or_else(|| panic!("no sidebar row for {key}"));
        b.click(at);
        assert!(b.rt.state.tab == tab, "clicking the {key} row opens it");
    }
    for (i, (tab, key, _, _)) in Tab::ALL.into_iter().enumerate() {
        let digit = char::from_digit(i as u32 + 1, 10).unwrap();
        let ev = KeyEvent {
            key: Key::Command(digit),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        };
        b.rt.update(|s, tx| {
            on_key(s, &ev, tx);
        });
        b.rt.frame(0.0);
        assert!(b.rt.state.tab == tab, "⌘{digit} opens {key}");
    }
}

#[test]
fn clicking_a_switch_changes_the_setting_and_the_switch_follows() {
    let mut b = bench(true, (1080.0, 720.0));
    b.go(Tab::Wallpaper);
    let before = b.app.core().lock().settings.wallpaper.pan;
    let at = b.inside("pan", |e| e.role == "switch");
    b.click(at);
    b.until("pan flipped", |s| s.wallpaper.pan != before);
    b.sync();
    assert_eq!(
        b.rt.state.snap.settings.wallpaper.pan, !before,
        "the window shows the new value"
    );
    // And back again, from the redrawn control.
    let at = b.inside("pan", |e| e.role == "switch");
    b.click(at);
    b.until("pan restored", |s| s.wallpaper.pan == before);
}

#[test]
fn picking_a_segment_changes_the_fit_mode() {
    use crate::settings::FitMode;
    let mut b = bench(false, (1080.0, 720.0));
    b.go(Tab::Wallpaper);
    assert!(b.app.core().lock().settings.wallpaper.fit == FitMode::Auto);
    let at = b.inside("mode", |e| e.key.as_deref() == Some("segment2"));
    b.click(at);
    b.until("fit = fill", |s| s.wallpaper.fit == FitMode::Fill);
    b.sync();
    // "Fill when the crop is under" only belongs to Automatic.
    assert!(
        b.rt.find("crop").is_none(),
        "the Automatic-only row leaves with the mode"
    );
}

#[test]
fn the_tick_on_a_discover_result_selects_it_without_opening_its_details() {
    use crate::licensing::Decision;
    use crate::sources::Candidate;
    let mut b = bench(true, (1080.0, 720.0));
    // The page only lists results once there is a library folder to save them to.
    b.rt.update(|s, _| {
        let mut snap = (*s.snap).clone();
        snap.library.root = Some(std::env::temp_dir());
        s.snap = Arc::new(snap);
        s.discover.started = true;
    });
    let decision: Decision = serde_json::from_value(serde_json::json!({
        "allowed": true, "licenseId": "cc0", "license": "CC0", "licenseUrl": null,
        "attributionRequired": false, "shareAlike": false, "obligations": [],
        "reason": "", "evidence": "", "program": "", "policyUrl": "",
    }))
    .expect("a decision");
    b.rt.update(|s, _| {
        s.go(Tab::Discover);
        s.discover.items = ["a", "b"]
            .into_iter()
            .map(|n| Candidate {
                id: format!("t:{n}"),
                source: "t".into(),
                source_id: n.into(),
                title: format!("Painting {n}"),
                decision: Some(decision.clone()),
                ..Default::default()
            })
            .collect();
    });
    b.settle();
    // The tick is the small round control in the tile's top-right corner.
    let at = b.inside("cand-t:a", |e| e.role == "checkbox");
    b.click(at);
    let d = &b.rt.state.discover;
    assert!(d.selected.contains("t:a"), "the tick selects");
    assert!(d.focus.is_none(), "and leaves the details closed");
    b.click(at);
    assert!(
        b.rt.state.discover.selected.is_empty(),
        "the tick deselects"
    );
    // The rest of the tile still opens the details.
    let (x, y) = b.rt.find("cand-t:b").expect("tile");
    b.click((x, y + 20.0));
    assert_eq!(b.rt.state.discover.focus.as_deref(), Some("t:b"));
}

#[test]
fn the_x_in_the_browse_search_clears_it() {
    let mut b = bench(true, (1080.0, 720.0));
    b.go(Tab::Browse);
    b.rt.update(|s, _| s.browse.query = rune::TextEdit::new("monet"));
    b.settle();
    let at = b.inside("page", |e| e.key.as_deref() == Some("action"));
    b.click(at);
    assert_eq!(
        b.rt.state.browse.query.text, "",
        "the search is empty again"
    );
}

/// A settings window as someone sees it the first time: no art folder yet.
fn first_run() -> Bench {
    let dir = std::env::temp_dir().join(format!("galeria-first-run-{}", fastrand::u64(..)));
    let app = crate::host::AppHandle::new(crate::core::Core::new(
        crate::core::Paths::in_dir(&dir).unwrap(),
    ));
    let snap = Arc::new(app.core().snapshot());
    let ui = SettingsUi::new(Ctx::headless(app.clone()), snap).immediate();
    let rt = Runtime::new(ui, view, (1180.0, 800.0), Env::new(true, 2.0));
    let mut bench = Bench { rt, app, dir };
    bench.settle();
    bench
}

fn step(b: &Bench) -> Option<onboarding::Step> {
    b.rt.state.onboarding.as_ref().map(|o| o.step)
}

#[test]
fn the_first_run_walks_from_the_welcome_to_a_folder_with_a_new_manifest() {
    use onboarding::Step;
    let mut b = first_run();
    assert_eq!(
        step(&b),
        Some(Step::Welcome),
        "no art folder: the first run"
    );
    assert!(b.rt.find("now").is_none(), "the app's pages wait behind it");
    for expect in [Step::Desktop, Step::Folder] {
        let at = b.rt.find("next").expect("Continue");
        b.click(at);
        b.settle();
        assert_eq!(step(&b), Some(expect));
    }
    // Nothing chosen: Continue does nothing.
    let at = b.rt.find("next").expect("Continue");
    b.click(at);
    assert_eq!(step(&b), Some(Step::Folder));
    assert!(b.app.core().lock().settings.library_path.is_none());

    // A folder picked (the system picker cannot run here), then Continue.
    let folder = b.dir.join("My Paintings");
    std::fs::create_dir_all(&folder).unwrap();
    let picked = folder.clone();
    b.rt.update(move |s, _| {
        s.onboarding.as_mut().unwrap().folder = Some(picked);
    });
    b.settle();
    let at = b.rt.find("next").expect("Continue");
    b.click(at);
    b.settle();
    assert_eq!(step(&b), Some(Step::Paintings));
    assert_eq!(
        b.app.core().lock().settings.library_path.as_deref(),
        Some(folder.as_path()),
        "the folder is the art folder now"
    );
    let manifest =
        std::fs::read_to_string(folder.join("manifest.json")).expect("a manifest was started");
    let value: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(value["items"], serde_json::json!([]));
    let report = b.app.core().snapshot().library;
    assert!(
        report.manifest_found && report.error.is_none(),
        "{:?}",
        report.error
    );

    // Skipping the downloads opens the app.
    let at = b.rt.find("skip").expect("Skip for Now");
    b.click(at);
    b.settle();
    assert!(b.rt.state.onboarding.is_none());
    assert!(b.rt.state.tab == Tab::Now);
}

#[test]
fn the_first_run_leaves_a_folder_that_has_a_manifest_as_it_is() {
    let mut b = first_run();
    let folder = b.dir.join("Existing");
    std::fs::create_dir_all(&folder).unwrap();
    let theirs = "[\n\t{ \"file\": \"a.jpg\", \"title\": \"Mine\" }\n]\n";
    std::fs::write(folder.join("manifest.json"), theirs).unwrap();
    let picked = folder.clone();
    b.rt.update(move |s, _| {
        let o = s.onboarding.as_mut().unwrap();
        o.step = onboarding::Step::Folder;
        o.folder = Some(picked);
    });
    b.settle();
    let at = b.rt.find("next").expect("Continue");
    b.click(at);
    b.settle();
    assert_eq!(step(&b), Some(onboarding::Step::Paintings));
    assert_eq!(
        std::fs::read_to_string(folder.join("manifest.json")).unwrap(),
        theirs
    );
}

#[test]
fn a_dozen_results_are_ticked_to_start_with_and_only_on_the_first_run() {
    use crate::licensing::Decision;
    use crate::sources::Candidate;
    let decision: Decision = serde_json::from_value(serde_json::json!({
        "allowed": true, "licenseId": "cc0", "license": "CC0", "licenseUrl": null,
        "attributionRequired": false, "shareAlike": false, "obligations": [],
        "reason": "", "evidence": "", "program": "", "policyUrl": "",
    }))
    .unwrap();
    let found = |n: usize| -> Vec<Candidate> {
        (0..n)
            .map(|i| Candidate {
                id: format!("t:{i}"),
                source: "t".into(),
                source_id: i.to_string(),
                title: format!("Painting {i}"),
                // Every fifth is not open: it can never be ticked.
                decision: (i % 5 != 4).then(|| decision.clone()),
                ..Default::default()
            })
            .collect()
    };
    let mut b = first_run();
    b.rt.update(|s, _| {
        s.onboarding.as_mut().unwrap().step = onboarding::Step::Paintings;
        s.discover.items = found(30);
        s.onboarding_results();
    });
    let d = &b.rt.state.discover;
    assert_eq!(d.selected.len(), 12);
    assert!(
        !d.selected.contains("t:4"),
        "only paintings that may be downloaded"
    );
    // In the app itself a search ticks nothing.
    b.rt.update(|s, _| {
        s.onboard_finish();
        s.discover.selected.clear();
        s.onboarding_results();
    });
    assert!(b.rt.state.discover.selected.is_empty());
}

#[test]
fn someone_with_an_art_folder_never_sees_the_first_run() {
    let dir = std::env::temp_dir().join(format!("galeria-returning-{}", fastrand::u64(..)));
    let app = crate::host::AppHandle::new(crate::core::Core::new(
        crate::core::Paths::in_dir(&dir).unwrap(),
    ));
    let mut snap = app.core().snapshot();
    snap.library.root = Some(dir.clone());
    let ui = SettingsUi::new(Ctx::headless(app), Arc::new(snap));
    assert!(ui.onboarding.is_none());
    std::fs::remove_dir_all(&dir).ok();
}

/// The Paintings step with three shelves already read: Cleveland landscapes and
/// winters, and the Met's landscapes. Nothing is ticked.
fn shelves() -> Bench {
    use crate::licensing::Decision;
    use crate::sources::Candidate;
    let decision: Decision = serde_json::from_value(serde_json::json!({
        "allowed": true, "licenseId": "cc0", "license": "CC0", "licenseUrl": null,
        "attributionRequired": false, "shareAlike": false, "obligations": [],
        "reason": "", "evidence": "", "program": "", "policyUrl": "",
    }))
    .unwrap();
    let shelf = |museum: &str, theme: &str, n: usize| -> Vec<Candidate> {
        (0..n)
            .map(|i| Candidate {
                id: format!("{museum}:{theme}{i}"),
                source: museum.into(),
                source_id: format!("{theme}{i}"),
                locator: format!("{theme}{i}"),
                title: format!("{theme} {i}"),
                decision: Some(decision.clone()),
                ..Default::default()
            })
            .collect()
    };
    let mut b = first_run();
    b.rt.update(move |s, _| {
        let o = s.onboarding.as_mut().unwrap();
        o.step = onboarding::Step::Paintings;
        o.started = true;
        for (museum, theme, n) in [
            ("cma", "landscape", 5),
            ("cma", "winter", 4),
            ("met", "landscape", 3),
        ] {
            o.results
                .insert((museum.into(), theme.into()), shelf(museum, theme, n));
        }
        s.onboard_show("cma", "landscape");
    });
    b.settle();
    b
}

fn ticked(b: &Bench) -> Vec<String> {
    b.rt.state.discover.selected.iter().cloned().collect()
}

#[test]
fn ticks_stay_when_looking_at_another_museum_or_theme() {
    let mut b = shelves();
    assert!(ticked(&b).is_empty());
    // Two landscapes at Cleveland, by their ticks.
    for id in ["cand-cma:landscape0", "cand-cma:landscape3"] {
        let at = b.inside(id, |e| e.role == "checkbox");
        b.click(at);
    }
    // Over to the winters (a press on the theme's name), and one of those.
    let at = b.inside("theme-winter", |e| e.role == "text");
    b.click(at);
    b.settle();
    assert_eq!(
        b.rt.state.discover.items.len(),
        4,
        "the winters are on show"
    );
    assert!(ticked(&b).is_empty(), "nothing ticked here yet");
    let at = b.inside("cand-cma:winter1", |e| e.role == "checkbox");
    b.click(at);
    // Then another museum, and back.
    let at = b.inside("museum-met", |e| e.role == "text");
    b.click(at);
    b.settle();
    assert_eq!(b.rt.state.discover.source, "met");
    assert!(ticked(&b).is_empty());
    assert_eq!(
        b.rt.state.onboard_tally(),
        (3, 0),
        "all three are still wanted"
    );
    b.rt.update(|s, _| s.onboard_show("cma", "landscape"));
    assert_eq!(ticked(&b), ["cma:landscape0", "cma:landscape3"]);
    b.rt.update(|s, _| s.onboard_show("cma", "winter"));
    assert_eq!(ticked(&b), ["cma:winter1"]);
}

#[test]
fn a_theme_or_a_museum_can_be_taken_whole() {
    use onboarding::Chosen;
    let mut b = shelves();
    // The box on Winter takes every winter at Cleveland, without leaving the landscapes.
    let at = b.rt.find("all-theme-winter").expect("Winter's box");
    b.click(at);
    b.settle();
    assert_eq!(
        b.rt.state.shelf().1,
        "landscape",
        "still looking at landscapes"
    );
    assert_eq!(b.rt.state.onboard_tally(), (4, 0));
    assert_eq!(b.rt.state.onboard_theme("winter"), Chosen::All);
    assert_eq!(b.rt.state.onboard_museum("cma"), Chosen::Some);
    // Looking at it shows all four ticked; one can still be left out.
    b.rt.update(|s, _| s.onboard_show("cma", "winter"));
    b.settle();
    assert_eq!(ticked(&b).len(), 4);
    let at = b.inside("cand-cma:winter2", |e| e.role == "checkbox");
    b.click(at);
    assert_eq!(b.rt.state.onboard_tally(), (3, 0));
    assert_eq!(b.rt.state.onboard_theme("winter"), Chosen::Some);
    b.rt.update(|s, _| s.onboard_show("cma", "landscape"));
    assert_eq!(b.rt.state.onboard_tally(), (3, 0), "it stays left out");

    // The box on the Met takes every theme there: one shelf is known, the rest are not read yet.
    let at = b.rt.find("all-met").expect("the Met's box");
    b.click(at);
    b.settle();
    let themes = crate::genres::all().len();
    assert_eq!(b.rt.state.onboard_museum("met"), Chosen::All);
    assert_eq!(b.rt.state.onboard_tally(), (3 + 3, themes - 1));
    // And lets go of all of it again.
    let at = b.rt.find("all-met").expect("the Met's box");
    b.click(at);
    b.settle();
    assert_eq!(b.rt.state.onboard_museum("met"), Chosen::None);
    assert_eq!(b.rt.state.onboard_tally(), (3, 0));
}

#[test]
fn download_sends_for_everything_ticked_anywhere() {
    let mut b = shelves();
    let at = b.inside("cand-cma:landscape2", |e| e.role == "checkbox");
    b.click(at);
    let at = b.rt.find("all-theme-winter").expect("Winter's box");
    b.click(at);
    b.rt.update(|s, _| s.onboard_show("met", "landscape"));
    b.settle();
    let at = b.inside("cand-met:landscape1", |e| e.role == "checkbox");
    b.click(at);
    b.settle();
    let at = b.rt.find("next").expect("Download");
    b.click(at);
    assert_eq!(step(&b), Some(onboarding::Step::Hanging));
    assert_eq!(b.rt.state.onboarding.as_ref().unwrap().asked, 6);
    let start = Instant::now();
    let mut ids: Vec<String> = Vec::new();
    while start.elapsed() < Duration::from_secs(5) && ids.len() < 6 {
        ids = b
            .app
            .core()
            .downloads
            .list()
            .into_iter()
            .map(|j| j.id)
            .collect();
        std::thread::sleep(Duration::from_millis(10));
    }
    ids.sort();
    assert_eq!(
        ids,
        [
            "cma:landscape2",
            "cma:winter0",
            "cma:winter1",
            "cma:winter2",
            "cma:winter3",
            "met:landscape1"
        ]
    );
}

#[test]
fn deleting_single_batch_and_museum_removes_files_manifest_and_advances_desktop() {
    use crate::displays::{DisplayInfo, Insets};
    let mut b = bench(true, (1180.0, 800.0));
    let lib = b.dir.join("library");
    std::fs::create_dir_all(&lib).unwrap();
    for name in ["a.png", "b.png", "c.png"] {
        image::RgbImage::from_pixel(16, 12, image::Rgb([80, 90, 100]))
            .save(lib.join(name))
            .unwrap();
    }
    let manifest = serde_json::json!({
        "items": [
            { "id": "cma:1", "file": "a.png", "title": "First", "museum": "Cleveland Museum of Art, Cleveland", "tags": ["landscape"] },
            { "id": "cma:2", "file": "b.png", "title": "Second", "museum": "Cleveland Museum of Art, Cleveland", "tags": ["winter"] },
            { "id": "met:3", "file": "c.png", "title": "Third", "museum": "The Metropolitan Museum of Art", "tags": ["landscape"] }
        ]
    });
    std::fs::write(
        lib.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    // Seed downloads.json too, to verify heal() does not resurrect deleted paintings.
    std::fs::write(
        &b.app.core().paths.downloads_journal,
        serde_json::to_string(&vec![serde_json::json!({
            "id": "cma:1", "file": "a.png", "path": lib.join("a.png"), "title": "First"
        })])
        .unwrap(),
    )
    .unwrap();

    b.app.set_fixed_displays(vec![DisplayInfo {
        key: "Main".into(),
        name: "Main".into(),
        label: String::new(),
        x: 0.0,
        y: 0.0,
        width: 1440.0,
        height: 900.0,
        physical_x: 0,
        physical_y: 0,
        physical_width: 2880,
        physical_height: 1800,
        scale: 2.0,
        primary: true,
        insets: Insets::default(),
    }]);
    crate::core::refresh_displays(&b.app, true);
    crate::actions::set_library_path(&b.app, Some(lib.clone())).unwrap();
    crate::core::show(&b.app, "cma:1", None).unwrap();
    crate::core::set_favorite(&b.app, "cma:1", true);
    b.sync();

    // 1. Browse batch delete: select cma:1, click Delete…, cancel first, then confirm.
    b.go(Tab::Browse);
    b.rt.update(|s, _| {
        s.browse.picking = true;
        s.browse.picked.insert("cma:1".into());
    });
    b.settle();
    let del = b.rt.find("bulk-delete").expect("Delete…");
    b.click(del);
    b.settle();
    assert!(b.rt.find("bulk-confirm").is_some());
    let no = b.rt.find("bulk-confirm-no").expect("Cancel");
    b.click(no);
    b.settle();
    assert!(b.rt.find("bulk-confirm").is_none());
    assert!(lib.join("a.png").exists(), "Cancel leaves the file alone");

    let del = b.rt.find("bulk-delete").expect("Delete…");
    b.click(del);
    b.settle();
    let yes = b.rt.find("bulk-confirm-yes").expect("Delete 1 Painting");
    b.click(yes);

    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) && lib.join("a.png").exists() {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!lib.join("a.png").exists(), "file deleted from disk");
    let wait = Instant::now();
    while wait.elapsed() < Duration::from_secs(5) && b.app.core().snapshot().library.valid != 2 {
        std::thread::sleep(Duration::from_millis(10));
    }
    let snap = b.app.core().snapshot();
    assert_eq!(snap.library.valid, 2);
    assert!(!snap.settings.favorites.contains(&"cma:1".to_string()));
    assert_ne!(
        snap.displays[0].painting.as_ref().map(|p| p.id.as_str()),
        Some("cma:1"),
        "desktop moved off the deleted painting"
    );

    // 2. Whole-museum delete from Museums page removes the remaining Cleveland painting.
    b.sync();
    b.go(Tab::Sources);
    let del = b.inside("museum-Cleveland Museum of Art, Cleveland", |e| {
        e.label.as_deref() == Some("Delete…")
    });
    b.click(del);
    b.settle();
    let yes =
        b.rt.find("confirm-delete-museum-yes")
            .expect("Delete museum");
    b.click(yes);
    let wait = Instant::now();
    while wait.elapsed() < Duration::from_secs(5) && b.app.core().snapshot().library.valid != 1 {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!lib.join("b.png").exists());
    assert!(lib.join("c.png").exists(), "other museums stay untouched");
    assert_eq!(b.app.core().snapshot().library.valid, 1);
}

#[test]
fn onboarding_an_existing_library_reflects_manifest_paintings_and_does_not_retick_them() {
    use crate::licensing::Decision;
    use crate::sources::Candidate;
    use onboarding::Chosen;

    let mut b = first_run();
    let folder = b.dir.join("ExistingGaleria");
    std::fs::create_dir_all(&folder).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbImage::from_pixel(16, 12, image::Rgb([110, 90, 70]))
            .save(folder.join(name))
            .unwrap();
    }
    let manifest = serde_json::json!({
        "items": [
            { "id": "cma:1", "file": "a.png", "title": "Already Here", "source": "cma", "source_id": "1", "tags": ["landscape"] },
            { "id": "cma:2", "file": "b.png", "title": "Winter Here", "source": "cma", "source_id": "2", "tags": ["winter"] }
        ]
    });
    std::fs::write(
        folder.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let picked = folder.clone();
    b.rt.update(move |s, _| {
        let o = s.onboarding.as_mut().unwrap();
        o.step = onboarding::Step::Folder;
        o.folder = Some(picked);
    });
    b.settle();
    let next = b.rt.find("next").expect("Continue");
    b.click(next);
    b.settle();

    assert_eq!(step(&b), Some(onboarding::Step::Paintings));
    assert_eq!(b.rt.state.manifest_index.total, 2);
    assert_eq!(b.rt.state.manifest_index.by_source.get("cma"), Some(&2));
    assert_eq!(
        b.rt.state
            .manifest_index
            .by_shelf
            .get(&("cma".into(), "landscape".into())),
        Some(&1)
    );

    let decision: Decision = serde_json::from_value(serde_json::json!({
        "allowed": true, "licenseId": "cc0", "license": "CC0", "licenseUrl": null,
        "attributionRequired": false, "shareAlike": false, "obligations": [],
        "reason": "", "evidence": "", "program": "", "policyUrl": "",
    }))
    .unwrap();
    b.rt.update(move |s, _| {
        // cma:1 is already in manifest.json; cma:99 is new; winter shelf has only cma:2 (all owned).
        s.discover.items = vec![
            Candidate {
                id: "cma:1".into(),
                source: "cma".into(),
                source_id: "1".into(),
                title: "Already Here".into(),
                decision: Some(decision.clone()),
                ..Default::default()
            },
            Candidate {
                id: "cma:99".into(),
                source: "cma".into(),
                source_id: "99".into(),
                title: "Brand New".into(),
                decision: Some(decision.clone()),
                ..Default::default()
            },
        ];
        s.onboarding_results();
        s.onboarding.as_mut().unwrap().results.insert(
            ("cma".into(), "winter".into()),
            vec![Candidate {
                id: "cma:2".into(),
                source: "cma".into(),
                source_id: "2".into(),
                title: "Winter Here".into(),
                decision: Some(decision),
                ..Default::default()
            }],
        );
    });
    b.settle();

    assert!(
        b.rt.state.saved(&b.rt.state.discover.items[0]),
        "cma:1 recognized as already in library"
    );
    assert!(
        !b.rt.state.selectable(&b.rt.state.discover.items[0]),
        "cma:1 cannot be ticked again"
    );
    assert!(
        b.rt.state.discover.selected.is_empty(),
        "nothing auto-ticked when library already has paintings"
    );
    assert_eq!(
        b.rt.state.onboard_theme("winter"),
        Chosen::Owned,
        "shelf whose paintings are all in manifest.json shows Owned"
    );

    // With 0 new paintings ticked and an existing library, primary button opens Galeria directly.
    let open = b.rt.find("next").expect("Open Galeria");
    b.click(open);
    b.settle();
    assert!(b.rt.state.onboarding.is_none());
}
