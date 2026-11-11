//! Checks against the real museum services. They need the network, so they
//! are ignored by default:
//!
//!   cargo test --lib live_ -- --ignored --nocapture --test-threads 1

use super::*;

fn ctx() -> Ctx {
    Ctx {
        http: Http::new(),
        cache: std::env::temp_dir().join("galeria-live-tests"),
        app: None,
        museums: vec![],
    }
}

fn query(source: &str, text: &str) -> Query {
    Query {
        source: source.into(),
        text: text.into(),
        landscape_only: true,
        min_long_edge: 3000,
        ..Default::default()
    }
}

fn run<F: Future>(f: F) -> F::Output {
    tauri::async_runtime::block_on(f)
}

/// Search, check every result is licensed and well-formed, then re-resolve the first.
async fn exercise(source: &str, text: &str) -> Vec<Candidate> {
    let ctx = ctx();
    let page = search(&ctx, &query(source, text))
        .await
        .unwrap_or_else(|e| panic!("{source} search failed: {e}"));
    println!(
        "\n{source} \"{text}\": {} shown, {} hidden, total {:?}, more: {}",
        page.items.len(),
        page.hidden,
        page.total,
        page.next.is_some()
    );
    assert!(
        !page.items.is_empty(),
        "{source} returned nothing for \"{text}\" ({:?})",
        page.notice
    );
    for c in &page.items {
        let d = c.decision.as_ref().expect("decision");
        println!(
            "  {:<22} {:<44.44} | {:<26.26} | {:>5}x{:<5} a={:<5} | {} | {:?}",
            c.id,
            c.title,
            c.artist.as_deref().unwrap_or("-"),
            c.width.unwrap_or(0),
            c.height.unwrap_or(0),
            c.aspect.unwrap_or(0.0),
            d.license,
            c.genres
        );
        assert!(c.id.starts_with(&format!("{source}:")));
        assert!(!c.title.is_empty() && !c.museum.is_empty());
        assert!(
            c.thumb_url
                .as_deref()
                .is_some_and(|u| u.starts_with("https://")),
            "{:?}",
            c.thumb_url
        );
        assert!(c
            .plan
            .as_ref()
            .is_some_and(|p| p.url.starts_with("https://")));
        if let Some(a) = c.aspect {
            assert!(a >= 1.15, "landscape filter let {a} through");
        }
    }
    let first = &page.items[0];
    let again = resolve(&ctx, source, &first.locator)
        .await
        .unwrap_or_else(|e| panic!("{source} resolve failed: {e}"));
    assert_eq!(again.id, first.id);
    let d = again.decision.as_ref().unwrap();
    println!(
        "  resolve {} → allowed={} {} [{}]\n    {}\n    {}",
        again.id,
        d.allowed,
        d.license,
        d.evidence,
        d.reason,
        again.plan.as_ref().unwrap().url
    );
    if let Some(next) = &page.next {
        let mut q = query(source, text);
        q.cursor = Some(next.clone());
        let p2 = search(&ctx, &q)
            .await
            .unwrap_or_else(|e| panic!("{source} page 2 failed: {e}"));
        println!("  page 2: {} shown", p2.items.len());
        if let (Some(a), Some(b)) = (page.items.first(), p2.items.first()) {
            assert_ne!(a.id, b.id, "page 2 repeats page 1");
        }
    }
    page.items
}

#[test]
#[ignore]
fn live_cma() {
    let items = run(exercise("cma", "landscape"));
    assert!(items.iter().all(|c| c.decision.as_ref().unwrap().allowed));
}

#[test]
#[ignore]
fn live_aic() {
    let items = run(exercise("aic", "monet"));
    assert!(items.iter().all(|c| c.decision.as_ref().unwrap().allowed));
}

