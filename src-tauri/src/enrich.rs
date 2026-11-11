//! Optional online enrichment: museum APIs (Cleveland, Rijksmuseum), Wikidata
//! and Wikipedia fill in descriptions, artist nationality/life dates,
//! acquisition credit lines and English titles that the manifest lacks.
//! Results are cached on disk; the manifest itself is never modified.

use crate::util::{collapse_ws, non_empty, now, strip_html};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

const UA: &str = concat!(
    "Galeria/",
    env!("CARGO_PKG_VERSION"),
    " (desktop wallpaper app; https://github.com/nikolovlazar/galeria)"
);
/// Re-try failed lookups after a week; successful ones are kept for 90 days.
const RETRY_FAILED_AFTER: u64 = 7 * 24 * 3600;
const REFRESH_AFTER: u64 = 90 * 24 * 3600;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Enrichment {
    pub fetched_at: u64,
    pub ok: bool,
    pub title_en: Option<String>,
    pub description: Option<String>,
    /// Human readable origin of the description, e.g. "Wikipedia".
    pub description_source: Option<String>,
    pub description_license: Option<String>,
    pub description_lang: Option<String>,
    pub did_you_know: Option<String>,
    pub artist_bio: Option<String>,
    pub credit_line: Option<String>,
    pub accession: Option<String>,
    pub culture: Option<String>,
    pub technique: Option<String>,
    pub measurements: Option<String>,
    pub wikipedia_url: Option<String>,
    pub wikidata_id: Option<String>,
}

impl Enrichment {
    pub fn is_stale(&self) -> bool {
        let age = now().saturating_sub(self.fetched_at);
        if self.ok {
            age > REFRESH_AFTER
        } else {
            age > RETRY_FAILED_AFTER
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArtistInfo {
    pub bio: Option<String>,
    pub wikidata_id: Option<String>,
    pub fetched_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Store {
    pub works: HashMap<String, Enrichment>,
    pub artists: HashMap<String, ArtistInfo>,
}

/// Everything the enricher needs to know about a painting.
#[derive(Debug, Clone)]
pub struct Job {
    pub key: String,
    pub source: Option<String>,
    pub source_id: Option<String>,
    pub source_page: Option<String>,
    pub artist: Option<String>,
    pub title: String,
}

pub struct Enricher {
    client: reqwest::Client,
}

impl Enricher {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .timeout(Duration::from_secs(25))
            .build()
            .expect("http client");
        Self { client }
    }

    pub async fn enrich(&self, job: &Job, artists: &mut HashMap<String, ArtistInfo>) -> Enrichment {
        let mut e = Enrichment {
            fetched_at: now(),
            ..Default::default()
        };
        let source = job.source.as_deref().unwrap_or("");
        let sid = job.source_id.clone().unwrap_or_default();
        let mut creator_qid: Option<String> = None;
        let mut any_ok = false;

        match source {
            "cma" | "cleveland" if !sid.is_empty() => {
                if self.cleveland(&sid, &mut e).await.is_ok() {
                    any_ok = true;
                }
            }
            "rijks" | "rijksmuseum" => {
                if let Some(page) = job
                    .source_page
                    .as_deref()
                    .filter(|p| p.contains("id.rijksmuseum.nl"))
                {
                    if let Ok(q) = self.rijks(page, &mut e).await {
                        any_ok = true;
                        creator_qid = q;
                    }
                }
            }
            _ => {}
        }

        // Find the work on Wikidata through the museum's identifiers.
        let ident = match source {
            "nga" if !sid.is_empty() => {
                Some(format!("?item wdt:P4683 \"{}\" .", sparql_escape(&sid)))
            }
            "cma" | "cleveland" => e
                .accession
                .clone()
                .or(Some(sid.clone()))
                .filter(|s| !s.is_empty())
                .map(|acc| {
                    format!(
                        "?item wdt:P217 \"{}\" ; wdt:P195 wd:Q657415 .",
                        sparql_escape(&acc)
                    )
                }),
            "rijks" | "rijksmuseum" => e.accession.clone().map(|acc| {
                format!(
                    "?item wdt:P217 \"{}\" ; wdt:P195 wd:Q190804 .",
                    sparql_escape(&acc)
                )
            }),
            "met" if !sid.is_empty() => {
                Some(format!("?item wdt:P3634 \"{}\" .", sparql_escape(&sid)))
            }
            _ => None,
        };
        if let Some(ident) = ident {
            if let Ok(w) = self.wikidata_work(&ident).await {
                any_ok = true;
                if e.wikidata_id.is_none() {
                    e.wikidata_id = w.item;
                }
                if e.artist_bio.is_none() {
                    e.artist_bio = w.bio;
                }
                if creator_qid.is_none() {
                    creator_qid = w.creator;
                }
                if let Some(article) = w.article {
                    e.wikipedia_url = Some(article.clone());
                    // Prefer an English museum text; fall back to Wikipedia; Dutch is the last resort.
                    let needs_text =
                        e.description.is_none() || e.description_lang.as_deref() != Some("en");
                    if needs_text {
                        if let Ok(Some(extract)) = self.wikipedia_summary(&article).await {
                            e.description = Some(extract);
                            e.description_source = Some("Wikipedia".into());
                            e.description_license = Some("CC BY-SA 4.0".into());
                            e.description_lang = Some("en".into());
                        }
                    }
                }
            }
        }

        // Artist nationality + life dates, cached per artist.
        if e.artist_bio.is_none() {
            if let Some(name) = job.artist.as_deref().filter(|n| is_named_artist(n)) {
                let cached = artists
                    .get(name)
                    .filter(|a| now().saturating_sub(a.fetched_at) < REFRESH_AFTER);
                if let Some(a) = cached {
                    e.artist_bio = a.bio.clone();
                } else {
                    let info = match &creator_qid {
                        Some(q) => self.artist_by_qid(q).await,
                        None => self.artist_by_name(name).await,
                    };
                    if let Ok(info) = info {
                        e.artist_bio = info.bio.clone();
                        artists.insert(name.to_string(), info);
                    }
                }
            }
        }

        e.ok = any_ok;
        e
    }

    async fn get_json(&self, url: &str, accept: &str) -> Result<Value, String> {
        let res = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, accept)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("{url}: HTTP {}", res.status()));
        }
        res.json::<Value>().await.map_err(|e| e.to_string())
    }

