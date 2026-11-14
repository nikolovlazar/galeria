//! National Gallery of Art, Washington.
//!
//! The Gallery has no search API. It publishes its whole catalogue as CC0
//! open data (https://github.com/NationalGalleryOfArt/opendata), including a
//! per-image `openaccess` flag. Galeria downloads two of those CSV files once,
//! keeps a small index of the open-access paintings and searches it locally.

use super::*;
use crate::licensing::{self, Evidence};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

const DATA: &str = "https://raw.githubusercontent.com/NationalGalleryOfArt/opendata/main/data";
const IIIF_MAX: u32 = 4096;
const INDEX_VERSION: u32 = 2;
/// Open-access status is only as current as the catalogue: refuse downloads from a stale one.
pub const MAX_AGE_DAYS: u64 = 45;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Item {
    id: u32,
    title: String,
    artist: String,
    date: String,
    medium: String,
    dimensions: String,
    credit_line: String,
    accession: String,
    uuid: String,
    w: u32,
    h: u32,
    open: bool,
    /// The Gallery's own themes and keywords ("landscape", "Rivers", …) and school ("Dutch").
    #[serde(default)]
    terms: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    built_at: u64,
    items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogStatus {
    pub source: &'static str,
    pub ready: bool,
    pub built_at: Option<u64>,
    pub count: usize,
    pub stale: bool,
    pub size_hint: &'static str,
}

static INDEX: LazyLock<Mutex<Option<Arc<Index>>>> = LazyLock::new(|| Mutex::new(None));

fn index_path(cache: &Path) -> PathBuf {
    cache.join("catalogs").join("nga-index.json")
}

fn load(cache: &Path) -> Option<Arc<Index>> {
    let mut slot = INDEX.lock().unwrap();
    if slot.is_none() {
        let idx: Index = serde_json::from_slice(&std::fs::read(index_path(cache)).ok()?).ok()?;
        if idx.version != INDEX_VERSION {
            return None;
        }
        *slot = Some(Arc::new(idx));
    }
    slot.clone()
}

pub fn status(cache: &Path) -> CatalogStatus {
    let idx = load(cache);
    let age_days = idx
        .as_ref()
        .map(|i| crate::util::now().saturating_sub(i.built_at) / 86_400);
    CatalogStatus {
        source: "nga",
        ready: idx.is_some(),
        built_at: idx.as_ref().map(|i| i.built_at),
        count: idx.as_ref().map(|i| i.items.len()).unwrap_or(0),
        stale: age_days.is_some_and(|d| d > MAX_AGE_DAYS),
        size_hint: "about 35 MB",
    }
}

fn progress(ctx: &Ctx, stage: &str, received: u64, total: u64) {
    if let Some(shell) = ctx.app.as_ref().and_then(|app| app.shell()) {
        shell.catalog_progress(host::CatalogProgress {
            source: "nga".into(),
            stage: stage.into(),
            received,
            total,
        });
    }
}

async fn fetch_csv(
    ctx: &Ctx,
    name: &str,
    stage: &str,
    approx: u64,
    dir: &Path,
) -> Result<PathBuf, String> {
    let path = dir.join(name);
    let mut resp = ctx.http.get(&format!("{DATA}/{name}"), &[]).await?;
    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let (mut received, mut last) = (0u64, std::time::Instant::now());
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("The download of {name} was interrupted: {e}"))?
    {
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        received += chunk.len() as u64;
        if last.elapsed().as_millis() > 200 {
            last = std::time::Instant::now();
            progress(ctx, stage, received, approx.max(received));
        }
    }
    Ok(path)
}

/// Download the open dataset and (re)build the local index. Returns the number of paintings.
pub async fn prepare(ctx: &Ctx) -> Result<usize, String> {
    let dir = ctx.cache.join("catalogs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let images = fetch_csv(
        ctx,
        "published_images.csv",
        "Downloading the image list",
        92_000_000,
        &dir,
    )
    .await?;
    let objects = fetch_csv(
        ctx,
        "objects.csv",
        "Downloading the object records",
        83_000_000,
        &dir,
    )
    .await?;
    let terms = fetch_csv(
        ctx,
        "objects_terms.csv",
        "Downloading themes and keywords",
        20_500_000,
        &dir,
    )
    .await?;
    progress(ctx, "Indexing open-access paintings", 1, 1);
    let (i2, o2, t2) = (images.clone(), objects.clone(), terms.clone());
    let built = host::spawn_blocking(move || build(&i2, &o2, Some(&t2)))
        .await
        .map_err(|e| e.to_string())?;
    for f in [&images, &objects, &terms] {
        let _ = std::fs::remove_file(f);
    }
    let idx = built?;
    let n = idx.items.len();
    if n == 0 {
        return Err("The National Gallery dataset contained no open-access paintings; its format may have changed.".into());
    }
    crate::settings::write_json_atomic(&index_path(&ctx.cache), &idx).map_err(|e| e.to_string())?;
    *INDEX.lock().unwrap() = Some(Arc::new(idx));
    Ok(n)
}

