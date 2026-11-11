//! Rijksmuseum — Data Services (https://data.rijksmuseum.nl/docs/).
//!
//! Search returns bare identifiers; each one is then read as an EDM record
//! from the OAI-PMH endpoint, which holds the title, maker, rights statement
//! and image address in a single response.

use super::*;
use crate::licensing::{self, Evidence};
use std::collections::HashMap;

const SEARCH: &str =
    "https://data.rijksmuseum.nl/search/collection?type=painting&imageAvailable=true";
const OAI: &str = "https://data.rijksmuseum.nl/oai";
const LD: [(&str, &str); 1] = [("Accept", "application/ld+json")];
/// Larger requests can exceed 100 MB; this is plenty for an 8K display.
const MAX_EDGE: u32 = 6500;
const ENGLISH: &str = "http://vocab.getty.edu/aat/300388277";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cursor {
    pending: Vec<String>,
    more: Vec<String>,
    total: Option<u64>,
}

async fn id_page(http: &Http, url: &str) -> Result<(Vec<String>, Option<String>, u64), String> {
    let v = http.json(url, &LD).await?;
    let ids = v["orderedItems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i["id"].as_str().map(str::to_string))
        .collect();
    Ok((
        ids,
        v["next"]["id"].as_str().map(str::to_string),
        v["partOf"]["totalItems"].as_u64().unwrap_or(0),
    ))
}

async fn first(ctx: &Ctx, q: &Query) -> Result<Cursor, String> {
    let text = q.text.trim();
    if text.is_empty() {
        let (pending, next, total) = id_page(&ctx.http, SEARCH).await?;
        return Ok(Cursor {
            pending,
            more: next.into_iter().collect(),
            total: Some(total),
        });
    }
    // There is no free-text search: look in makers, titles and descriptions and merge.
    let urls: Vec<String> = ["creator", "title", "description"]
        .iter()
        .map(|f| format!("{SEARCH}&{f}={}", enc(text)))
        .collect();
    let http = ctx.http.clone();
    let pages = map_concurrent(urls, 3, move |u| {
        let http = http.clone();
        async move { id_page(&http, &u).await }
    })
    .await;
    if pages.iter().all(|p| p.is_err()) {
        return Err(pages.into_iter().find_map(|p| p.err()).unwrap_or_default());
    }
    let mut cur = Cursor::default();
    let mut seen = std::collections::HashSet::new();
    let mut total = 0;
    for (ids, next, n) in pages.into_iter().flatten() {
        total += n;
        cur.pending
            .extend(ids.into_iter().filter(|i| seen.insert(i.clone())));
        cur.more.extend(next);
    }
    // An upper bound: a work found by title and by maker is counted once below.
    cur.total = Some(if cur.more.is_empty() {
        cur.pending.len() as u64
    } else {
        total
    });
    Ok(cur)
}

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let mut cur: Cursor = match q.cursor.as_deref() {
        Some(c) => serde_json::from_str(c)
            .map_err(|_| "This search can no longer be continued; start it again.".to_string())?,
        None => first(ctx, q).await?,
    };
    if cur.pending.len() < PAGE_SIZE && !cur.more.is_empty() {
        let url = cur.more.remove(0);
        if let Ok((ids, next, _)) = id_page(&ctx.http, &url).await {
            let fresh: Vec<String> = ids
                .into_iter()
                .filter(|i| !cur.pending.contains(i))
                .collect();
            cur.pending.extend(fresh);
            if let Some(n) = next {
                cur.more.insert(0, n);
            }
        }
    }
    let n = cur.pending.len().min(PAGE_SIZE);
    let ids: Vec<String> = cur.pending.drain(..n).collect();
    let limit = licensing::source("rijks")
        .map(|s| s.etiquette.search_concurrency)
        .unwrap_or(6);
    let http = ctx.http.clone();
    let records = map_concurrent(ids, limit, move |id| {
        let http = http.clone();
        async move { record(&http, &id).await }
    })
    .await;
    let failed = records.iter().filter(|r| r.is_err()).count();
    let all: Vec<Candidate> = records.into_iter().flatten().collect();
    let fetched = all.len();
    let items: Vec<Candidate> = all.into_iter().filter(|c| passes(q, c)).collect();
    let more = !cur.pending.is_empty() || !cur.more.is_empty();
    Ok(Page {
        hidden: fetched - items.len(),
        items,
        total: cur.total,
        next: more.then(|| serde_json::to_string(&cur).unwrap_or_default()),
        notice: (failed > 0 && fetched == 0).then(|| {
            "The Rijksmuseum did not return these records; try again in a moment.".to_string()
        }),
        ..Default::default()
    })
}