    async fn cleveland(&self, id: &str, e: &mut Enrichment) -> Result<(), String> {
        let url = format!(
            "https://openaccess-api.clevelandart.org/api/artworks/{}",
            urlenc(id)
        );
        let v = self.get_json(&url, "application/json").await?;
        let d = &v["data"];
        let s = |k: &str| non_empty(d[k].as_str().map(strip_html));
        if let Some(desc) = s("description").or_else(|| s("wall_description")) {
            e.description = Some(desc);
            e.description_source = Some("Cleveland Museum of Art".into());
            e.description_license = Some("CC0".into());
            e.description_lang = Some("en".into());
        }
        e.did_you_know = s("did_you_know");
        e.credit_line = s("creditline");
        e.accession = s("accession_number");
        e.technique = s("technique");
        e.measurements = s("measurements");
        e.culture = d["culture"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|c| c.as_str())
            .map(collapse_ws);
        if let Some(desc) = d["creators"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|c| c["description"].as_str())
        {
            e.artist_bio = bio_from_parenthetical(desc);
        }
        Ok(())
    }

    /// Rijksmuseum Linked Art JSON-LD. Returns the creator's Wikidata QID when present.
    async fn rijks(&self, page: &str, e: &mut Enrichment) -> Result<Option<String>, String> {
        const EN: &str = "http://vocab.getty.edu/aat/300388277";
        const NL: &str = "http://vocab.getty.edu/aat/300388256";
        const DESCRIPTION: &str = "http://vocab.getty.edu/aat/300435452";
        const DIMENSIONS: &str = "http://vocab.getty.edu/aat/300435430";
        const OBJECT_NUMBER: &str = "http://vocab.getty.edu/aat/300312355";
        const PREFERRED_TITLE: &str = "http://vocab.getty.edu/aat/300404670";

        let v = self.get_json(page, "application/ld+json").await?;
        let has_class = |node: &Value, id: &str| {
            node["classified_as"]
                .as_array()
                .is_some_and(|a| a.iter().any(|c| c["id"].as_str() == Some(id)))
        };
        fn lang_of(node: &Value) -> Option<&str> {
            node["language"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|l| l["id"].as_str())
        }

        let mut desc_en = None;
        let mut desc_nl = None;
        let mut dims = Vec::new();
        for r in v["referred_to_by"].as_array().into_iter().flatten() {
            let content = r["content"]
                .as_str()
                .map(collapse_ws)
                .filter(|s| !s.is_empty());
            let Some(content) = content else { continue };
            if has_class(r, DESCRIPTION) {
                match lang_of(r) {
                    Some(EN) => desc_en = Some(content),
                    Some(NL) => desc_nl = Some(content),
                    _ => {}
                }
            } else if has_class(r, DIMENSIONS)
                && lang_of(r) == Some(EN)
                && !content.starts_with("depth")
            {
                dims.push(content);
            }
        }
        if let Some(d) = desc_en {
            e.description = Some(d);
            e.description_lang = Some("en".into());
        } else if let Some(d) = desc_nl {
            e.description = Some(d);
            e.description_lang = Some("nl".into());
        }
        if e.description.is_some() {
            e.description_source = Some("Rijksmuseum".into());
            e.description_license = Some("CC0".into());
        }
        if !dims.is_empty() {
            e.measurements = Some(dims.join("; "));
        }
        for idn in v["identified_by"].as_array().into_iter().flatten() {
            if idn["type"] == "Identifier" && has_class(idn, OBJECT_NUMBER) {
                e.accession = idn["content"].as_str().map(str::to_string);
            }
            if idn["type"] == "Name" && e.title_en.is_none() && lang_of(idn) == Some(EN) {
                let preferred = idn["classified_as"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|c| c["id"].as_str() == Some(PREFERRED_TITLE)));
                if preferred {
                    e.title_en = idn["content"].as_str().map(collapse_ws);
                }
            }
        }
        let mut qid = None;
        'outer: for part in v["produced_by"]["part"].as_array().into_iter().flatten() {
            for who in part["carried_out_by"].as_array().into_iter().flatten() {
                for eq in who["equivalent"].as_array().into_iter().flatten() {
                    if let Some(id) = eq["id"]
                        .as_str()
                        .and_then(|s| s.strip_prefix("http://www.wikidata.org/entity/"))
                    {
                        qid = Some(id.to_string());
                        break 'outer;
                    }
                }
            }
        }
        Ok(qid)
    }

    async fn sparql(&self, query: &str) -> Result<Vec<Value>, String> {
        let res = self
            .client
            .get("https://query.wikidata.org/sparql")
            .query(&[("query", query), ("format", "json")])
            .header(reqwest::header::ACCEPT, "application/sparql-results+json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("wikidata: HTTP {}", res.status()));
        }
        let v: Value = res.json().await.map_err(|e| e.to_string())?;
        Ok(v["results"]["bindings"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }

    async fn wikidata_work(&self, ident: &str) -> Result<WorkInfo, String> {
        let q = format!(
            r#"SELECT ?item ?article ?creator ?birth ?death ?demonym WHERE {{
  {ident}
  OPTIONAL {{ ?article schema:about ?item ; schema:isPartOf <https://en.wikipedia.org/> . }}
  OPTIONAL {{ ?item wdt:P170 ?creator .
    OPTIONAL {{ ?creator wdt:P569 ?birth }} OPTIONAL {{ ?creator wdt:P570 ?death }}
    OPTIONAL {{ ?creator wdt:P27 ?country . ?country wdt:P1549 ?demonym . FILTER(LANG(?demonym) = "en") }} }}
}} LIMIT 20"#
        );
        let rows = self.sparql(&q).await?;
        if rows.is_empty() {
            return Err("not on wikidata".into());
        }
        let val = |r: &Value, k: &str| r[k]["value"].as_str().map(str::to_string);
        let item = rows.iter().find_map(|r| val(r, "item")).map(|u| qid_of(&u));
        let article = rows.iter().find_map(|r| val(r, "article"));
        let creator = rows
            .iter()
            .find_map(|r| val(r, "creator"))
            .filter(|u| u.contains("/entity/Q"))
            .map(|u| qid_of(&u));
        Ok(WorkInfo {
            item,
            article,
            creator,
            bio: bio_from_rows(&rows),
        })
    }

    async fn artist_by_qid(&self, qid: &str) -> Result<ArtistInfo, String> {
        let q = format!(
            r#"SELECT ?birth ?death ?demonym WHERE {{
  OPTIONAL {{ wd:{qid} wdt:P569 ?birth }} OPTIONAL {{ wd:{qid} wdt:P570 ?death }}
  OPTIONAL {{ wd:{qid} wdt:P27 ?country . ?country wdt:P1549 ?demonym . FILTER(LANG(?demonym) = "en") }}
}} LIMIT 20"#
        );
        let rows = self.sparql(&q).await?;
        Ok(ArtistInfo {
            bio: bio_from_rows(&rows),
            wikidata_id: Some(qid.to_string()),
            fetched_at: now(),
        })
    }

    async fn artist_by_name(&self, name: &str) -> Result<ArtistInfo, String> {
        let res = self
            .client
            .get("https://www.wikidata.org/w/api.php")
            .query(&[
                ("action", "wbsearchentities"),
                ("search", name),
                ("language", "en"),
                ("type", "item"),
                ("limit", "7"),
                ("format", "json"),
            ])
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = res.json().await.map_err(|e| e.to_string())?;
        const ARTISTIC: &[&str] = &[
            "painter",
            "artist",
            "draughtsman",
            "draftsman",
            "printmaker",
            "illustrator",
            "engraver",
            "watercolorist",
            "sculptor",
        ];
        let hit = v["search"].as_array().into_iter().flatten().find(|r| {
            let d = r["description"].as_str().unwrap_or("").to_lowercase();
            ARTISTIC.iter().any(|w| d.contains(w))
        });
        match hit.and_then(|h| h["id"].as_str()) {
            Some(qid) => self.artist_by_qid(qid).await,
            None => Ok(ArtistInfo {
                bio: None,
                wikidata_id: None,
                fetched_at: now(),
            }),
        }
    }

    async fn wikipedia_summary(&self, article_url: &str) -> Result<Option<String>, String> {
        let title = article_url.rsplit("/wiki/").next().unwrap_or("");
        if title.is_empty() {
            return Ok(None);
        }
        let url = format!("https://en.wikipedia.org/api/rest_v1/page/summary/{title}");
        let v = self.get_json(&url, "application/json").await?;
        Ok(non_empty(v["extract"].as_str().map(collapse_ws)))
    }
}

