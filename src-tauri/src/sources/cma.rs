//! Cleveland Museum of Art — Open Access API (https://openaccess-api.clevelandart.org/).

use super::*;
use crate::licensing::{self, Evidence};

const API: &str = "https://openaccess-api.clevelandart.org/api/artworks";
/// Larger full-size TIFFs are not fetched (the download limit is 700 MB).
const MAX_FULL_BYTES: u64 = 600 * 1024 * 1024;
const FIELDS: &str = "id,accession_number,share_license_status,copyright,title,creation_date,creators,technique,measurements,department,culture,url,creditline,images,type,description";

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let skip: usize = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    // `cc0=1` asks the museum for open-access works only; every record is still checked below.
    let mut url = format!(
        "{API}?cc0=1&has_image=1&type=Painting&limit={PAGE_SIZE}&skip={skip}&fields={FIELDS}"
    );
    if !q.text.trim().is_empty() {
        url.push_str(&format!("&q={}", enc(q.text.trim())));
    }
    let v = ctx.http.json(&url, &[]).await?;
    let total = v["info"]["total"].as_u64().unwrap_or(0);
    let all: Vec<Candidate> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(candidate)
        .collect();
    let fetched = all.len();
    let items: Vec<Candidate> = all.into_iter().filter(|c| passes(q, c)).collect();
    Ok(Page {
        hidden: fetched - items.len(),
        items,
        total: Some(total),
        next: (((skip + PAGE_SIZE) as u64) < total).then(|| (skip + PAGE_SIZE).to_string()),
        ..Default::default()
    })
}

pub async fn resolve(ctx: &Ctx, accession: &str) -> Result<Candidate, String> {
    let v = ctx
        .http
        .json(&format!("{API}/{}", enc(accession)), &[])
        .await?;
    candidate(&v["data"]).ok_or_else(|| {
        "The Cleveland Museum of Art has no downloadable image for this work.".to_string()
    })
}

