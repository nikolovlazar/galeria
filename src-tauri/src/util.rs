use std::hash::{DefaultHasher, Hash, Hasher};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn hash_str(parts: &[&str]) -> String {
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Strip HTML tags and decode the handful of entities museum APIs use.
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&rsquo;", "\u{2019}")
        .replace("&lsquo;", "\u{2018}")
        .replace("&rdquo;", "\u{201d}")
        .replace("&ldquo;", "\u{201c}")
        .replace("&ndash;", "\u{2013}")
        .replace("&mdash;", "\u{2014}")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    collapse_ws(&out)
}

pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `"Ōhara Donshū"` → `"ohara-donshu"`: ASCII, lowercase, hyphenated, at most `max` characters.
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in deunicode::deunicode(s).to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if matches!(c, '\'' | '"' | '`') {
            // "L'Été" → "lete", not "l-ete"
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let mut out = out.trim_matches('-').to_string();
    if out.len() > max {
        out.truncate(max);
        if let Some(i) = out.rfind('-') {
            if i > max / 2 {
                out.truncate(i);
            }
        }
    }
    out.trim_matches('-').to_string()
}

/// Today's date (UTC) as `YYYY-MM-DD`.
pub fn today() -> String {
    // Civil-from-days (Howard Hinnant).
    let z = (now() / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs_and_dates() {
        assert_eq!(slug("Ōhara Donshū", 60), "ohara-donshu");
        assert_eq!(
            slug("The Cascades at Tivoli, with a Storm Approaching", 60),
            "the-cascades-at-tivoli-with-a-storm-approaching"
        );
        assert_eq!(slug("L'Été — Saint-Lazare", 60), "lete-saint-lazare");
        assert_eq!(slug("Явление Христа народу", 60), "iavlenie-khrista-narodu");
        assert_eq!(slug("!!!", 60), "");
        assert!(slug(&"word ".repeat(40), 60).len() <= 60);
        let t = today();
        assert!(
            t.len() == 10 && t.starts_with("20") && t.as_bytes()[4] == b'-',
            "{t}"
        );
    }

    #[test]
    fn strips_html() {
        assert_eq!(
            strip_html("<p>A <i>fine</i>&nbsp;day &amp; night</p>\n\n"),
            "A fine day & night"
        );
    }
}