struct WorkInfo {
    item: Option<String>,
    article: Option<String>,
    creator: Option<String>,
    bio: Option<String>,
}

fn qid_of(uri: &str) -> String {
    uri.rsplit('/').next().unwrap_or(uri).to_string()
}

fn urlenc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn sparql_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_named_artist(name: &str) -> bool {
    let n = name.to_lowercase();
    !(n.is_empty()
        || n.starts_with("unknown")
        || n.starts_with("anonymous")
        || n.contains("unattributed")
        || n.contains("workshop")
        || n.contains("school of")
        || n.contains("follower of"))
}

/// "Claude Monet (French, 1840–1926)" → "French, 1840–1926"
fn bio_from_parenthetical(s: &str) -> Option<String> {
    let start = s.find('(')?;
    let end = s.rfind(')')?;
    (end > start + 1).then(|| collapse_ws(&s[start + 1..end]))
}

fn year_of(date: &str) -> Option<String> {
    // Wikidata times look like "1816-03-22T00:00:00Z" or "-0450-01-01T00:00:00Z".
    let (neg, rest) = match date.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, date.strip_prefix('+').unwrap_or(date)),
    };
    let y: i64 = rest.split('-').next()?.parse().ok()?;
    Some(if neg {
        format!("{y} BCE")
    } else {
        y.to_string()
    })
}

