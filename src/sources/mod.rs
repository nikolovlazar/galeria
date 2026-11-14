//! Museum sources Galeria can browse and download from.
//!
//! Every provider turns its museum's records into [`Candidate`]s. A candidate
//! carries the [`Decision`] made by `licensing::evaluate` from the fields the
//! museum itself publishes, and a private download [`Plan`]. Nothing here
//! trusts the window: a download always re-reads the record from the museum
//! via [`resolve`] and re-evaluates the license.

pub mod aic;
pub mod cma;
pub mod commons;
pub mod met;
pub mod nga;
pub mod rijks;

use crate::host::{self, AppHandle};
use crate::licensing::Decision;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

pub const UA: &str = concat!(
    "Galeria/",
    env!("CARGO_PKG_VERSION"),
    " (desktop wallpaper app; https://github.com/nikolovlazar/galeria)"
);

pub const PAGE_SIZE: usize = 36;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Candidate {
    /// `source:source_id`, the same key the manifest uses.
    pub id: String,
    pub source: String,
    pub source_id: String,
    /// What the provider needs to find this record again (opaque to the UI).
    pub locator: String,
    pub title: String,
    pub title_original: Option<String>,
    pub artist: Option<String>,
    pub artist_bio: Option<String>,
    pub date: Option<String>,
    pub medium: Option<String>,
    pub dimensions: Option<String>,
    pub description: Option<String>,
    pub museum: String,
    pub credit_line: Option<String>,
    pub accession: Option<String>,
    pub department: Option<String>,
    pub page_url: Option<String>,
    pub thumb_url: Option<String>,
    pub preview_url: Option<String>,
    /// Pixel size of the file Galeria would save, when the museum states it.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Width ÷ height of the work, from pixels or physical measurements.
    pub aspect: Option<f64>,
    /// Size of the download, when the museum states it.
    pub download_bytes: Option<u64>,
    pub genres: Vec<String>,
    pub decision: Option<Decision>,
    pub in_library: bool,
    pub notes: Vec<String>,
    #[serde(skip)]
    pub plan: Option<Plan>,
}

/// How to fetch a candidate's image and what to record about it.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    /// Image credit for the plate and the manifest.
    pub credit: String,
    /// `painting`, `icon`, `fresco`, …
    pub kind: Option<String>,
    /// Extra manifest fields (`persistent_url`, `commons_file`, …).
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Query {
    pub source: String,
    pub text: String,
    /// Opaque continuation returned by the previous page.
    pub cursor: Option<String>,
    /// Commons only: restrict to works held by this institution.
    pub museum: Option<String>,
    pub landscape_only: bool,
    pub min_long_edge: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub items: Vec<Candidate>,
    /// Matches at the museum (before Galeria's own format/size filters).
    pub total: Option<u64>,
    pub next: Option<String>,
    pub notice: Option<String>,
    /// The source needs its catalogue downloaded before it can be searched.
    pub needs_catalog: bool,
    /// Results on this page that the filters hid.
    pub hidden: usize,
}

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
}

impl Default for Http {
    fn default() -> Self {
        Self::new()
    }
}

impl Http {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(UA)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .expect("http client");
        Self { client }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub async fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<reqwest::Response, String> {
        let mut req = self.client.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let host = url.split('/').nth(2).unwrap_or("the server").to_string();
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                format!("{host} took too long to answer.")
            } else if e.is_connect() {
                format!("Could not reach {host}. Check your internet connection.")
            } else {
                format!("Request to {host} failed: {e}")
            }
        })?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        Err(match status.as_u16() {
            403 | 429 => format!(
                "{host} is limiting requests right now (HTTP {}). Try again in a few minutes.",
                status.as_u16()
            ),
            404 => format!("{host} has no such record (HTTP 404)."),
            code => format!("{host} answered with HTTP {code}."),
        })
    }

    pub async fn json(&self, url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
        let resp = self.get(url, headers).await?;
        let text = resp.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|_| {
            let host = url.split('/').nth(2).unwrap_or("the server");
            format!(
                "{host} sent something that is not JSON (it may be blocking automated requests)."
            )
        })
    }

    pub async fn text(&self, url: &str, headers: &[(&str, &str)]) -> Result<String, String> {
        self.get(url, headers)
            .await?
            .text()
            .await
            .map_err(|e| e.to_string())
    }
}

pub struct Ctx {
    pub http: Http,
    /// App cache dir (catalogues live in `<cache>/catalogs`).
    pub cache: PathBuf,
    pub app: Option<AppHandle>,
    /// Museums already named in the library, so new works join them under the same name.
    pub museums: Vec<String>,
}

pub fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Percent-encode one path segment, leaving the unreserved characters readable.
pub fn enc_path(s: &str) -> String {
    const SEGMENT: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    utf8_percent_encode(s, SEGMENT).to_string()
}

pub fn clean(s: Option<&str>) -> Option<String> {
    s.map(crate::util::strip_html).filter(|s| !s.is_empty())
}