fn columns(headers: &csv::StringRecord, names: &[&str]) -> Result<Vec<usize>, String> {
    names
        .iter()
        .map(|n| {
            headers.iter().position(|h| h == *n).ok_or_else(|| {
                format!("The National Gallery dataset no longer has a \"{n}\" column.")
            })
        })
        .collect()
}

fn build(images: &Path, objects: &Path, terms: Option<&Path>) -> Result<Index, String> {
    // objectid → primary image (uuid, width, height, open access?)
    let mut primary: HashMap<u32, (String, u32, u32, bool)> = HashMap::new();
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(images)
        .map_err(|e| e.to_string())?;
    let c = columns(
        rdr.headers().map_err(|e| e.to_string())?,
        &[
            "uuid",
            "viewtype",
            "width",
            "height",
            "openaccess",
            "depictstmsobjectid",
        ],
    )?;
    for rec in rdr.records().flatten() {
        if rec.get(c[1]) != Some("primary") {
            continue;
        }
        let Some(id) = rec.get(c[5]).and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let (w, h) = (
            rec.get(c[2]).and_then(|s| s.parse().ok()).unwrap_or(0),
            rec.get(c[3]).and_then(|s| s.parse().ok()).unwrap_or(0),
        );
        let open = rec.get(c[4]) == Some("1");
        if w > 0 && h > 0 && open {
            primary
                .entry(id)
                .or_insert_with(|| (rec.get(c[0]).unwrap_or("").to_string(), w, h, open));
        }
    }
    let mut items = Vec::new();
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(objects)
        .map_err(|e| e.to_string())?;
    let c = columns(
        rdr.headers().map_err(|e| e.to_string())?,
        &[
            "objectid",
            "title",
            "displaydate",
            "medium",
            "dimensions",
            "attribution",
            "creditline",
            "classification",
            "accessionnum",
            "isvirtual",
        ],
    )?;
    for rec in rdr.records().flatten() {
        if rec.get(c[7]) != Some("Painting") || rec.get(c[9]) == Some("1") {
            continue;
        }
        let Some(id) = rec.get(c[0]).and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let Some((uuid, w, h, open)) = primary.get(&id).cloned() else {
            continue;
        };
        let get = |i: usize| crate::util::collapse_ws(rec.get(c[i]).unwrap_or(""));
        items.push(Item {
            id,
            title: get(1),
            date: get(2),
            medium: get(3),
            dimensions: overall_dimensions(rec.get(c[4]).unwrap_or("")),
            artist: get(5),
            credit_line: get(6),
            accession: get(8),
            uuid,
            w,
            h,
            open,
            terms: Vec::new(),
        });
    }
    if let Some(path) = terms {
        let mut by_object: HashMap<u32, Vec<String>> =
            items.iter().map(|i| (i.id, Vec::new())).collect();
        let mut rdr = csv::ReaderBuilder::new()
            .flexible(true)
            .from_path(path)
            .map_err(|e| e.to_string())?;
        let c = columns(
            rdr.headers().map_err(|e| e.to_string())?,
            &["objectid", "termtype", "term"],
        )?;
        for rec in rdr.records().flatten() {
            if !matches!(rec.get(c[1]), Some("Theme" | "Keyword" | "School")) {
                continue;
            }
            let term = rec.get(c[2]).unwrap_or("").trim();
            // "Themes" / "Keywords" are the roots of the Gallery's term trees, not terms.
            if term.is_empty() || term == "Themes" || term == "Keywords" {
                continue;
            }
            if let Some(list) = rec
                .get(c[0])
                .and_then(|s| s.parse::<u32>().ok())
                .and_then(|id| by_object.get_mut(&id))
            {
                if list.len() < 24 && !list.iter().any(|t| t == term) {
                    list.push(term.to_string());
                }
            }
        }
        for i in &mut items {
            i.terms = by_object.remove(&i.id).unwrap_or_default();
        }
    }
    items.sort_by(|a, b| a.artist.cmp(&b.artist).then_with(|| a.title.cmp(&b.title)));
    Ok(Index {
        version: INDEX_VERSION,
        built_at: crate::util::now(),
        items,
    })
}

/// The dataset lists several measurements, one per line; the plate wants the overall size.
fn overall_dimensions(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let line = lines
        .iter()
        .find(|l| l.starts_with("overall"))
        .or(lines.first())
        .copied()
        .unwrap_or("");
    crate::util::collapse_ws(line.split_once(':').map(|(_, v)| v).unwrap_or(line))
}