fn bio_from_rows(rows: &[Value]) -> Option<String> {
    let val = |r: &Value, k: &str| r[k]["value"].as_str().map(str::to_string);
    let birth = rows
        .iter()
        .find_map(|r| val(r, "birth"))
        .and_then(|d| year_of(&d));
    let death = rows
        .iter()
        .find_map(|r| val(r, "death"))
        .and_then(|d| year_of(&d));
    // Several demonyms are common ("American", "Americans"); prefer the adjective.
    let mut demonyms: Vec<String> = rows.iter().filter_map(|r| val(r, "demonym")).collect();
    demonyms.sort_by_key(|d| (d.ends_with('s') && !d.ends_with("ss"), d.len()));
    let demonym = demonyms.into_iter().next();
    let years = match (birth, death) {
        (Some(b), Some(d)) => Some(format!("{b}\u{2013}{d}")),
        (Some(b), None) => Some(format!("born {b}")),
        (None, Some(d)) => Some(format!("died {d}")),
        (None, None) => None,
    };
    match (demonym, years) {
        (Some(n), Some(y)) => Some(format!("{n}, {y}")),
        (Some(n), None) => Some(n),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_cleveland_bio() {
        assert_eq!(
            bio_from_parenthetical("Claude Monet (French, 1840-1926)").as_deref(),
            Some("French, 1840-1926")
        );
        assert_eq!(bio_from_parenthetical("Anonymous"), None);
    }

    #[test]
    fn builds_bio_from_wikidata_rows() {
        let rows = vec![
            json!({"birth": {"value": "1816-03-22T00:00:00Z"}, "death": {"value": "1872-12-14T00:00:00Z"}, "demonym": {"value": "Americans"}}),
            json!({"birth": {"value": "1816-03-22T00:00:00Z"}, "demonym": {"value": "American"}}),
        ];
        assert_eq!(
            bio_from_rows(&rows).as_deref(),
            Some("American, 1816\u{2013}1872")
        );
    }
}