async fn record(http: &Http, id_url: &str) -> Result<Candidate, String> {
    let url = format!(
        "{OAI}?verb=GetRecord&metadataPrefix=edm&identifier={}",
        enc(id_url)
    );
    let xml = http.text(&url, &[]).await?;
    let mut c = parse_edm(&xml, id_url)?;
    // The EDM record has no pixel size; the image server does.
    if let Ok(info) = http
        .json(
            &format!("https://iiif.micr.io/{}/info.json", c.source_id),
            &[],
        )
        .await
    {
        if let (Some(w), Some(h)) = (u32_of(&info["width"]), u32_of(&info["height"])) {
            let (fw, fh) = fit_long_edge(w, h, MAX_EDGE);
            c.width = Some(fw);
            c.height = Some(fh);
            c.aspect = Some(((w as f64 / h as f64) * 1000.0).round() / 1000.0);
            if let Some(plan) = c.plan.as_mut() {
                plan.url = image_url(&c.source_id, w, h);
                if (fw, fh) != (w, h) {
                    plan.extra.insert("original_width".into(), w.into());
                    plan.extra.insert("original_height".into(), h.into());
                    c.notes.push(format!(
                        "The scan is {w}×{h} px; Galeria saves it at {fw}×{fh}."
                    ));
                }
            }
        }
    }
    Ok(c)
}

fn image_url(micrio: &str, w: u32, h: u32) -> String {
    let size = if w.max(h) <= MAX_EDGE {
        "max".to_string()
    } else if w >= h {
        format!("{MAX_EDGE},")
    } else {
        format!(",{MAX_EDGE}")
    };
    format!("https://iiif.micr.io/{micrio}/full/{size}/0/default.jpg")
}

/// Before a download: read the record again and ask whether the museum offers the image for download.
pub async fn resolve(ctx: &Ctx, id_url: &str) -> Result<Candidate, String> {
    if !id_url.starts_with("https://id.rijksmuseum.nl/") {
        return Err("Not a Rijksmuseum identifier.".into());
    }
    let mut c = record(&ctx.http, id_url).await?;
    let http = &ctx.http;
    let object = http.json(id_url, &LD).await?;
    let visual = object["shows"][0]["id"]
        .as_str()
        .ok_or("The Rijksmuseum record has no image.")?;
    let visual = http.json(visual, &LD).await?;
    let digital = visual["digitally_shown_by"][0]["id"]
        .as_str()
        .ok_or("The Rijksmuseum record has no digital image.")?;
    let digital = http.json(digital, &LD).await?;
    let downloadable = digital["referred_to_by"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["content"].as_str() == Some("downloadbaar"));

    let mut ev = evidence_of(&c);
    ev.insert("downloadable".into(), downloadable.to_string());
    c.decision = Some(licensing::evaluate("rijks", &ev));

    // The linked-data record says which title is the English one.
    let english = object["identified_by"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|n| {
            let is_en = n["language"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|l| l["id"].as_str() == Some(ENGLISH));
            (n["type"].as_str() == Some("Name") && is_en)
                .then(|| n["content"].as_str().map(crate::util::collapse_ws))
                .flatten()
        });
    if let Some(en) = english.filter(|e| !e.is_empty() && *e != c.title) {
        if c.title_original.is_none() {
            c.title_original = Some(std::mem::take(&mut c.title));
        }
        c.title = en;
    }
    Ok(c)
}

