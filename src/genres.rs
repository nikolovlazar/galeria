//! Genres: a small fixed taxonomy (`resources/genres.json`) that paintings are
//! sorted into. Manifest `tags`, `kind` and `category` decide first; paintings
//! without any are classified from words in their title and medium.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genre {
    pub id: String,
    pub name: String,
    /// Manifest tags that mean this genre. The first is written for downloads.
    pub tags: Vec<String>,
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// What to type into a museum's search box to browse this genre.
    pub search: String,
}

#[derive(Deserialize)]
struct File {
    genres: Vec<Genre>,
}

static GENRES: LazyLock<Vec<Genre>> = LazyLock::new(|| {
    serde_json::from_str::<File>(include_str!("../resources/genres.json"))
        .expect("resources/genres.json is invalid")
        .genres
});

pub const OTHER: &str = "other";

pub fn all() -> &'static [Genre] {
    &GENRES
}

pub fn get(id: &str) -> Option<&'static Genre> {
    GENRES.iter().find(|g| g.id == id)
}

pub fn name(id: &str) -> &str {
    get(id)
        .map(|g| g.name.as_str())
        .unwrap_or(if id == OTHER { "Other" } else { id })
}

/// Lowercase, punctuation → spaces, padded so `" word "` matches whole words.
fn haystack(texts: &[&str]) -> String {
    let mut s = String::from(" ");
    for t in texts {
        for c in t.chars() {
            if c.is_alphanumeric() {
                s.extend(c.to_lowercase());
            } else if c == '-' && (s.ends_with(" saint") || s.ends_with(" sainte")) {
                // "Gare Saint-Lazare", "Mont Sainte-Victoire": a place, not a saint.
                let cut = s.rfind(' ').map(|i| i + 1).unwrap_or(0);
                s.truncate(cut);
            } else if !s.ends_with(' ') {
                s.push(' ');
            }
        }
        if !s.ends_with(' ') {
            s.push(' ');
        }
    }
    s
}

fn by_keywords(texts: &[&str]) -> Vec<String> {
    let hay = haystack(texts);
    // Whole words only, singular or plural ("Landscapes" is a landscape; "seated" is not the sea).
    let found = |k: &str| {
        let needle = haystack(&[k]);
        hay.contains(&needle) || hay.contains(&format!("{}s ", needle.trim_end()))
    };
    GENRES
        .iter()
        .filter(|g| g.keywords.iter().any(|k| found(k)))
        .map(|g| g.id.clone())
        .collect()
}

/// Genres of a painting. Explicit manifest labels win; words are the fallback.
pub fn classify(
    tags: &[String],
    kind: Option<&str>,
    category: Option<&str>,
    texts: &[&str],
) -> Vec<String> {
    let has = |list: &[String], v: &str| list.iter().any(|x| x.eq_ignore_ascii_case(v));
    let mut out: Vec<String> = GENRES
        .iter()
        .filter(|g| {
            tags.iter().any(|t| has(&g.tags, t))
                || kind.is_some_and(|k| has(&g.kinds, k))
                || category.is_some_and(|c| has(&g.categories, c))
        })
        .map(|g| g.id.clone())
        .collect();
    if out.is_empty() {
        out = by_keywords(texts);
    }
    out
}

/// Manifest tags to write for a set of genres (the vocabulary existing manifests use).
pub fn tags_for(genres: &[String]) -> Vec<String> {
    genres
        .iter()
        .filter_map(|g| get(g))
        .filter_map(|g| g.tags.first().cloned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_win_over_words() {
        let g = classify(
            &["seascape".into()],
            Some("painting"),
            Some("general"),
            &["A Mountain Valley"],
        );
        assert_eq!(g, vec!["seascape"]);
        let g = classify(&[], Some("icon"), Some("orthodox"), &["Saint George"]);
        assert_eq!(g, vec!["orthodox"]);
    }

    #[test]
    fn words_are_whole() {
        let g = classify(
            &[],
            None,
            None,
            &["Winter Landscape with Skaters", "oil on panel"],
        );
        assert!(
            g.contains(&"winter".to_string()) && g.contains(&"landscape".to_string()),
            "{g:?}"
        );
        // "seated" must not match "sea", "nice" must not match "ice".
        assert!(classify(&[], None, None, &["Seated Woman in Nice"]).is_empty());
        assert_eq!(
            classify(&[], None, None, &["Still Life with Lemons"]),
            vec!["still-life"]
        );
    }

    #[test]
    fn tags_round_trip() {
        assert_eq!(
            tags_for(&["figures".into(), "landscape".into()]),
            vec!["figures-genre", "landscape"]
        );
        for g in all() {
            assert!(!g.tags.is_empty() && !g.search.is_empty(), "{}", g.id);
        }
    }
}
