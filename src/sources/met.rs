//! The Metropolitan Museum of Art — Collection API (https://metmuseum.github.io/).

use super::*;
use crate::licensing::{self, Evidence};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

const API: &str = "https://collectionapi.metmuseum.org/public/collection";
/// Object records looked up this session (the API blocks clients that repeat themselves).
static OBJECTS: LazyLock<Mutex<HashMap<String, Value>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

async fn object(http: &Http, id: &str) -> Result<Value, String> {
    if let Some(v) = OBJECTS.lock().unwrap().get(id) {
        return Ok(v.clone());
    }
    let v = http
        .json(&format!("{API}/v1/objects/{}", enc(id)), &[])
        .await?;
    let mut cache = OBJECTS.lock().unwrap();
    if cache.len() > 3000 {
        cache.clear();
    }
    cache.insert(id.to_string(), v.clone());
    Ok(v)
}

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let offset: u64 = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    // The Met's search needs a term; with none typed, browse its paintings.
    let text = if q.text.trim().is_empty() {
        "painting"
    } else {
        q.text.trim()
    };
    let url = format!(
        "{API}/v1.1/search?hasImages=true&medium=Paintings&limit={PAGE_SIZE}&offset={offset}&q={}",
        enc(text)
    );
    let v = ctx.http.json(&url, &[]).await?;
    let total = v["total"].as_u64().unwrap_or(0);
    let ids: Vec<String> = v["objectIDs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(str_of)
        .collect();
    let limit = licensing::source("met")
        .map(|s| s.etiquette.search_concurrency)
        .unwrap_or(4);
    let http = ctx.http.clone();
    let records = map_concurrent(ids, limit, move |id| {
        let http = http.clone();
        async move { object(&http, &id).await }
    })
    .await;
    let failed = records.iter().filter(|r| r.is_err()).count();
    let all: Vec<Candidate> = records
        .iter()
        .filter_map(|r| r.as_ref().ok())
        .filter_map(candidate)
        .collect();
    let fetched = all.len();
    // The search cannot filter on rights, so works that are not open access are dropped here.
    let items: Vec<Candidate> = all
        .into_iter()
        .filter(|c| c.decision.as_ref().is_some_and(|d| d.allowed) && passes(q, c))
        .collect();
    Ok(Page {
        hidden: fetched - items.len(),
        items,
        total: Some(total),
        next: (offset + (PAGE_SIZE as u64) < total.min(10_000))
            .then(|| (offset + PAGE_SIZE as u64).to_string()),
        notice: (failed > 0).then(|| {
            format!("{failed} records could not be loaded; The Met may be limiting requests.")
        }),
        ..Default::default()
    })
}

pub async fn resolve(ctx: &Ctx, id: &str) -> Result<Candidate, String> {
    // Always a fresh read before a download.
    OBJECTS.lock().unwrap().remove(id);
    let v = object(&ctx.http, id).await?;
    candidate(&v).ok_or_else(|| "The Met has no open-access image for this work.".to_string())
}

fn candidate(o: &Value) -> Option<Candidate> {
    let id = str_of(&o["objectID"])?;
    let image = o["primaryImage"].as_str().filter(|s| !s.is_empty());
    let mut ev = Evidence::new();
    ev.insert(
        "isPublicDomain".into(),
        o["isPublicDomain"].as_bool().unwrap_or(false).to_string(),
    );
    let decision = licensing::evaluate("met", &ev);
    // Without an image there is nothing to show or download.
    let image = image?.to_string();

    // Physical size gives the shape; the API does not state pixel dimensions.
    let aspect = o["measurements"].as_array().and_then(|m| {
        m.iter().find_map(|e| {
            let d = &e["elementMeasurements"];
            match (d["Width"].as_f64(), d["Height"].as_f64()) {
                (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(w / h),
                _ => None,
            }
        })
    });
    let keywords: Vec<String> = o["tags"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["term"].as_str().map(str::to_string))
        .chain(
            [
                o["classification"].as_str(),
                o["objectName"].as_str(),
                o["culture"].as_str(),
            ]
            .into_iter()
            .flatten()
            .map(str::to_string),
        )
        .collect();
    let kw: Vec<&str> = keywords.iter().map(String::as_str).collect();
    let nationality = str_of(&o["artistNationality"]);
    let bio = str_of(&o["artistDisplayBio"]);
    let mut extra = Map::new();
    if let Some(d) = str_of(&o["department"]) {
        extra.insert("department".into(), d.into());
    }
    Some(finish(
        Candidate {
            source: "met".into(),
            source_id: id.clone(),
            locator: id.clone(),
            title: str_of(&o["title"]).unwrap_or_else(|| "Untitled".into()),
            artist: str_of(&o["artistDisplayName"]),
            artist_bio: bio.or(nationality),
            date: str_of(&o["objectDate"]),
            medium: str_of(&o["medium"]),
            dimensions: str_of(&o["dimensions"]),
            museum: "The Metropolitan Museum of Art, New York".into(),
            credit_line: str_of(&o["creditLine"]),
            accession: str_of(&o["accessionNumber"]),
            department: str_of(&o["department"]),
            page_url: str_of(&o["objectURL"]).or_else(|| {
                Some(format!(
                    "https://www.metmuseum.org/art/collection/search/{id}"
                ))
            }),
            thumb_url: o["primaryImageSmall"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| Some(image.clone())),
            preview_url: o["primaryImageSmall"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            aspect,
            decision: Some(decision),
            notes: vec![
                "The Met's open-access files are up to 4,000 px on the long edge.".to_string(),
            ],
            plan: Some(Plan {
                url: image,
                credit: "The Metropolitan Museum of Art (CC0)".into(),
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

    #[test]
    fn maps_a_record() {
        let o = serde_json::json!({
            "objectID": 10181, "isPublicDomain": true, "primaryImage": "https://images.metmuseum.org/x/original/ap29.35.jpg",
            "primaryImageSmall": "https://images.metmuseum.org/x/web-large/ap29.35.jpg", "title": "Landscape",
            "artistDisplayName": "Ralph Albert Blakelock", "artistDisplayBio": "1847–1919", "objectDate": "1885–95",
            "medium": "Oil on canvas", "objectURL": "https://www.metmuseum.org/art/collection/search/10181",
            "measurements": [{ "elementName": "Overall", "elementMeasurements": { "Height": 68.6, "Width": 94.9 } }],
            "tags": [{ "term": "Landscapes" }, { "term": "Trees" }]
        });
        let c = candidate(&o).unwrap();
        assert_eq!(c.id, "met:10181");
        assert!(c.decision.as_ref().unwrap().allowed);
        assert_eq!(c.aspect, Some(1.383));
        assert!(c.genres.contains(&"landscape".to_string()));
        let mut restricted = o.clone();
        restricted["isPublicDomain"] = false.into();
        assert!(!candidate(&restricted).unwrap().decision.unwrap().allowed);
        let mut no_image = o.clone();
        no_image["primaryImage"] = "".into();
        assert!(candidate(&no_image).is_none());
    }
}