fn candidate(a: &Value) -> Option<Candidate> {
    let accession = str_of(&a["accession_number"])?;
    let images = &a["images"];
    // The full-size file is an uncompressed TIFF, sometimes over a gigabyte. Above
    // the limit, take the museum's 3,400 px print JPEG instead.
    let size_of = |i: &Value| {
        i["filesize"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .or_else(|| i["filesize"].as_u64())
    };
    let full_bytes = size_of(&images["full"]);
    let skip_full = full_bytes.is_some_and(|b| b > MAX_FULL_BYTES);
    let image = ["full", "print", "web"]
        .iter()
        .filter(|k| !(skip_full && **k == "full"))
        .map(|k| &images[*k])
        .find(|i| i["url"].is_string())?;
    let url = image["url"].as_str()?.to_string();
    let (w, h) = (u32_of(&image["width"]), u32_of(&image["height"]));
    let download_bytes = size_of(image);

    let creator = a["creators"]
        .as_array()
        .and_then(|c| {
            c.iter()
                .find(|c| c["role"].as_str() == Some("artist"))
                .or_else(|| c.first())
        })
        .and_then(|c| str_of(&c["description"]));
    let (artist, artist_bio) = match creator {
        Some(c) => {
            let (name, bio) = split_artist(&c);
            (Some(name), bio)
        }
        None => (None, None),
    };

    let mut ev = Evidence::new();
    ev.insert(
        "share_license_status".into(),
        str_of(&a["share_license_status"]).unwrap_or_default(),
    );
    let decision = licensing::evaluate("cma", &ev);
    let mut notes = Vec::new();
    if let Some(c) = str_of(&a["copyright"]) {
        notes.push(format!("Copyright note in the museum's record: {c}"));
    }
    if url.ends_with(".tif") {
        notes.push("Saved from the museum's full-size TIFF and converted to JPEG.".to_string());
    }
    if let Some(b) = full_bytes.filter(|_| skip_full) {
        notes.push(format!(
            "The museum's full scan is {:.1} GB, too large to fetch for a wallpaper; Galeria takes its print-size file instead.",
            b as f64 / 1e9
        ));
    }
    let mut extra = Map::new();
    if let Some(d) = str_of(&a["department"]) {
        extra.insert("department".into(), d.into());
    }
    let description = clean(a["description"].as_str());
    let culture = a["culture"]
        .as_array()
        .and_then(|c| c.first())
        .and_then(str_of)
        .unwrap_or_default();
    Some(finish(
        Candidate {
            source: "cma".into(),
            source_id: accession.clone(),
            locator: accession.clone(),
            title: str_of(&a["title"]).unwrap_or_else(|| "Untitled".into()),
            artist,
            artist_bio,
            date: str_of(&a["creation_date"]),
            medium: str_of(&a["technique"]),
            dimensions: str_of(&a["measurements"]),
            description: description.clone(),
            museum: "Cleveland Museum of Art, Cleveland".into(),
            credit_line: str_of(&a["creditline"]),
            accession: Some(accession.clone()),
            department: str_of(&a["department"]),
            page_url: Some(format!("https://www.clevelandart.org/art/{accession}")),
            thumb_url: images["web"]["url"].as_str().map(str::to_string),
            preview_url: images["print"]["url"]
                .as_str()
                .or(images["web"]["url"].as_str())
                .map(str::to_string),
            width: w,
            height: h,
            download_bytes,
            decision: Some(decision),
            notes,
            plan: Some(Plan {
                url,
                credit: "The Cleveland Museum of Art (CC0)".into(),
                kind: Some("painting".into()),
                extra,
                ..Default::default()
            }),
            ..Default::default()
        },
        // Titles and culture only: descriptions mention too much else ("…better known for his portraits").
        &[culture.as_str()],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_record() {
        let a = serde_json::json!({
            "accession_number": "1972.47", "share_license_status": "CC0", "title": "Virgin and Child in a Landscape",
            "creation_date": "1531", "technique": "oil on wood", "creditline": "John L. Severance Fund",
            "creators": [{ "description": "Jan Gossaert (Flemish, c. 1475/78–1532)", "role": "artist" }],
            "images": { "web": { "url": "https://cdn/1972.47_web.jpg", "width": "778", "height": "893" },
                        "full": { "url": "https://cdn/1972.47_full.tif", "width": "5536", "height": "6931" } }
        });
        let c = candidate(&a).unwrap();
        assert_eq!(c.id, "cma:1972.47");
        assert_eq!(c.artist.as_deref(), Some("Jan Gossaert"));
        assert_eq!(c.artist_bio.as_deref(), Some("Flemish, c. 1475/78–1532"));
        assert_eq!((c.width, c.height), (Some(5536), Some(6931)));
        assert!(c.decision.as_ref().unwrap().allowed);
        assert!(c.plan.unwrap().url.ends_with("_full.tif"));

        // A gigabyte TIFF is passed over for the print file.
        let mut huge = a.clone();
        huge["images"] = serde_json::json!({
            "web": { "url": "https://cdn/x_web.jpg", "width": "900", "height": "314", "filesize": "96599" },
            "print": { "url": "https://cdn/x_print.jpg", "width": "3400", "height": "1187", "filesize": "1203087" },
            "full": { "url": "https://cdn/x_full.tif", "width": "32683", "height": "11412", "filesize": "1118962804" }
        });
        let c = candidate(&huge).unwrap();
        assert!(c.plan.as_ref().unwrap().url.ends_with("_print.jpg"));
        assert_eq!((c.width, c.download_bytes), (Some(3400), Some(1_203_087)));
        assert!(
            c.notes.iter().any(|n| n.contains("1.1 GB")),
            "{:?}",
            c.notes
        );
        assert!(
            c.genres.contains(&"religious".to_string())
                && c.genres.contains(&"landscape".to_string())
        );

        let mut b = a.clone();
        b["share_license_status"] = "Copyrighted".into();
        assert!(!candidate(&b).unwrap().decision.unwrap().allowed);
    }
}
