//! Wikimedia Commons — the route to works held by museums that have no
//! open-access programme of their own. Every file carries its own license,
//! which is read from the file's metadata and checked against the license table.

use super::*;
use crate::licensing::{self, Evidence};

const API: &str = "https://commons.wikimedia.org/w/api.php";
const META: &str = "LicenseShortName|License|LicenseUrl|UsageTerms|AttributionRequired|Copyrighted|Artist|Credit|ImageDescription|ObjectName|DateTimeOriginal|Categories|Restrictions";
/// Originals above this are fetched as a rendition instead (some are 300 MB).
const MAX_ORIGINAL_BYTES: u64 = 60 * 1024 * 1024;
const MAX_ORIGINAL_EDGE: u32 = 12_000;
/// The widest rendition Commons will generate.
const RENDITION_WIDTH: u32 = 3840;

fn params(extra: &[(&str, String)]) -> String {
    let mut p: Vec<(String, String)> = vec![
        ("action".into(), "query".into()),
        ("format".into(), "json".into()),
        ("formatversion".into(), "2".into()),
        ("prop".into(), "imageinfo".into()),
        ("iiprop".into(), "url|size|mime|extmetadata".into()),
        ("iiurlwidth".into(), "500".into()),
        ("iiextmetadatafilter".into(), META.into()),
        ("iiextmetadatalanguage".into(), "en".into()),
    ];
    p.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
    p.iter()
        .map(|(k, v)| format!("{k}={}", enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let offset: u64 = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let mut terms = q.text.trim().to_string();
    if let Some(m) = q.museum.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        // "Musée du Louvre, Paris" → the institution's name without the city.
        let name = m.split(',').next().unwrap_or(m).trim();
        terms = format!("{terms} \"{name}\"");
    }
    if terms.trim().is_empty() {
        terms = "painting".into();
    }
    let floor = q.min_long_edge.max(1000).saturating_sub(1);
    let gsr = format!("{} filetype:bitmap filew:>{floor}", terms.trim());
    let url = format!(
        "{API}?{}",
        params(&[
            ("generator", "search".into()),
            ("gsrsearch", gsr),
            ("gsrnamespace", "6".into()),
            ("gsrlimit", PAGE_SIZE.to_string()),
            ("gsroffset", offset.to_string()),
        ])
    );
    let v = ctx.http.json(&url, &[]).await?;
    let mut pages: Vec<&Value> = v["query"]["pages"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    pages.sort_by_key(|p| p["index"].as_u64().unwrap_or(u64::MAX));
    let fetched = pages.len();
    let chosen = q.museum.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let items: Vec<Candidate> = pages
        .into_iter()
        .filter_map(|p| candidate(p, chosen, &ctx.museums))
        .filter(|c| passes(q, c))
        .collect();
    Ok(Page {
        hidden: fetched - items.len(),
        items,
        total: None,
        next: v["continue"]["gsroffset"].as_u64().map(|o| o.to_string()),
        ..Default::default()
    })
}

pub async fn resolve(ctx: &Ctx, locator: &str) -> Result<Candidate, String> {
    // `File:Name.jpg`, optionally followed by `|<collection the user was browsing>`.
    let (title, chosen) = match locator.split_once('|') {
        Some((t, m)) => (t, Some(m)),
        None => (locator, None),
    };
    if !title.starts_with("File:") {
        return Err("Not a Wikimedia Commons file.".into());
    }
    let url = format!("{API}?{}", params(&[("titles", title.to_string())]));
    let v = ctx.http.json(&url, &[]).await?;
    v["query"]["pages"]
        .as_array()
        .and_then(|p| p.first())
        .filter(|p| p["missing"].as_bool() != Some(true))
        .and_then(|p| candidate_any(p, chosen, &ctx.museums))
        .ok_or_else(|| "Wikimedia Commons no longer has this file.".to_string())
}

/// Files that pass a size check but are not a whole painting.
fn is_fragment(title: &str) -> bool {
    let t = title.to_lowercase();
    let tile = t.as_bytes().windows(2).enumerate().any(|(i, w)| {
        // "-x0-y1" tiles of gigapixel scans
        w == b"-x"
            && t[i + 2..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
            && t[i..].contains("-y")
    });
    tile || [
        "detail",
        "crop",
        " tile",
        "fragment",
        "(cropped)",
        "x-ray",
        "infrared",
        "reverse",
        "verso",
    ]
    .iter()
    .any(|k| t.contains(k))
}

/// "File:Ivan Aivazovsky - The Ninth Wave - Google Art Project.jpg" → "Ivan Aivazovsky - The Ninth Wave"
fn title_from_file(title: &str) -> String {
    let t = title.strip_prefix("File:").unwrap_or(title);
    let t = t.rsplit_once('.').map(|(a, _)| a).unwrap_or(t);
    let t = t.replace('_', " ");
    let t = t
        .trim_end_matches(" - Google Art Project")
        .trim_end_matches(" - Google Cultural Institute");
    // "Icon with scenes from Genesis (Byzantine and Christian Museum of Athens, 1-15-2023)":
    // a trailing note about where or when the photo was taken is not part of the title.
    let t = match t.rsplit_once(" (") {
        Some((head, note)) if t.ends_with(')') && !head.trim().is_empty() => {
            let n = note.to_lowercase();
            let about_photo = n.chars().any(|c| c.is_ascii_digit())
                || ["museum", "gallery", "galerie", "louvre", "cropped", "edit"]
                    .iter()
                    .any(|k| n.contains(k));
            if about_photo {
                head
            } else {
                t
            }
        }
        _ => t,
    };
    crate::util::collapse_ws(t)
}

/// Commons appends machine-readable Wikidata snippets to some fields:
/// `Calm Sealabel QS:Lde,"Ruhige See"` → `Calm Sea`; `German: Stürmische See.title QS:…` → `Stürmische See`.
fn tidy(s: &str) -> String {
    let mut t = s;
    for marker in ["label QS:", "title QS:", "date QS:"] {
        t = t.split(marker).next().unwrap_or(t);
    }
    let t = t.trim().trim_end_matches('.').trim();
    // A leading language name ("German: …").
    let t = match t.split_once(": ") {
        Some((lang, rest))
            if lang.len() < 12 && lang.chars().all(|c| c.is_alphabetic()) && !rest.is_empty() =>
        {
            rest
        }
        _ => t,
    };
    crate::util::collapse_ws(t)
}

/// A reproduction of a flat artwork (which is what Galeria shows), as opposed
/// to someone's photograph of a statue, a building or a gallery room.
fn looks_like_artwork(file_title: &str, cats: &str, public_domain: bool) -> bool {
    let hay = format!("{file_title} {cats}").to_lowercase();
    const NOT: [&str; 12] = [
        "statue",
        "sculpture",
        "monument",
        "signature",
        "grave",
        "tomb",
        "portrait of the artist",
        "interior of",
        "exterior",
        "building",
        "museum 4y",
        "bust of",
    ];
    if NOT.iter().any(|k| hay.contains(k)) {
        return false;
    }
    const YES: [&str; 14] = [
        "painting",
        "paintings",
        "icon",
        "fresco",
        "mosaic",
        "mural",
        "oil on",
        "tempera",
        "watercolor",
        "watercolour",
        "google art project",
        "altarpiece",
        "triptych",
        "miniature",
    ];
    // Public-domain files found by a painting search are overwhelmingly scans of the works themselves.
    public_domain || YES.iter().any(|k| hay.contains(k))
}

/// "Ivan Aivazovsky - The Ninth Wave" → the painter, when the file is named that way.
fn artist_from_name(name: &str) -> Option<String> {
    let (first, rest) = name.split_once(" - ")?;
    let words = first.split_whitespace().count();
    (!rest.is_empty() && (1..=5).contains(&words) && !first.chars().any(|c| c.is_ascii_digit()))
        .then(|| first.trim().to_string())
}

fn museum_from_categories(cats: &str) -> Option<String> {
    const HINTS: [&str; 12] = [
        "Museum",
        "Gallery",
        "Galerie",
        "Galleria",
        "Musée",
        "Museo",
        "Hermitage",
        "Louvre",
        "Prado",
        "Uffizi",
        "Pinakothek",
        "Kunsthalle",
    ];
    const PREFIXES: [&str; 9] = [
        "Paintings in the ",
        "Paintings in ",
        "Collections of the ",
        "Collections of ",
        "Google Art Project works in the ",
        "Google Art Project works in ",
        "Paintings of the ",
        "Oil paintings in the ",
        "Artworks in the ",
    ];
    cats.split('|').map(str::trim).find_map(|c| {
        let stripped = PREFIXES.iter().find_map(|p| c.strip_prefix(p))?;
        HINTS
            .iter()
            .any(|h| stripped.contains(h))
            .then(|| stripped.to_string())
    })
}

/// A museum the library already names, when the file's title, description or categories mention it.
fn known_museum(known: &[String], texts: &[&str]) -> Option<String> {
    let hay = texts.join(" ").to_lowercase();
    known
        .iter()
        .filter(|m| m.as_str() != "Collection not recorded")
        // "Musée du Louvre, Paris" → "musée du louvre"
        .map(|m| (m, m.split(',').next().unwrap_or(m).trim().to_lowercase()))
        .filter(|(_, name)| name.len() >= 6 && hay.contains(name.as_str()))
        .max_by_key(|(_, name)| name.len())
        .map(|(m, _)| m.clone())
}

fn candidate(p: &Value, museum: Option<&str>, known: &[String]) -> Option<Candidate> {
    let title = p["title"].as_str()?;
    if is_fragment(title) {
        return None;
    }
    let c = candidate_any(p, museum, known)?;
    let cats = p["imageinfo"][0]["extmetadata"]["Categories"]["value"]
        .as_str()
        .unwrap_or("");
    let public_domain = c
        .decision
        .as_ref()
        .is_some_and(|d| d.allowed && !d.attribution_required);
    looks_like_artwork(title, cats, public_domain).then_some(c)
}

fn candidate_any(p: &Value, chosen: Option<&str>, known: &[String]) -> Option<Candidate> {
    let file_title = p["title"].as_str()?.to_string();
    let ii = p["imageinfo"].as_array()?.first()?;
    let mime = ii["mime"].as_str().unwrap_or("");
    if !matches!(
        mime,
        "image/jpeg" | "image/png" | "image/tiff" | "image/webp"
    ) {
        return None;
    }
    let (w, h) = (u32_of(&ii["width"])?, u32_of(&ii["height"])?);
    let bytes = ii["size"].as_u64().unwrap_or(0);
    let original = ii["url"].as_str()?.split('?').next()?.to_string();
    let meta = |k: &str| clean(ii["extmetadata"][k]["value"].as_str());

    let license = meta("LicenseShortName")
        .or_else(|| meta("License"))
        .unwrap_or_default();
    let mut ev = Evidence::new();
    ev.insert("license".into(), license.clone());
    if let Some(u) = meta("LicenseUrl") {
        ev.insert("license_url".into(), u);
    }
    if let Some(a) = meta("AttributionRequired") {
        ev.insert("attribution_required".into(), a);
    }
    let decision = licensing::evaluate("commons", &ev);

    // For a faithful scan of an old painting "Artist" is the painter. For a
    // photograph under an attribution license it is the photographer, who must
    // be credited but did not paint the picture.
    let creator = meta("Artist")
        .map(|a| a.trim_start_matches("Creator:").trim().to_string())
        .filter(|a| !a.is_empty() && a.len() < 120);
    let photo = decision.attribution_required;
    let name = title_from_file(&file_title);
    let object = meta("ObjectName")
        .map(|o| tidy(&o))
        .filter(|o| !o.is_empty() && o.len() < 160);
    let title = object.unwrap_or_else(|| name.clone());
    let artist = if photo {
        None
    } else {
        creator.clone().or_else(|| artist_from_name(&name))
    };
    // "1873date QS:P571,+1873-00-00T00:00:00Z/9" → "1873"
    let date = if photo {
        None
    } else {
        meta("DateTimeOriginal")
            .map(|d| tidy(&d))
            .filter(|d| !d.is_empty() && d.len() < 40)
    };
    let credit = match (&creator, photo) {
        (Some(c), true) => format!("Photo: {c}, {}, via Wikimedia Commons", decision.license),
        (None, true) => format!("{}, via Wikimedia Commons", decision.license),
        _ => "Wikimedia Commons (public domain)".to_string(),
    };
    let cats = meta("Categories").unwrap_or_default();
    let about = meta("ImageDescription").unwrap_or_default();
    let museum = chosen
        .map(str::to_string)
        .or_else(|| known_museum(known, &[file_title.as_str(), cats.as_str(), about.as_str()]))
        .or_else(|| museum_from_categories(&cats))
        .unwrap_or_else(|| "Collection not recorded".to_string());

    // Very large originals are throttled and can be hundreds of megabytes: take a rendition.
    let too_big =
        bytes > MAX_ORIGINAL_BYTES || w.max(h) > MAX_ORIGINAL_EDGE || mime == "image/tiff";
    let (url, sw, sh) = if too_big && w > RENDITION_WIDTH {
        let name = file_title
            .strip_prefix("File:")
            .unwrap_or(&file_title)
            .replace(' ', "_");
        let sh = ((h as f64) * (RENDITION_WIDTH as f64 / w as f64)).round() as u32;
        (
            format!(
                "https://commons.wikimedia.org/wiki/Special:FilePath/{}?width={RENDITION_WIDTH}",
                enc_path(&name)
            ),
            RENDITION_WIDTH,
            sh,
        )
    } else {
        (original, w, h)
    };
    let mut extra = Map::new();
    extra.insert("commons_file".into(), file_title.clone().into());
    let mut notes = Vec::new();
    if (sw, sh) != (w, h) {
        extra.insert("original_width".into(), w.into());
        extra.insert("original_height".into(), h.into());
        notes.push(format!(
            "The original is {w}×{h} px ({} MB); Galeria saves a {sw} px rendition.",
            bytes / 1_000_000
        ));
    }
    if photo {
        notes.push("This is a photograph under an attribution license: the photographer and license always appear on the plate.".to_string());
    }
    if let Some(r) = meta("Restrictions").filter(|r| !r.is_empty()) {
        notes.push(format!("Commons notes other restrictions: {r}."));
    }
    let description = meta("ImageDescription").filter(|d| d.len() > 20 && d.len() < 1200);
    let kind = {
        let hay = format!("{} {}", file_title, cats).to_lowercase();
        if hay.contains("fresco") {
            "fresco"
        } else if hay.contains("mosaic") {
            "mosaic"
        } else if hay.contains("icon") && !hay.contains("iconography") {
            "icon"
        } else {
            "painting"
        }
    };
    let page_url = ii["descriptionurl"].as_str().map(str::to_string);
    Some(finish(
        Candidate {
            source: "commons".into(),
            source_id: file_title.clone(),
            locator: match chosen {
                Some(m) => format!("{file_title}|{m}"),
                None => file_title.clone(),
            },
            title,
            artist,
            date,
            description: description.clone(),
            museum,
            page_url,
            thumb_url: ii["thumburl"]
                .as_str()
                .map(|u| u.split('?').next().unwrap_or(u).to_string()),
            preview_url: ii["thumburl"]
                .as_str()
                .map(|u| u.split('?').next().unwrap_or(u).to_string()),
            width: Some(sw),
            height: Some(sh),
            download_bytes: ((sw, sh) == (w, h) && bytes > 0).then_some(bytes),
            decision: Some(decision),
            notes,
            plan: Some(Plan {
                url,
                credit,
                kind: Some(kind.into()),
                extra,
                ..Default::default()
            }),
            ..Default::default()
        },
        &[name.as_str(), cats.as_str()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, w: u32, h: u32, size: u64, license: &str, artist: &str) -> Value {
        serde_json::json!({
            "title": title, "index": 1,
            "imageinfo": [{
                "url": "https://upload.wikimedia.org/wikipedia/commons/1/17/X.jpg?utm_source=commons", "descriptionurl": "https://commons.wikimedia.org/wiki/File:X.jpg",
                "thumburl": "https://thumb.wikimedia.org/x/500px-X.jpg?utm=1", "width": w, "height": h, "size": size, "mime": "image/jpeg",
                "extmetadata": {
                    "LicenseShortName": { "value": license }, "Artist": { "value": artist },
                    "DateTimeOriginal": { "value": "1873<div style=\"display: none;\">date QS:P571,+1873-00-00T00:00:00Z/9</div>" },
                    "Categories": { "value": "Paintings by Ivan Aivazovsky|Paintings in the State Tretyakov Gallery|PD-Art" }
                }
            }]
        })
    }

    #[test]
    fn public_domain_scan() {
        let p = page(
            "File:Ivan Aivazovsky - The Ninth Wave - Google Art Project.jpg",
            5000,
            3300,
            9_000_000,
            "Public domain",
            "<a href=\"x\">Ivan Aivazovsky</a>",
        );
        let c = candidate(&p, None, &[]).unwrap();
        assert_eq!(c.title, "Ivan Aivazovsky - The Ninth Wave");
        assert_eq!(c.artist.as_deref(), Some("Ivan Aivazovsky"));
        assert_eq!(c.date.as_deref(), Some("1873"));
        assert_eq!(c.museum, "State Tretyakov Gallery");
        let d = c.decision.as_ref().unwrap();
        assert!(d.allowed && !d.attribution_required);
        let plan = c.plan.as_ref().unwrap();
        assert_eq!(
            plan.url,
            "https://upload.wikimedia.org/wikipedia/commons/1/17/X.jpg"
        );
        assert_eq!(plan.credit, "Wikimedia Commons (public domain)");
        assert!(c.genres.is_empty() || !c.genres.contains(&"portrait".to_string()));
    }

    #[test]
    fn attributed_photograph() {
        let p = page(
            "File:Grecia, icona di san giorgio.jpg",
            4200,
            3000,
            5_000_000,
            "CC BY-SA 4.0",
            "Sailko",
        );
        let c = candidate(&p, Some("Byzantine and Christian Museum, Athens"), &[]).unwrap();
        let d = c.decision.as_ref().unwrap();
        assert!(d.allowed && d.attribution_required && d.license == "CC BY-SA 4.0");
        assert_eq!(c.artist, None, "the photographer is not the painter");
        assert_eq!(c.date, None, "the photo date is not the painting's date");
        assert_eq!(
            c.plan.as_ref().unwrap().credit,
            "Photo: Sailko, CC BY-SA 4.0, via Wikimedia Commons"
        );
        assert_eq!(c.museum, "Byzantine and Christian Museum, Athens");
        assert_eq!(c.plan.as_ref().unwrap().kind.as_deref(), Some("icon"));
    }

    #[test]
    fn refusals_fragments_and_giants() {
        let nc = candidate(
            &page("File:A.jpg", 4000, 3000, 1, "CC BY-NC 2.0", "x"),
            None,
            &[],
        )
        .map(|c| c.decision.unwrap().allowed);
        assert_eq!(nc, Some(false));
        assert!(candidate(
            &page(
                "File:Night Watch-x0-y1.jpg",
                9000,
                9000,
                1,
                "Public domain",
                "x"
            ),
            None,
            &[]
        )
        .is_none());
        assert!(candidate(
            &page(
                "File:Mona Lisa (detail).jpg",
                4000,
                3000,
                1,
                "Public domain",
                "x"
            ),
            None,
            &[]
        )
        .is_none());
        assert!(!is_fragment("File:Maxwell-Boltzmann by Extra Artist.jpg"));
        // Photographs of things that are not paintings are left out.
        assert!(candidate(
            &page(
                "File:Statue of Hovhannes Aivazovsky, Yerevan 01.jpg",
                3264,
                1836,
                1,
                "CC BY-SA 4.0",
                "x"
            ),
            None,
            &[]
        )
        .is_none());
        assert!(
            candidate(
                &page(
                    "File:Aivazovsky Russian Museum.jpg",
                    5796,
                    3864,
                    1,
                    "CC BY-SA 4.0",
                    "x"
                ),
                None,
                &[]
            )
            .is_some(),
            "its categories say it is a painting"
        );
        assert_eq!(
            tidy("Calm Sealabel QS:Lde,\"Ruhige See\"label QS:Len,\"Calm Sea\""),
            "Calm Sea"
        );
        assert_eq!(
            tidy("German: Stürmische See im Abendrot.title QS:P1476,de:\"x\""),
            "Stürmische See im Abendrot"
        );
        assert_eq!(tidy("Saint George: an icon"), "Saint George: an icon");
        assert_eq!(
            artist_from_name("Hovhannes Aivazovsky - The Ninth Wave").as_deref(),
            Some("Hovhannes Aivazovsky")
        );
        assert_eq!(artist_from_name("1850 - A view of the bay"), None);
        assert_eq!(title_from_file("File:Icon with scenes from Genesis (Byzantine and Christian Museum of Athens, 1-15-2023).jpg"), "Icon with scenes from Genesis");
        assert_eq!(
            title_from_file("File:Stacks of Wheat (End of Summer).jpg"),
            "Stacks of Wheat (End of Summer)"
        );
        // A file that names a museum already in the library joins it under the library's name.
        let known = vec![
            "Byzantine and Christian Museum, Athens".to_string(),
            "Musée du Louvre, Paris".to_string(),
        ];
        let c = candidate(
            &page(
                "File:Icon with scenes from Genesis (Byzantine and Christian Museum of Athens).jpg",
                6000,
                4000,
                1,
                "CC BY-SA 4.0",
                "x",
            ),
            None,
            &known,
        )
        .unwrap();
        assert_eq!(c.museum, "Byzantine and Christian Museum, Athens");
        assert_eq!(
            c.locator,
            "File:Icon with scenes from Genesis (Byzantine and Christian Museum of Athens).jpg"
        );
        let c = candidate(
            &page("File:Some icon.jpg", 6000, 4000, 1, "CC BY-SA 4.0", "x"),
            Some("Musée du Louvre, Paris"),
            &known,
        )
        .unwrap();
        assert_eq!(
            (c.museum.as_str(), c.locator.as_str()),
            (
                "Musée du Louvre, Paris",
                "File:Some icon.jpg|Musée du Louvre, Paris"
            )
        );
        let giant = candidate(
            &page(
                "File:The Night Watch - HD.jpg",
                14168,
                11528,
                190_000_000,
                "Public domain",
                "Rembrandt",
            ),
            None,
            &[],
        )
        .unwrap();
        assert_eq!((giant.width, giant.height), (Some(3840), Some(3124)));
        assert!(giant
            .plan
            .unwrap()
            .url
            .starts_with("https://commons.wikimedia.org/wiki/Special:FilePath/The_Night_Watch_"));
    }
}