pub fn str_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => clean(Some(s)),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn u32_of(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n as u32),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| *n > 0)
}

/// "Jan Gossaert (Flemish, c. 1475/78–1532)" → ("Jan Gossaert", "Flemish, c. 1475/78–1532")
pub fn split_artist(s: &str) -> (String, Option<String>) {
    match s.rfind(" (") {
        Some(i) if s.ends_with(')') => (
            s[..i].trim().to_string(),
            Some(s[i + 2..s.len() - 1].trim().to_string()).filter(|b| !b.is_empty()),
        ),
        _ => (s.trim().to_string(), None),
    }
}

/// Size after fitting `w`×`h` inside a `max` long edge (never upscaling).
pub fn fit_long_edge(w: u32, h: u32, max: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= max || long == 0 {
        return (w, h);
    }
    let s = max as f64 / long as f64;
    (
        ((w as f64) * s).round().max(1.0) as u32,
        ((h as f64) * s).round().max(1.0) as u32,
    )
}

/// Apply the Discover filters where the facts are known; unknowns pass.
pub fn passes(q: &Query, c: &Candidate) -> bool {
    if q.landscape_only {
        if let Some(a) = c.aspect {
            if a < 1.15 {
                return false;
            }
        }
    }
    if q.min_long_edge > 0 {
        if let (Some(w), Some(h)) = (c.width, c.height) {
            if w.max(h) < q.min_long_edge {
                return false;
            }
        }
    }
    true
}

pub fn finish(mut c: Candidate, keywords: &[&str]) -> Candidate {
    c.id = format!("{}:{}", c.source, c.source_id);
    if c.aspect.is_none() {
        if let (Some(w), Some(h)) = (c.width, c.height) {
            c.aspect = Some(w as f64 / h as f64);
        }
    }
    if let Some(a) = c.aspect {
        c.aspect = Some((a * 1000.0).round() / 1000.0);
    }
    if c.genres.is_empty() {
        let mut texts: Vec<&str> = vec![c.title.as_str(), c.medium.as_deref().unwrap_or("")];
        texts.extend_from_slice(keywords);
        c.genres = crate::genres::classify(&[], None, None, &texts);
    }
    c
}

/// Run `f` over `items`, at most `limit` at a time, keeping the input order.
pub async fn map_concurrent<I, T, F, Fut>(items: Vec<I>, limit: usize, f: F) -> Vec<T>
where
    I: Send + 'static,
    T: Send + 'static,
    F: Fn(I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let sem = Arc::new(Semaphore::new(limit.max(1)));
    let f = Arc::new(f);
    let handles: Vec<_> = items
        .into_iter()
        .map(|item| {
            let sem = sem.clone();
            let f = f.clone();
            host::spawn(async move {
                let _permit = sem.acquire_owned().await;
                f(item).await
            })
        })
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}

pub const SOURCES: [&str; 6] = ["nga", "cma", "rijks", "met", "aic", "commons"];

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    match q.source.as_str() {
        "cma" => cma::search(ctx, q).await,
        "aic" => aic::search(ctx, q).await,
        "met" => met::search(ctx, q).await,
        "rijks" => rijks::search(ctx, q).await,
        "nga" => nga::search(ctx, q).await,
        "commons" => commons::search(ctx, q).await,
        other => Err(format!("Unknown source \"{other}\"")),
    }
}

/// Re-read one record from its museum (used right before a download).
pub async fn resolve(ctx: &Ctx, source: &str, locator: &str) -> Result<Candidate, String> {
    match source {
        "cma" => cma::resolve(ctx, locator).await,
        "aic" => aic::resolve(ctx, locator).await,
        "met" => met::resolve(ctx, locator).await,
        "rijks" => rijks::resolve(ctx, locator).await,
        "nga" => nga::resolve(ctx, locator).await,
        "commons" => commons::resolve(ctx, locator).await,
        other => Err(format!("Unknown source \"{other}\"")),
    }
}

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(
            split_artist("Jan Gossaert (Flemish, c. 1475/78–1532)"),
            (
                "Jan Gossaert".into(),
                Some("Flemish, c. 1475/78–1532".into())
            )
        );
        assert_eq!(split_artist("Anonymous"), ("Anonymous".into(), None));
        assert_eq!(fit_long_edge(9310, 6237, 3000), (3000, 2010));
        assert_eq!(fit_long_edge(2000, 1000, 3000), (2000, 1000));
        let q = Query {
            landscape_only: true,
            min_long_edge: 3000,
            ..Default::default()
        };
        let c = |w, h| Candidate {
            width: Some(w),
            height: Some(h),
            aspect: Some(w as f64 / h as f64),
            ..Default::default()
        };
        assert!(passes(&q, &c(4000, 3000)));
        assert!(!passes(&q, &c(3000, 4000)), "portrait");
        assert!(!passes(&q, &c(2000, 1200)), "too small");
        assert!(
            passes(&q, &Candidate::default()),
            "unknown sizes are not hidden"
        );
    }
}