/// A work still in copyright is found, shown as not open, and refused.
#[test]
#[ignore]
fn live_refuses_works_in_copyright() {
    // Picasso, "The Old Guitarist" (Art Institute of Chicago): the museum's record says is_public_domain = false.
    let c = run(resolve(&ctx(), "aic", "28067")).expect("record");
    let d = c.decision.expect("decision");
    println!(
        "\n{} → allowed={} {} [{}]\n  {}",
        c.title, d.allowed, d.license, d.evidence, d.reason
    );
    assert!(!d.allowed, "a copyrighted work must never be downloadable");
    // A Commons file under a NonCommercial license.
    let page = run(search(
        &ctx(),
        &Query {
            source: "commons".into(),
            text: "insource:\"cc-by-nc\" painting".into(),
            ..Default::default()
        },
    ))
    .unwrap();
    let refused: Vec<_> = page
        .items
        .iter()
        .filter(|c| !c.decision.as_ref().unwrap().allowed)
        .collect();
    for c in refused.iter().take(3) {
        let d = c.decision.as_ref().unwrap();
        println!(
            "  refused: {:<50.50} | {} | {}",
            c.title, d.license, d.reason
        );
    }
    assert!(page
        .items
        .iter()
        .all(|c| c.decision.as_ref().unwrap().allowed
            == matches!(
                c.decision.as_ref().unwrap().license_id.as_deref(),
                Some("cc0" | "pdm" | "cc-by" | "cc-by-sa")
            )));
}

#[test]
#[ignore]
fn live_met() {
    let items = run(exercise("met", "hudson river"));
    assert!(items.iter().all(|c| c.decision.as_ref().unwrap().allowed));
}

#[test]
#[ignore]
fn live_rijks() {
    run(exercise("rijks", "Ruisdael"));
    run(exercise("rijks", ""));
}

#[test]
#[ignore]
fn live_commons() {
    let items = run(exercise("commons", "Aivazovsky"));
    assert!(items.iter().any(|c| c.decision.as_ref().unwrap().allowed));
    let mut q = query("commons", "icon");
    q.museum = Some("Byzantine and Christian Museum, Athens".into());
    q.landscape_only = false;
    let page = run(search(&ctx(), &q)).unwrap();
    println!("\ncommons museum-scoped: {}", page.items.len());
    for c in page.items.iter().take(8) {
        let d = c.decision.as_ref().unwrap();
        println!(
            "  {:<60.60} | {} | allowed={} | {}",
            c.title,
            d.license,
            d.allowed,
            c.plan.as_ref().unwrap().credit
        );
    }
}

#[test]
#[ignore]
fn live_nga() {
    let ctx = ctx();
    if !nga::status(&ctx.cache).ready {
        let n = run(nga::prepare(&ctx)).expect("catalogue");
        println!("indexed {n} open-access paintings");
    }
    let st = nga::status(&ctx.cache);
    println!("catalogue: {st:?}");
    assert!(st.count > 2000, "{}", st.count);
    let items = run(exercise("nga", "cuyp"));
    assert!(items.iter().all(|c| c.decision.as_ref().unwrap().allowed));
    run(exercise("nga", ""));
}

/// The page-address rule, against addresses already recorded in a real manifest.
#[test]
#[ignore]
fn live_nga_page_urls_match_manifest() {
    let Ok(path) = std::env::var("GALERIA_MANIFEST") else {
        return;
    };
    let m: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let (mut ok, mut bad) = (0, 0);
    for e in m["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["source"] == "nga")
    {
        let (Some(id), Some(title), Some(page)) = (
            e["source_id"].as_str().and_then(|s| s.parse::<u32>().ok()),
            e["title"].as_str(),
            e["source_page"].as_str(),
        ) else {
            continue;
        };
        let mine = nga::page_url(id, title);
        if mine == page {
            ok += 1;
        } else {
            bad += 1;
            println!("  differs: {mine}\n       vs: {page}");
        }
    }
    println!("nga page urls: {ok} match, {bad} differ");
    assert!(ok > 0);
}