fn evidence_of(c: &Candidate) -> Evidence {
    let mut ev = Evidence::new();
    let rights = c
        .plan
        .as_ref()
        .and_then(|p| p.extra.get("rights_statement"))
        .and_then(|r| r.as_str())
        .unwrap_or("");
    ev.insert("rights".into(), rights.to_string());
    ev
}

/// The first four-digit year in a date such as `1735-11-18`, `ca. 1628` or `1628/1629`.
fn first_year(text: &str) -> Option<String> {
    let b = text.as_bytes();
    (0..b.len().saturating_sub(3))
        .find(|&i| b[i..i + 4].iter().all(u8::is_ascii_digit))
        .map(|i| text[i..i + 4].to_string())
}

fn score(s: &str, words: &[&str]) -> usize {
    let hay = format!(" {} ", s.to_lowercase());
    words.iter().filter(|w| hay.contains(*w)).count()
}

/// Rough "is this Dutch?" for titles, which the EDM record does not label reliably.
fn looks_dutch(s: &str) -> bool {
    const NL: [&str; 22] = [
        " van ",
        " het ",
        " een ",
        " met ",
        " bij ",
        " op ",
        " te ",
        " en ",
        " aan ",
        " uit ",
        " voor ",
        " der ",
        " den ",
        "gezicht",
        "landschap",
        "portret",
        "stilleven",
        " kerk",
        "schepen",
        " de ",
        " naar ",
        " zee",
    ];
    const EN: [&str; 18] = [
        " of ",
        " the ",
        " with ",
        " a ",
        " an ",
        " and ",
        " at ",
        " near ",
        " on ",
        " from ",
        " by ",
        "view",
        "landscape",
        "portrait",
        "still life",
        "church",
        "ships",
        " sea",
    ];
    score(s, &NL) > score(s, &EN)
}

fn medium_of(labels: &[String]) -> Option<String> {
    if labels.is_empty() {
        return None;
    }
    let short = |l: &str| l.split(" (").next().unwrap_or(l).trim().to_string();
    let paint = labels
        .iter()
        .find(|l| l.contains("paint"))
        .map(|l| short(l).replace(" paint", ""));
    let supports: Vec<String> = labels
        .iter()
        .filter(|l| !l.contains("paint"))
        .map(|l| short(l))
        .collect();
    Some(match (paint, supports.is_empty()) {
        (Some(p), false) => format!("{p} on {}", supports.join(" and ")),
        (Some(p), true) => p,
        (None, _) => supports.join(", "),
    })
}

