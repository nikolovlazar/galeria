//! Art Institute of Chicago — public API (https://api.artic.edu/docs/).

use super::*;
use crate::licensing::{self, Evidence};

const API: &str = "https://api.artic.edu/api/v1/artworks";
const IIIF: &str = "https://www.artic.edu/iiif/2";
const FIELDS: &str = "id,title,artist_display,artist_title,date_display,medium_display,dimensions,image_id,is_public_domain,artwork_type_title,classification_titles,subject_titles,style_title,thumbnail,credit_line,main_reference_number,department_title,short_description";
/// The size the museum's own download button serves for public-domain works.
const DOWNLOAD_EDGE: u32 = 3000;
/// The API asks every client to identify its project.
const HEADERS: [(&str, &str); 1] = [(
    "AIC-User-Agent",
    "Galeria desktop wallpaper app (https://github.com/nikolovlazar/galeria)",
)];

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let page: u64 = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(1);
    let mut params = serde_json::json!({
        "limit": PAGE_SIZE,
        "page": page,
        "fields": FIELDS,
        "query": { "bool": { "filter": [
            { "term": { "is_public_domain": true } },
            { "term": { "artwork_type_title.keyword": "Painting" } },
            { "exists": { "field": "image_id" } }
        ] } }
    });
    if !q.text.trim().is_empty() {
        // A plain `q` only re-orders the public-domain paintings; a `must` clause makes
        // the words a requirement, so the count and later pages stay on topic.
        params["query"]["bool"]["must"] = serde_json::json!([{ "multi_match": {
            "query": q.text.trim(),
            "operator": "and",
            "type": "cross_fields",
            "fields": ["title^3", "artist_title^3", "artist_display", "subject_titles^2", "term_titles", "style_title", "classification_titles", "place_of_origin", "medium_display", "description"]
        } }]);
    }
    let url = format!("{API}/search?params={}", enc(&params.to_string()));
    let v = ctx.http.json(&url, &HEADERS).await?;
    let total = v["pagination"]["total"].as_u64().unwrap_or(0);
    let pages = v["pagination"]["total_pages"].as_u64().unwrap_or(1);
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
        // The API refuses to page past its first 1,000 results.
        next: (page < pages && page * (PAGE_SIZE as u64) < 1000).then(|| (page + 1).to_string()),
        ..Default::default()
    })
}

pub async fn resolve(ctx: &Ctx, id: &str) -> Result<Candidate, String> {
    let v = ctx
        .http
        .json(&format!("{API}/{}?fields={FIELDS}", enc(id)), &HEADERS)
        .await?;
    candidate(&v["data"])
        .ok_or_else(|| "The Art Institute of Chicago has no image for this work.".to_string())
}

fn candidate(a: &Value) -> Option<Candidate> {
    let id = str_of(&a["id"])?;
    let image_id = a["image_id"].as_str()?.to_string();
    let mut ev = Evidence::new();
    ev.insert(
        "is_public_domain".into(),
        a["is_public_domain"].as_bool().unwrap_or(false).to_string(),
    );
    let decision = licensing::evaluate("aic", &ev);

    // `thumbnail.width/height` describe the full-resolution master.
    let (ow, oh) = (
        u32_of(&a["thumbnail"]["width"]),
        u32_of(&a["thumbnail"]["height"]),
    );
    let (w, h) = match (ow, oh) {
        (Some(w), Some(h)) => {
            let (w, h) = fit_long_edge(w, h, DOWNLOAD_EDGE);
            (Some(w), Some(h))
        }
        _ => (None, None),
    };
    let display = a["artist_display"].as_str().unwrap_or("");
    let artist = str_of(&a["artist_title"]).or_else(|| clean(display.lines().next()));
    let artist_bio = clean(display.lines().nth(1));
    let keywords: Vec<String> = ["subject_titles", "classification_titles"]
        .iter()
        .flat_map(|k| {
            a[*k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
        })
        .chain(a["style_title"].as_str().map(str::to_string))
        .collect();
    let kw: Vec<&str> = keywords.iter().map(String::as_str).collect();
    let mut extra = Map::new();
    if let Some(d) = str_of(&a["department_title"]) {
        extra.insert("department".into(), d.into());
    }
    if let (Some(w), Some(h)) = (ow, oh) {
        extra.insert("original_width".into(), w.into());
        extra.insert("original_height".into(), h.into());
    }
    let mut notes = Vec::new();
    if ow.is_some_and(|w| w.max(oh.unwrap_or(0)) > DOWNLOAD_EDGE) {
        notes.push(format!(
            "The museum offers downloads at {DOWNLOAD_EDGE} px; its master scan is larger."
        ));
    }
    Some(finish(
        Candidate {
            source: "aic".into(),
            source_id: id.clone(),
            locator: id.clone(),
            title: str_of(&a["title"]).unwrap_or_else(|| "Untitled".into()),
            artist,
            artist_bio,
            date: str_of(&a["date_display"]),
            medium: str_of(&a["medium_display"]),
            dimensions: str_of(&a["dimensions"]),
            description: clean(a["short_description"].as_str()),
            museum: "Art Institute of Chicago".into(),
            credit_line: str_of(&a["credit_line"]),
            accession: str_of(&a["main_reference_number"]),
            department: str_of(&a["department_title"]),
            page_url: Some(format!("https://www.artic.edu/artworks/{id}")),
            thumb_url: Some(format!("{IIIF}/{image_id}/full/400,/0/default.jpg")),
            preview_url: Some(format!("{IIIF}/{image_id}/full/843,/0/default.jpg")),
            width: w,
            height: h,
            decision: Some(decision),
            notes,
            plan: Some(Plan {
                url: format!(
                    "{IIIF}/{image_id}/full/!{DOWNLOAD_EDGE},{DOWNLOAD_EDGE}/0/default.jpg"
                ),
                headers: vec![(HEADERS[0].0, HEADERS[0].1.to_string())],
                credit: "The Art Institute of Chicago (CC0)".into(),
                kind: Some("painting".into()),
                extra,
            }),
            ..Default::default()
        },
        &kw,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_a_record() {
        let a = serde_json::json!({
            "id": 27992, "title": "A Sunday on La Grande Jatte — 1884", "image_id": "2d484387",
            "is_public_domain": true, "artist_title": "Georges Seurat", "artist_display": "Georges Seurat\nFrench, 1859–1891",
            "thumbnail": { "width": 9310, "height": 6237 }, "subject_titles": ["landscapes", "river"]
        });
        let c = candidate(&a).unwrap();
        assert_eq!(c.id, "aic:27992");
        assert_eq!(c.artist_bio.as_deref(), Some("French, 1859–1891"));
        assert_eq!((c.width, c.height), (Some(3000), Some(2010)));
        assert!(c.decision.as_ref().unwrap().allowed);
        assert!(c
            .plan
            .as_ref()
            .unwrap()
            .url
            .ends_with("/full/!3000,3000/0/default.jpg"));
        assert!(c.genres.contains(&"river-lake".to_string()));
        let mut b = a.clone();
        b["is_public_domain"] = false.into();
        assert!(!candidate(&b).unwrap().decision.unwrap().allowed);
    }
}
