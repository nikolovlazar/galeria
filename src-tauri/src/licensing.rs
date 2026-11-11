//! Licensing rules for downloads.
//!
//! The rules themselves live in `resources/licensing.json` (compiled into the
//! app): which licenses Galeria accepts, and for every source which field of
//! the museum's own record proves that an image is open access. This module
//! only evaluates them. A download is refused unless [`evaluate`] says yes,
//! and the evidence it was based on is written to the manifest next to the
//! painting.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::LazyLock;

const RULES_JSON: &str = include_str!("../resources/licensing.json");

static RULES: LazyLock<Rules> = LazyLock::new(|| {
    serde_json::from_str(RULES_JSON).expect("resources/licensing.json is invalid")
});

pub fn rules() -> &'static Rules {
    &RULES
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rules {
    pub version: u32,
    pub reviewed: String,
    pub policy: String,
    pub disclaimer: String,
    pub licenses: Vec<License>,
    pub sources: Vec<SourceRule>,
    #[serde(default)]
    pub museums: Vec<MuseumNote>,
    pub fallback: Fallback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub id: String,
    pub label: String,
    pub allowed: bool,
    #[serde(default)]
    pub attribution: bool,
    #[serde(default)]
    pub share_alike: bool,
    #[serde(default)]
    pub url: Option<String>,
    /// Why a license is refused.
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub obligations: Vec<String>,
    #[serde(default)]
    pub exact: Vec<String>,
    #[serde(default)]
    pub prefixes: Vec<String>,
    #[serde(default)]
    pub contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRule {
    pub id: String,
    pub name: String,
    /// Museum name as Galeria writes it to the manifest (none for aggregators).
    pub museum: Option<String>,
    #[serde(default)]
    pub match_museum: Vec<String>,
    pub program: String,
    /// License the programme grants when the rule passes (when it is always the same).
    pub grants: Option<String>,
    pub policy_url: String,
    pub data_url: String,
    pub quote: String,
    pub summary: String,
    #[serde(skip_serializing)]
    pub rule: Rule,
    pub evidence: String,
    pub attribution: Attribution,
    pub images: String,
    pub etiquette: Etiquette,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub all: Vec<Condition>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Condition {
    pub field: String,
    pub op: Op,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// The record's field must equal `value` (case-insensitive).
    Equals,
    /// The record's field must not equal `value`; a missing field passes.
    NotEquals,
    /// The field holds a license name, code or URL that must be an accepted license.
    License,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribution {
    pub required: Option<bool>,
    pub requested: Option<String>,
    pub credit: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Etiquette {
    pub search_concurrency: usize,
    pub download_interval_ms: u64,
    #[serde(default)]
    pub note: Option<String>,
}

/// A holding institution that Galeria does not download from directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MuseumNote {
    pub name: String,
    #[serde(rename = "match")]
    pub matches: Vec<String>,
    pub summary: String,
    #[serde(default)]
    pub policy_url: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fallback {
    pub direct: bool,
    pub via: String,
    pub summary: String,
}

/// What a provider read from the museum's record, e.g. `share_license_status → CC0`.
pub type Evidence = BTreeMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub allowed: bool,
    pub license_id: Option<String>,
    /// Display label: "CC0", "CC BY-SA 4.0", "Public domain", or why not.
    pub license: String,
    pub license_url: Option<String>,
    pub attribution_required: bool,
    pub share_alike: bool,
    pub obligations: Vec<String>,
    /// One sentence a person can read.
    pub reason: String,
    /// The raw facts, e.g. `share_license_status = CC0`.
    pub evidence: String,
    pub program: String,
    pub policy_url: String,
}

pub fn source(id: &str) -> Option<&'static SourceRule> {
    RULES.sources.iter().find(|s| s.id == id)
}

fn license_by_id(id: &str) -> Option<&'static License> {
    RULES.licenses.iter().find(|l| l.id == id)
}

fn normalize(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .replace('_', "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// First `N.N` in a license string ("CC BY-SA 4.0" → "4.0").
fn version_of(raw: &str) -> Option<String> {
    let b = raw.as_bytes();
    (0..b.len().saturating_sub(2)).find_map(|i| {
        (b[i].is_ascii_digit()
            && b[i + 1] == b'.'
            && b[i + 2].is_ascii_digit()
            && (i == 0 || !b[i - 1].is_ascii_digit()))
        .then(|| raw[i..i + 3].to_string())
    })
}

/// Map a license name, code or URL (as museums and Commons write them) onto the
/// license table. Returns the license and a display label including its version.
pub fn classify_license(raw: &str) -> Option<(&'static License, String)> {
    let n = normalize(raw);
    if n.is_empty() {
        return None;
    }
    let lic = RULES.licenses.iter().find(|l| {
        l.exact.contains(&n)
            || l.prefixes.iter().any(|p| n.starts_with(p.as_str()))
            || l.contains.iter().any(|c| n.contains(c.as_str()))
    })?;
    let label = match version_of(raw) {
        Some(v) if lic.id.starts_with("cc-by") => format!("{} {v}", lic.label),
        _ => lic.label.clone(),
    };
    Some((lic, label))
}

fn license_url(lic: &License, raw: &str, ev: &Evidence) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("http") && raw.contains("creativecommons.org") {
        return Some(raw.replacen("http://", "https://", 1));
    }
    if let Some(u) = ev.get("license_url").filter(|u| u.starts_with("http")) {
        return Some(u.replacen("http://", "https://", 1));
    }
    match (lic.id.as_str(), version_of(raw)) {
        ("cc-by", Some(v)) => Some(format!("https://creativecommons.org/licenses/by/{v}/")),
        ("cc-by-sa", Some(v)) => Some(format!("https://creativecommons.org/licenses/by-sa/{v}/")),
        _ => lic.url.clone(),
    }
}

/// Decide whether a record may be downloaded, according to its source's rule.
pub fn evaluate(source_id: &str, ev: &Evidence) -> Decision {
    let Some(src) = source(source_id) else {
        return Decision {
            allowed: false,
            license_id: None,
            license: "Unknown source".into(),
            license_url: None,
            attribution_required: false,
            share_alike: false,
            obligations: vec![],
            reason: format!("Galeria has no licensing rule for the source \"{source_id}\", so it will not download from it."),
            evidence: String::new(),
            program: String::new(),
            policy_url: String::new(),
        };
    };
    let deny = |license: &str, reason: String, evidence: String| Decision {
        allowed: false,
        license_id: None,
        license: license.to_string(),
        license_url: None,
        attribution_required: false,
        share_alike: false,
        obligations: vec![],
        reason,
        evidence,
        program: src.program.clone(),
        policy_url: src.policy_url.clone(),
    };

    let mut facts: Vec<String> = Vec::new();
    let mut found: Option<(&'static License, String, String)> = None;
    for c in &src.rule.all {
        let actual = ev.get(&c.field).map(|s| s.trim()).unwrap_or("");
        let shown = if actual.is_empty() {
            "(missing)"
        } else {
            actual
        };
        match c.op {
            Op::Equals => {
                let want = c.value.as_deref().unwrap_or("");
                if !actual.eq_ignore_ascii_case(want) {
                    return deny(
                        "Not open access",
                        format!(
                            "{} does not mark this image as open access: its record has {} = {shown}, and the {} programme requires {want}.",
                            src.name, c.field, src.program
                        ),
                        format!("{} = {shown}", c.field),
                    );
                }
                facts.push(format!("{} = {actual}", c.field));
            }
            Op::NotEquals => {
                let bad = c.value.as_deref().unwrap_or("");
                if !actual.is_empty() && actual.eq_ignore_ascii_case(bad) {
                    return deny(
                        "Not downloadable",
                        format!(
                            "{} does not offer this image for download ({} = {actual}).",
                            src.name, c.field
                        ),
                        format!("{} = {actual}", c.field),
                    );
                }
                if !actual.is_empty() {
                    facts.push(format!("{} = {actual}", c.field));
                }
            }
            Op::License => match classify_license(actual) {
                None => {
                    return deny(
                        if actual.is_empty() {
                            "No license stated"
                        } else {
                            "Unrecognised license"
                        },
                        if actual.is_empty() {
                            format!("{} states no license for this image, so Galeria will not download it.", src.name)
                        } else {
                            format!("Galeria does not recognise the license \"{actual}\" and only downloads images under licenses it knows to be free.")
                        },
                        format!("{} = {shown}", c.field),
                    )
                }
                Some((lic, label)) if !lic.allowed => {
                    return deny(
                        &label,
                        lic.reason.clone().unwrap_or_else(|| {
                            format!("{label} is not a license Galeria accepts.")
                        }),
                        format!("{} = {actual}", c.field),
                    )
                }
                Some((lic, label)) => {
                    facts.push(format!("{} = {actual}", c.field));
                    found = Some((lic, label, actual.to_string()));
                }
            },
        }
    }

    // A programme that always grants the same license overrides whatever was read.
    if let Some(lic) = src.rule.license.as_deref().and_then(license_by_id) {
        found = Some((lic, lic.label.clone(), String::new()));
    }
    let evidence = facts.join("; ");
    let Some((lic, label, raw)) = found else {
        return deny(
            "No license stated",
            format!("{} states no license for this image.", src.name),
            evidence,
        );
    };
    if !lic.allowed {
        return deny(&label, lic.reason.clone().unwrap_or_default(), evidence);
    }
    let attribution_required = lic.attribution
        || src.attribution.required == Some(true)
        || ev
            .get("attribution_required")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut obligations = lic.obligations.clone();
    if obligations.is_empty() {
        if let Some(req) = &src.attribution.requested {
            obligations.push(format!(
                "No obligations. As a courtesy the museum asks for the credit: {req}"
            ));
        }
    }
    Decision {
        allowed: true,
        license_id: Some(lic.id.clone()),
        license_url: license_url(lic, &raw, ev),
        license: label.clone(),
        attribution_required,
        share_alike: lic.share_alike,
        obligations,
        reason: match src.rule.license {
            Some(_) => format!(
                "{} releases this image under {label} through its {} programme.",
                src.name, src.program
            ),
            None => format!(
                "{}: this image is {}.",
                src.name,
                if lic.id == "pdm" {
                    "in the public domain".to_string()
                } else {
                    format!("licensed {label}")
                }
            ),
        },
        evidence,
        program: src.program.clone(),
        policy_url: src.policy_url.clone(),
    }
}

/// How Galeria treats a holding museum named in a manifest.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MuseumPolicy {
    /// Provider id when Galeria can download from this museum directly.
    pub source: Option<String>,
    pub direct: bool,
    pub program: Option<String>,
    pub license: Option<String>,
    pub summary: String,
    pub policy_url: Option<String>,
}

pub fn museum_policy(museum: &str) -> MuseumPolicy {
    let m = museum.to_lowercase();
    if let Some(s) = RULES
        .sources
        .iter()
        .find(|s| s.match_museum.iter().any(|k| m.contains(k.as_str())))
    {
        return MuseumPolicy {
            source: Some(s.id.clone()),
            direct: true,
            program: Some(s.program.clone()),
            license: s
                .grants
                .as_deref()
                .and_then(license_by_id)
                .map(|l| l.label.clone()),
            summary: s.summary.clone(),
            policy_url: Some(s.policy_url.clone()),
        };
    }
    if let Some(n) = RULES
        .museums
        .iter()
        .find(|n| n.matches.iter().any(|k| m.contains(k.as_str())))
    {
        return MuseumPolicy {
            source: None,
            direct: false,
            program: None,
            license: n.license.clone(),
            summary: n.summary.clone(),
            policy_url: n.policy_url.clone(),
        };
    }
    MuseumPolicy {
        source: None,
        direct: RULES.fallback.direct,
        program: None,
        license: None,
        summary: RULES.fallback.summary.clone(),
        policy_url: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pairs: &[(&str, &str)]) -> Evidence {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn rules_file_is_consistent() {
        let r = rules();
        assert!(!r.sources.is_empty());
        for s in &r.sources {
            assert!(s.policy_url.starts_with("https://"), "{}", s.id);
            assert!(!s.summary.is_empty() && !s.quote.is_empty(), "{}", s.id);
            assert!(!s.rule.all.is_empty(), "{} has no conditions", s.id);
            if let Some(g) = &s.grants {
                assert!(
                    license_by_id(g).is_some_and(|l| l.allowed),
                    "{} grants unknown license {g}",
                    s.id
                );
            }
            if let Some(l) = &s.rule.license {
                assert!(
                    license_by_id(l).is_some(),
                    "{} rule names unknown license {l}",
                    s.id
                );
            }
        }
        for l in &r.licenses {
            assert!(
                l.allowed || l.reason.is_some(),
                "refused license {} needs a reason",
                l.id
            );
        }
    }

    #[test]
    fn museum_flags() {
        let d = evaluate("cma", &ev(&[("share_license_status", "CC0")]));
        assert!(
            d.allowed && d.license == "CC0" && !d.attribution_required,
            "{d:?}"
        );
        let d = evaluate("cma", &ev(&[("share_license_status", "Copyrighted")]));
        assert!(!d.allowed && d.reason.contains("Copyrighted"), "{d:?}");
        assert!(
            !evaluate("cma", &ev(&[])).allowed,
            "a missing flag is not permission"
        );

        assert!(evaluate("met", &ev(&[("isPublicDomain", "true")])).allowed);
        assert!(!evaluate("met", &ev(&[("isPublicDomain", "false")])).allowed);
        assert!(evaluate("aic", &ev(&[("is_public_domain", "true")])).allowed);
        assert!(!evaluate("aic", &ev(&[("is_public_domain", "false")])).allowed);
        assert!(evaluate("nga", &ev(&[("openaccess", "1")])).allowed);
        assert!(!evaluate("nga", &ev(&[("openaccess", "0")])).allowed);
        assert!(
            !evaluate("louvre", &ev(&[("anything", "CC0")])).allowed,
            "unknown sources are refused"
        );
    }

    #[test]
    fn rijksmuseum_rights_statements() {
        let pd = evaluate(
            "rijks",
            &ev(&[(
                "rights",
                "http://creativecommons.org/publicdomain/mark/1.0/",
            )]),
        );
        assert!(
            pd.allowed && pd.license == "Public domain" && !pd.attribution_required,
            "{pd:?}"
        );
        let cc0 = evaluate(
            "rijks",
            &ev(&[
                (
                    "rights",
                    "https://creativecommons.org/publicdomain/zero/1.0/",
                ),
                ("downloadable", "true"),
            ]),
        );
        assert!(cc0.allowed && cc0.license == "CC0");
        let by = evaluate(
            "rijks",
            &ev(&[("rights", "http://creativecommons.org/licenses/by/4.0/")]),
        );
        assert!(
            by.allowed && by.attribution_required && by.license == "CC BY 4.0",
            "{by:?}"
        );
        assert_eq!(
            by.license_url.as_deref(),
            Some("https://creativecommons.org/licenses/by/4.0/")
        );
        assert!(
            !evaluate(
                "rijks",
                &ev(&[("rights", "http://rightsstatements.org/vocab/InC/1.0/")])
            )
            .allowed
        );
        assert!(!evaluate("rijks", &ev(&[])).allowed);
        let no_dl = evaluate(
            "rijks",
            &ev(&[
                (
                    "rights",
                    "http://creativecommons.org/publicdomain/mark/1.0/",
                ),
                ("downloadable", "false"),
            ]),
        );
        assert!(!no_dl.allowed, "{no_dl:?}");
    }

    #[test]
    fn commons_licenses() {
        for (raw, ok, label, attribution) in [
            ("pd", true, "Public domain", false),
            ("Public domain", true, "Public domain", false),
            ("PD-Art", true, "Public domain", false),
            ("cc0", true, "CC0", false),
            ("CC BY-SA 4.0", true, "CC BY-SA 4.0", true),
            ("cc-by-sa-3.0", true, "CC BY-SA 3.0", true),
            ("CC BY 2.0", true, "CC BY 2.0", true),
            ("cc-by-3.0", true, "CC BY 3.0", true),
            ("CC BY-NC 2.0", false, "CC BY-NC 2.0", false),
            ("CC BY-NC-SA 4.0", false, "CC BY-NC-SA 4.0", false),
            ("CC BY-ND 4.0", false, "CC BY-ND 4.0", false),
            ("GFDL", false, "GFDL", false),
            ("Copyrighted free use", false, "In copyright", false),
            (
                "Some museum's own terms",
                false,
                "Unrecognised license",
                false,
            ),
            ("", false, "No license stated", false),
        ] {
            let d = evaluate("commons", &ev(&[("license", raw)]));
            assert_eq!(d.allowed, ok, "{raw}: {d:?}");
            assert_eq!(d.license, label, "{raw}");
            assert_eq!(d.attribution_required, attribution, "{raw}");
            assert!(!d.reason.is_empty());
        }
        let d = evaluate("commons", &ev(&[("license", "CC BY-SA 4.0")]));
        assert!(d.share_alike && d.obligations.len() == 3);
        assert_eq!(
            d.license_url.as_deref(),
            Some("https://creativecommons.org/licenses/by-sa/4.0/")
        );
    }

    #[test]
    fn museums_map_to_sources() {
        let p = museum_policy("Cleveland Museum of Art, Cleveland");
        assert_eq!(p.source.as_deref(), Some("cma"));
        assert!(p.direct && p.license.as_deref() == Some("CC0"));
        let p = museum_policy("Rila Monastery, Bulgaria");
        assert!(!p.direct && p.source.is_none());
        assert!(p
            .summary
            .contains("photographs by individual photographers"));
    }

    /// Every museum in the author's manifest has a specific answer, and only
    /// the open-access programmes are ever downloaded from directly.
    #[test]
    fn museums_without_open_access_are_never_direct() {
        for (museum, expect_url) in [
            (
                "Musée du Louvre, Paris",
                Some("https://collections.louvre.fr/en/page/cgu"),
            ),
            (
                "Musée d'Orsay, Paris",
                Some("https://www.musee-orsay.fr/en/legal-information"),
            ),
            (
                "Musée de l'Orangerie, Paris",
                Some("https://www.musee-orsay.fr/en/legal-information"),
            ),
            (
                "Musée Marmottan Monet, Paris",
                Some("https://www.marmottan.fr/en/press-and-professionals/order-your-photographs/"),
            ),
            (
                "Museo del Prado, Madrid",
                Some("https://www.museodelprado.es/en/legal-information"),
            ),
            (
                "Uffizi Gallery, Florence",
                Some("https://www.uffizi.it/en/professional-services/photo-rights-reproductions"),
            ),
            (
                "Kunsthistorisches Museum, Vienna",
                Some("https://www.khm.at/en/museum/rights-reproduction"),
            ),
            (
                "State Tretyakov Gallery, Moscow",
                Some("https://www.tretyakovgallery.ru/about/copirith/"),
            ),
            (
                "State Russian Museum, St Petersburg",
                Some("https://en.rusmuseum.ru/"),
            ),
            (
                "Alte Nationalgalerie, Berlin",
                Some("https://www.smb.museum/en/open-science/use/"),
            ),
            (
                "Byzantine and Christian Museum, Athens",
                Some("https://www.byzantinemuseum.gr/en/terms"),
            ),
            (
                "National Gallery of Greece, Athens",
                Some("https://library.nationalgallery.gr/en/suxnes_erotiseis/index.html"),
            ),
            (
                "Chazen Museum of Art, Madison, Wisconsin",
                Some("https://chazen.wisc.edu/contact-us/request-an-image"),
            ),
            ("Chora Church (Kariye Museum), Istanbul", None),
            ("Stanza della Segnatura, Apostolic Palace, Vatican", None),
        ] {
            let p = museum_policy(museum);
            assert!(
                !p.direct && p.source.is_none(),
                "{museum} must not be a direct source"
            );
            assert_eq!(p.policy_url.as_deref(), expect_url, "{museum}");
            assert_ne!(
                p.summary, RULES.fallback.summary,
                "{museum} should have its own note"
            );
        }
        // A museum Galeria has never heard of gets the conservative default.
        let p = museum_policy("Some Regional Museum");
        assert!(!p.direct && p.summary == RULES.fallback.summary);
        // Names that look alike do not get confused with an open-access source.
        assert!(museum_policy("National Gallery of Greece, Athens")
            .source
            .is_none());
        for (museum, source) in [
            ("National Gallery of Art, Washington", "nga"),
            ("Rijksmuseum, Amsterdam", "rijks"),
            ("The Metropolitan Museum of Art, New York", "met"),
            ("Art Institute of Chicago", "aic"),
        ] {
            assert_eq!(
                museum_policy(museum).source.as_deref(),
                Some(source),
                "{museum}"
            );
        }
    }
}