fn parse_edm(xml: &str, id_url: &str) -> Result<Candidate, String> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| format!("The Rijksmuseum sent an unreadable record: {e}"))?;
    let attr = |n: roxmltree::Node, name: &str| {
        n.attributes()
            .find(|a| a.name() == name)
            .map(|a| a.value().to_string())
    };
    let named = |name: &'static str| {
        doc.descendants()
            .filter(move |n| n.is_element() && n.tag_name().name() == name)
    };
    let text = |n: roxmltree::Node| {
        n.text()
            .map(crate::util::collapse_ws)
            .filter(|s| !s.is_empty())
    };
    let cho = named("ProvidedCHO")
        .next()
        .ok_or("The Rijksmuseum has no such record.")?;
    let child = |name: &'static str| {
        cho.children()
            .filter(move |n| n.is_element() && n.tag_name().name() == name)
    };
    // English text of a repeated element, falling back to whatever is there.
    let english = |name: &'static str| {
        child(name)
            .find(|n| attr(*n, "lang").as_deref() == Some("en"))
            .and_then(text)
            .or_else(|| child(name).find_map(text))
    };

    // Labels of the agents and concepts the record points at.
    let mut labels: HashMap<String, String> = HashMap::new();
    for n in doc
        .descendants()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "Agent" | "Concept" | "Place"))
    {
        let Some(about) = attr(n, "about") else {
            continue;
        };
        let prefs: Vec<roxmltree::Node> = n
            .children()
            .filter(|c| c.tag_name().name() == "prefLabel")
            .collect();
        let label = prefs
            .iter()
            .find(|p| attr(**p, "lang").as_deref() == Some("en"))
            .or(prefs.first())
            .and_then(|p| text(*p));
        if let Some(l) = label {
            labels.insert(about, l);
        }
    }
    let label_of = |n: roxmltree::Node| attr(n, "resource").and_then(|r| labels.get(&r).cloned());

    let image = named("isShownBy")
        .next()
        .and_then(|n| attr(n, "resource"))
        .ok_or("This Rijksmuseum work has no image.")?;
    let micrio = image
        .strip_prefix("https://iiif.micr.io/")
        .and_then(|rest| rest.split('/').next())
        .filter(|s| !s.is_empty())
        .ok_or("This Rijksmuseum image is not served by its image service.")?
        .to_string();
    let rights = named("rights")
        .next()
        .and_then(|n| attr(n, "resource"))
        .unwrap_or_default();
    let page = named("isShownAt")
        .next()
        .and_then(|n| attr(n, "resource"))
        .map(|u| u.replace("/nl/collectie/object/", "/en/collection/object/"));

    let mut titles: Vec<String> = Vec::new();
    for t in child("title").filter_map(text) {
        if !titles.contains(&t) {
            titles.push(t);
        }
    }
    let (title, title_original) = match titles.len() {
        0 => ("Untitled".to_string(), None),
        1 => (titles.remove(0), None),
        _ => {
            let en = titles.iter().position(|t| !looks_dutch(t)).unwrap_or(0);
            let title = titles.remove(en);
            (title, titles.into_iter().next())
        }
    };

    let creator = child("creator").next();
    let artist = creator
        .and_then(label_of)
        .or_else(|| creator.and_then(text));
    let artist_bio = creator.and_then(|c| attr(c, "resource")).and_then(|about| {
        let agent = doc.descendants().find(|n| {
            n.tag_name().name() == "Agent" && attr(*n, "about").as_deref() == Some(about.as_str())
        })?;
        let year = |name: &str| {
            agent
                .children()
                .find(|c| c.tag_name().name() == name)
                .and_then(|c| c.text())
                .and_then(first_year)
        };
        match (year("dateOfBirth"), year("dateOfDeath")) {
            (Some(b), Some(d)) => Some(format!("{b}–{d}")),
            (Some(b), None) => Some(format!("born {b}")),
            (None, Some(d)) => Some(format!("died {d}")),
            _ => None,
        }
    });
    let media: Vec<String> = child("medium").filter_map(label_of).collect();
    let subjects: Vec<String> = child("subject").filter_map(label_of).collect();
    let dimensions = english("extent");
    // "height 62 cm x width 89 cm"
    let aspect = dimensions.as_deref().and_then(|d| {
        let num = |key: &str| {
            d.split(key)
                .nth(1)?
                .split_whitespace()
                .next()?
                .replace(',', ".")
                .parse::<f64>()
                .ok()
        };
        match (num("width "), num("height ")) {
            (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
            _ => None,
        }
    });
    let description = child("description")
        .find(|n| attr(*n, "lang").as_deref() == Some("en"))
        .and_then(text)
        .filter(|d| !looks_dutch(d));

    let mut extra = Map::new();
    extra.insert("persistent_url".into(), id_url.into());
    extra.insert("rights_statement".into(), rights.clone().into());
    let mut ev = Evidence::new();
    ev.insert("rights".into(), rights);
    let decision = licensing::evaluate("rijks", &ev);
    let kw: Vec<&str> = subjects.iter().map(String::as_str).collect();
    Ok(finish(
        Candidate {
            source: "rijks".into(),
            source_id: micrio.clone(),
            locator: id_url.to_string(),
            title,
            title_original,
            artist,
            artist_bio,
            date: english("created"),
            medium: medium_of(&media),
            dimensions,
            description: description.clone(),
            museum: "Rijksmuseum, Amsterdam".into(),
            credit_line: english("spn"),
            accession: child("identifier").find_map(text),
            page_url: page.or_else(|| Some(id_url.to_string())),
            thumb_url: Some(format!(
                "https://iiif.micr.io/{micrio}/full/400,/0/default.jpg"
            )),
            preview_url: Some(format!(
                "https://iiif.micr.io/{micrio}/full/1200,/0/default.jpg"
            )),
            aspect,
            decision: Some(decision),
            plan: Some(Plan {
                url: format!("https://iiif.micr.io/{micrio}/full/{MAX_EDGE},/0/default.jpg"),
                credit: "Rijksmuseum, Amsterdam".into(),
                kind: Some("painting".into()),
                extra,
                ..Default::default()
            }),
            ..Default::default()
        },
        &kw,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = r#"<?xml version="1.0" encoding="UTF-8" ?>
<OAI-PMH xmlns="http://www.openarchives.org/OAI/2.0/"><GetRecord><record><metadata>
<rdf:RDF xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:edm="http://www.europeana.eu/schemas/edm/" xmlns:mrel="http://id.loc.gov/vocabulary/relators/" xmlns:ore="http://www.openarchives.org/ore/terms/" xmlns:rdaGr2="http://rdvocab.info/ElementsGr2/" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:skos="http://www.w3.org/2004/02/skos/core#">
 <ore:Aggregation rdf:about="https://id.rijksmuseum.nl/200107792#aggregation">
  <edm:isShownAt rdf:resource="https://www.rijksmuseum.nl/nl/collectie/object/SK-A-4223--a8db"/>
  <edm:isShownBy rdf:resource="https://iiif.micr.io/OfGsE/full/max/0/default.jpg"/>
  <edm:rights rdf:resource="http://creativecommons.org/publicdomain/mark/1.0/"/>
 </ore:Aggregation>
 <edm:ProvidedCHO rdf:about="https://id.rijksmuseum.nl/200107792">
  <dcterms:created xml:lang="en">1790</dcterms:created>
  <dc:identifier>SK-A-4223</dc:identifier>
  <dc:creator rdf:resource="https://id.rijksmuseum.nl/2101168"/>
  <dc:description xml:lang="en">This untamed view of nature anticipates Romanticism.</dc:description>
  <dc:subject rdf:resource="https://id.rijksmuseum.nl/221168"/>
  <dc:title xml:lang="en">Berglandschap bij D&#252;sseldorf</dc:title>
  <dc:title xml:lang="en">Mountainous Landscape near D&#252;sseldorf</dc:title>
  <dc:title xml:lang="nl">Berglandschap bij D&#252;sseldorf</dc:title>
  <dcterms:extent xml:lang="en">height 62 cm x width 89 cm</dcterms:extent>
  <dcterms:extent xml:lang="nl">hoogte 62 cm x breedte 89 cm</dcterms:extent>
  <dcterms:medium rdf:resource="https://id.rijksmuseum.nl/220116"/>
  <dcterms:medium rdf:resource="https://id.rijksmuseum.nl/2209"/>
  <mrel:spn xml:lang="en">Purchased with the support of the Stichting</mrel:spn>
 </edm:ProvidedCHO>
 <edm:Agent rdf:about="https://id.rijksmuseum.nl/2101168">
  <skos:prefLabel xml:lang="en">Gerard van Nijmegen</skos:prefLabel>
  <rdaGr2:dateOfBirth xml:lang="nl">1735-11-18</rdaGr2:dateOfBirth>
  <rdaGr2:dateOfDeath xml:lang="nl">1808-04-29</rdaGr2:dateOfDeath>
 </edm:Agent>
 <skos:Concept rdf:about="https://id.rijksmuseum.nl/220116"><skos:prefLabel xml:lang="en">panel</skos:prefLabel><skos:prefLabel xml:lang="nl">paneel</skos:prefLabel></skos:Concept>
 <skos:Concept rdf:about="https://id.rijksmuseum.nl/2209"><skos:prefLabel xml:lang="nl">olieverf</skos:prefLabel><skos:prefLabel xml:lang="en">oil paint (paint)</skos:prefLabel></skos:Concept>
 <skos:Concept rdf:about="https://id.rijksmuseum.nl/221168"><skos:prefLabel xml:lang="en">mountains</skos:prefLabel></skos:Concept>
</rdf:RDF></metadata></record></GetRecord></OAI-PMH>"#;

    #[test]
    fn parses_an_edm_record() {
        let c = parse_edm(RECORD, "https://id.rijksmuseum.nl/200107792").unwrap();
        assert_eq!(c.id, "rijks:OfGsE");
        assert_eq!(c.title, "Mountainous Landscape near Düsseldorf");
        assert_eq!(
            c.title_original.as_deref(),
            Some("Berglandschap bij Düsseldorf")
        );
        assert_eq!(c.artist.as_deref(), Some("Gerard van Nijmegen"));
        assert_eq!(c.artist_bio.as_deref(), Some("1735–1808"));
        assert_eq!(c.medium.as_deref(), Some("oil on panel"));
        assert_eq!(c.date.as_deref(), Some("1790"));
        assert_eq!(c.accession.as_deref(), Some("SK-A-4223"));
        assert_eq!(c.aspect, Some(1.435));
        assert_eq!(
            c.page_url.as_deref(),
            Some("https://www.rijksmuseum.nl/en/collection/object/SK-A-4223--a8db")
        );
        let d = c.decision.as_ref().unwrap();
        assert!(d.allowed && d.license == "Public domain", "{d:?}");
        assert!(
            c.genres.contains(&"landscape".to_string()),
            "{:?}",
            c.genres
        );
        assert_eq!(
            c.plan.as_ref().unwrap().extra["persistent_url"],
            "https://id.rijksmuseum.nl/200107792"
        );
    }

    #[test]
    fn copyrighted_images_are_refused() {
        let xml = RECORD.replace(
            "http://creativecommons.org/publicdomain/mark/1.0/",
            "http://rightsstatements.org/vocab/InC/1.0/",
        );
        let c = parse_edm(&xml, "https://id.rijksmuseum.nl/1").unwrap();
        assert!(!c.decision.unwrap().allowed);
    }

    #[test]
    fn helpers() {
        assert_eq!(first_year("1735-11-18").as_deref(), Some("1735"));
        assert_eq!(first_year("ca. 1628").as_deref(), Some("1628"));
        assert_eq!(first_year("ca."), None);
        assert!(looks_dutch("Gezicht op Haarlem met bleekvelden"));
        assert!(!looks_dutch("View of Haarlem with Bleaching Grounds"));
        assert_eq!(
            image_url("abc", 5000, 4000),
            "https://iiif.micr.io/abc/full/max/0/default.jpg"
        );
        assert_eq!(
            image_url("abc", 9000, 4000),
            "https://iiif.micr.io/abc/full/6500,/0/default.jpg"
        );
        assert_eq!(
            image_url("abc", 4000, 9000),
            "https://iiif.micr.io/abc/full/,6500/0/default.jpg"
        );
        assert_eq!(
            medium_of(&["canvas".into(), "oil paint (paint)".into()]).as_deref(),
            Some("oil on canvas")
        );
    }
}