/// The website's address for an object: `/artworks/<id>-<slug>`, where the slug is the
/// title with accents folded, quotes dropped and common short words removed.
pub fn page_url(id: u32, title: &str) -> String {
    const STOP: [&str; 27] = [
        "a", "an", "as", "at", "before", "but", "by", "for", "from", "is", "in", "into", "like",
        "of", "off", "on", "onto", "per", "since", "than", "the", "this", "that", "to", "up",
        "via", "with",
    ];
    let folded: String = deunicode::deunicode(title)
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '`'))
        .collect();
    let mut slug = String::new();
    for word in folded
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty() && !STOP.contains(w))
    {
        if slug.len() + word.len() + 1 > 100 {
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(word);
    }
    if slug.is_empty() {
        format!("https://www.nga.gov/artworks/{id}")
    } else {
        format!("https://www.nga.gov/artworks/{id}-{slug}")
    }
}

fn candidate(i: &Item) -> Candidate {
    let (w, h) = fit_long_edge(i.w, i.h, IIIF_MAX);
    let base = format!("https://api.nga.gov/iiif/{}", i.uuid);
    let mut ev = Evidence::new();
    ev.insert(
        "openaccess".into(),
        if i.open { "1" } else { "0" }.to_string(),
    );
    let mut extra = Map::new();
    let mut notes = Vec::new();
    if (w, h) != (i.w, i.h) {
        extra.insert("original_width".into(), i.w.into());
        extra.insert("original_height".into(), i.h.into());
        notes.push(format!("The Gallery's scan is {}×{} px, but its public image service returns at most {IIIF_MAX} px.", i.w, i.h));
    }
    let non_empty = |s: &str| (!s.is_empty()).then(|| s.to_string());
    finish(
        Candidate {
            source: "nga".into(),
            source_id: i.id.to_string(),
            locator: i.id.to_string(),
            title: if i.title.is_empty() {
                "Untitled".into()
            } else {
                i.title.clone()
            },
            artist: non_empty(&i.artist),
            date: non_empty(&i.date),
            medium: non_empty(&i.medium),
            dimensions: non_empty(&i.dimensions),
            museum: "National Gallery of Art, Washington".into(),
            credit_line: non_empty(&i.credit_line),
            accession: non_empty(&i.accession),
            page_url: Some(page_url(i.id, &i.title)),
            thumb_url: Some(format!("{base}/full/!400,400/0/default.jpg")),
            preview_url: Some(format!("{base}/full/!1200,1200/0/default.jpg")),
            width: Some(w),
            height: Some(h),
            decision: Some(licensing::evaluate("nga", &ev)),
            notes,
            plan: Some(Plan {
                url: format!("{base}/full/max/0/default.jpg"),
                credit: "Courtesy National Gallery of Art, Washington".into(),
                kind: Some("painting".into()),
                extra,
                ..Default::default()
            }),
            ..Default::default()
        },
        &i.terms.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

pub async fn search(ctx: &Ctx, q: &Query) -> Result<Page, String> {
    let Some(idx) = load(&ctx.cache) else {
        return Ok(Page {
            needs_catalog: true,
            notice: Some("The National Gallery has no search service. Galeria downloads its open dataset once and searches it on this computer.".into()),
            ..Default::default()
        });
    };
    let tokens: Vec<String> = q
        .text
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let offset: usize = q
        .cursor
        .as_deref()
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let matches: Vec<&Item> = idx
        .items
        .iter()
        .filter(|i| {
            if tokens.is_empty() {
                return true;
            }
            let hay = format!(
                "{} {} {} {} {}",
                i.title,
                i.artist,
                i.medium,
                i.date,
                i.terms.join(" ")
            )
            .to_lowercase();
            tokens.iter().all(|t| hay.contains(t.as_str()))
        })
        .collect();
    let total = matches.len();
    let filtered: Vec<Candidate> = matches
        .into_iter()
        .map(candidate)
        .filter(|c| passes(q, c))
        .collect();
    let shown = filtered.len();
    let items: Vec<Candidate> = filtered.into_iter().skip(offset).take(PAGE_SIZE).collect();
    let st = status(&ctx.cache);
    Ok(Page {
        items,
        total: Some(total as u64),
        next: (offset + PAGE_SIZE < shown).then(|| (offset + PAGE_SIZE).to_string()),
        notice: st.stale.then(|| "This copy of the National Gallery catalogue is more than six weeks old. Refresh it before downloading.".to_string()),
        hidden: total - shown,
        ..Default::default()
    })
}

pub async fn resolve(ctx: &Ctx, id: &str) -> Result<Candidate, String> {
    let idx =
        load(&ctx.cache).ok_or("The National Gallery catalogue has not been downloaded yet.")?;
    if status(&ctx.cache).stale {
        return Err("The National Gallery catalogue on this computer is out of date. Refresh it so the open-access status is current, then try again.".into());
    }
    let n: u32 = id
        .parse()
        .map_err(|_| "Not a National Gallery object number.".to_string())?;
    idx.items
        .iter()
        .find(|i| i.id == n)
        .map(candidate)
        .ok_or_else(|| {
            "The National Gallery no longer lists this work as an open-access painting.".to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_urls() {
        assert_eq!(
            page_url(100922, "A Pier Overlooking Dordrecht"),
            "https://www.nga.gov/artworks/100922-pier-overlooking-dordrecht"
        );
        assert_eq!(
            page_url(1, "The Voyage of Life: Childhood"),
            "https://www.nga.gov/artworks/1-voyage-life-childhood"
        );
        assert_eq!(
            page_url(2, "Saint-Lazare Station (L'Été)"),
            "https://www.nga.gov/artworks/2-saint-lazare-station-lete"
        );
        assert_eq!(page_url(3, ""), "https://www.nga.gov/artworks/3");
    }

    #[test]
    fn builds_an_index_from_the_open_data() {
        let dir = std::env::temp_dir().join(format!("galeria-nga-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("i.csv"),
            "uuid,iiifurl,iiifthumburl,viewtype,sequence,width,height,maxpixels,openaccess,created,modified,depictstmsobjectid,assistivetext\n\
             u-1,x,y,primary,0,18051,10417,,1,a,b,100,\"alt, with comma\"\n\
             u-2,x,y,primary,0,3000,4000,,0,a,b,200,\n\
             u-3,x,y,alternate,1,3000,4000,,1,a,b,100,\n\
             u-4,x,y,primary,0,5000,3000,,1,a,b,300,\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("o.csv"),
            "objectid,uuid,accessioned,accessionnum,locationid,title,displaydate,beginyear,endyear,visualbrowsertimespan,medium,dimensions,inscription,markings,attributioninverted,attribution,provenancetext,creditline,classification,subclassification,visualbrowserclassification,parentid,isvirtual,departmentabbr,portfolio,series,volume,watermarks,lastdetectedmodification,wikidataid,customprinturl\n\
             100,x,1,1940.2.1,,A Pier Overlooking Dordrecht,early 1640s,1640,1645,x,oil on panel,\"overall: 44.5 × 75.5 cm (17 1/2 × 29 3/4 in.)\nframed: 60 × 91 cm\",,,\"Cuyp, Aelbert\",Aelbert Cuyp,prov,Gift,Painting,,,,0,,,,,,,,\n\
             200,x,1,1.1,,Not Open,1900,1900,1900,x,oil,,,,,Someone,,Gift,Painting,,,,0,,,,,,,,\n\
             300,x,1,2.2,,A Print,1900,1900,1900,x,etching,,,,,Someone,,Gift,Print,,,,0,,,,,,,,\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("t.csv"),
            "termid,objectid,termtype,term,visualbrowsertheme,visualbrowserstyle\n1,100,Theme,Themes,,\n2,100,Theme,landscape,,\n3,100,Keyword,Rivers,,\n4,100,School,Dutch,,\n5,100,Technique,painted surface,,\n6,300,Theme,portrait,,\n",
        )
        .unwrap();
        let idx = build(
            &dir.join("i.csv"),
            &dir.join("o.csv"),
            Some(&dir.join("t.csv")),
        )
        .unwrap();
        assert_eq!(
            idx.items.len(),
            1,
            "only open-access paintings with a primary image"
        );
        assert_eq!(idx.items[0].terms, vec!["landscape", "Rivers", "Dutch"]);
        let c = candidate(&idx.items[0]);
        assert_eq!(c.genres, vec!["landscape", "river-lake"]);
        assert_eq!(c.id, "nga:100");
        assert_eq!(c.artist.as_deref(), Some("Aelbert Cuyp"));
        assert_eq!(
            c.dimensions.as_deref(),
            Some("44.5 × 75.5 cm (17 1/2 × 29 3/4 in.)")
        );
        assert_eq!((c.width, c.height), (Some(4096), Some(2364)));
        assert!(c.decision.as_ref().unwrap().allowed);
        assert_eq!(c.plan.as_ref().unwrap().extra["original_width"], 18051);
        assert_eq!(
            c.plan.as_ref().unwrap().url,
            "https://api.nga.gov/iiif/u-1/full/max/0/default.jpg"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
