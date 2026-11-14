//! Writing to `manifest.json`.
//!
//! The manifest stays the single index of the library folder, and other tools
//! write it too, so Galeria is a careful guest: it only ever *adds* entries
//! (or removes entries whose file is gone, when asked), keeps every existing
//! key, value, order and the file's indentation, re-reads the file immediately
//! before writing, and replaces it atomically. A copy of the previous version
//! is kept in the app's data folder.

use crate::library::MANIFEST;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const ITEM_KEYS: [&str; 4] = ["items", "paintings", "artworks", "works"];
const BACKUPS_KEPT: usize = 8;

fn read(root: &Path) -> Result<(String, Value), String> {
    let path = root.join(MANIFEST);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("Could not read {MANIFEST}: {e}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("{MANIFEST} is not valid JSON: {e}"))?;
    Ok((text, value))
}

fn items_mut(v: &mut Value) -> Result<&mut Vec<Value>, String> {
    match v {
        Value::Array(a) => Ok(a),
        Value::Object(o) => {
            let key = ITEM_KEYS
                .iter()
                .find(|k| o.get(**k).is_some_and(|v| v.is_array()));
            match key {
                Some(k) => Ok(o
                    .get_mut(*k)
                    .and_then(|v| v.as_array_mut())
                    .expect("checked above")),
                None => Err(format!("{MANIFEST} has no \"items\" array")),
            }
        }
        _ => Err(format!("{MANIFEST} must be an array or an object")),
    }
}

/// Indentation of the existing file (so a rewrite produces a minimal diff).
fn indent_of(text: &str) -> String {
    text.lines()
        .nth(1)
        .map(|l| {
            l.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| "  ".to_string())
}

/// Keep summary numbers that other tools wrote in step with the items.
fn recount(v: &mut Value) {
    let Value::Object(top) = v else { return };
    let Some(items) = ITEM_KEYS
        .iter()
        .find_map(|k| top.get(*k).and_then(|v| v.as_array()))
        .cloned()
    else {
        return;
    };
    if top.get("count").is_some_and(|c| c.is_number()) {
        top.insert("count".into(), items.len().into());
    }
    let Some(Value::Object(counts)) = top.get_mut("counts") else {
        return;
    };
    let flagged = |flag: &str| {
        items
            .iter()
            .filter(|i| {
                i.get("tone_flags")
                    .and_then(|f| f.as_array())
                    .is_some_and(|f| f.iter().any(|x| x.as_str() == Some(flag)))
            })
            .count()
    };
    for (key, n) in [
        (
            "in_rotation",
            items
                .iter()
                .filter(|i| i.get("parked").and_then(|p| p.as_bool()) != Some(true))
                .count(),
        ),
        ("portrait", flagged("portrait")),
        ("flagged_dark", flagged("dark")),
        ("flagged_muted", flagged("muted")),
        ("squarish", flagged("squarish")),
    ] {
        if counts.get(key).is_some_and(|c| c.is_number()) {
            counts.insert(key.into(), n.into());
        }
    }
    for (key, field) in [("by_source", "source"), ("by_museum", "museum")] {
        let Some(Value::Object(map)) = counts.get_mut(key) else {
            continue;
        };
        for v in map.values_mut() {
            *v = 0.into();
        }
        for name in items
            .iter()
            .filter_map(|i| i.get(field).and_then(|s| s.as_str()))
        {
            let n = map.get(name).and_then(|n| n.as_u64()).unwrap_or(0);
            map.insert(name.to_string(), (n + 1).into());
        }
        map.retain(|_, n| n.as_u64() != Some(0));
    }
}

fn backup(root: &Path, text: &str, backups: Option<&Path>) {
    let Some(dir) = backups else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let name = format!(
        "manifest-{}-{}.json",
        crate::util::now(),
        crate::util::hash_str(&[&root.to_string_lossy()])
    );
    let _ = std::fs::write(dir.join(name), text);
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .collect();
        files.sort();
        let extra = files.len().saturating_sub(BACKUPS_KEPT);
        for old in files.into_iter().take(extra) {
            let _ = std::fs::remove_file(old);
        }
    }
}

fn write(root: &Path, original: &str, value: &Value, backups: Option<&Path>) -> Result<(), String> {
    let indent = indent_of(original);
    let mut out = Vec::with_capacity(original.len() + 4096);
    let fmt = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut ser = serde_json::Serializer::with_formatter(&mut out, fmt);
    value.serialize(&mut ser).map_err(|e| e.to_string())?;
    if original.ends_with('\n') {
        out.push(b'\n');
    }
    // Never replace the manifest with something that does not parse back.
    serde_json::from_slice::<Value>(&out)
        .map_err(|e| format!("refusing to write an invalid manifest: {e}"))?;
    backup(root, original, backups);
    let path = root.join(MANIFEST);
    let tmp = root.join(format!(".{MANIFEST}.galeria-{}.tmp", fastrand::u32(..)));
    std::fs::write(&tmp, &out).map_err(|e| format!("Could not write {MANIFEST}: {e}"))?;
    if let Ok(f) = std::fs::File::open(&tmp) {
        let _ = f.sync_all();
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not replace {MANIFEST}: {e}")
    })
}

fn key_of(item: &Value, field: &str) -> Option<String> {
    item.get(field).and_then(|v| v.as_str()).map(str::to_string)
}

/// Start an empty manifest in `root`, for a folder that has none yet. An existing
/// file is never touched. Returns whether one was created.
pub fn create(root: &Path) -> Result<bool, String> {
    let path = root.join(MANIFEST);
    if path.exists() {
        return Ok(false);
    }
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(format!("Could not create {MANIFEST}: {e}")),
    };
    use std::io::Write;
    file.write_all(b"{\n  \"items\": []\n}\n")
        .map_err(|e| format!("Could not write {MANIFEST}: {e}"))?;
    Ok(true)
}

/// Add entries. An entry whose `id` or `file` is already listed is skipped.
/// Returns how many were added.
pub fn append(
    root: &Path,
    entries: &[Map<String, Value>],
    backups: Option<&Path>,
) -> Result<usize, String> {
    if entries.is_empty() {
        return Ok(0);
    }
    let (text, mut value) = read(root)?;
    let items = items_mut(&mut value)?;
    let mut added = 0;
    for e in entries {
        let e = Value::Object(e.clone());
        let (id, file) = (key_of(&e, "id"), key_of(&e, "file"));
        if file.is_none() {
            continue;
        }
        let exists = items
            .iter()
            .any(|i| (id.is_some() && key_of(i, "id") == id) || key_of(i, "file") == file);
        if !exists {
            items.push(e);
            added += 1;
        }
    }
    if added > 0 {
        recount(&mut value);
        write(root, &text, &value, backups)?;
    }
    Ok(added)
}

/// Remove entries matching `ids` (by `"id"`, or by their file field when `"id"`
/// was omitted, matching `library::load`). Keeps a backup when anything changes.
pub fn remove_ids(
    root: &Path,
    ids: &HashSet<String>,
    backups: Option<&Path>,
) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let (text, mut value) = read(root)?;
    let items = items_mut(&mut value)?;
    let before = items.len();
    items.retain(|i| {
        let file = ["file", "path", "filename", "image", "src"]
            .iter()
            .find_map(|k| key_of(i, k));
        let id = key_of(i, "id").or(file);
        !id.is_some_and(|id| ids.contains(&id))
    });
    let removed = before - items.len();
    if removed > 0 {
        recount(&mut value);
        write(root, &text, &value, backups)?;
    }
    Ok(removed)
}

/// Remove entries whose file no longer exists (the user curates by deleting files).
pub fn remove_missing(root: &Path, backups: Option<&Path>) -> Result<usize, String> {
    let (text, mut value) = read(root)?;
    let items = items_mut(&mut value)?;
    let before = items.len();
    items.retain(|i| match key_of(i, "file") {
        Some(f) => root.join(&f).exists(),
        None => true,
    });
    let removed = before - items.len();
    if removed > 0 {
        recount(&mut value);
        write(root, &text, &value, backups)?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_manifest_is_empty_takes_entries_and_never_replaces_one() {
        let dir = std::env::temp_dir().join(format!("galeria-manifest-new-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(create(&dir).unwrap());
        let mut dims = crate::library::DimCache::default();
        let lib = crate::library::load(&dir, &mut dims);
        assert!(lib.report.manifest_found && lib.report.error.is_none());
        assert_eq!(lib.report.entries, 0);
        // A download can be added to it.
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        let mut entry = Map::new();
        entry.insert("file".into(), "a.jpg".into());
        assert_eq!(append(&dir, &[entry], None).unwrap(), 1);
        // Asking again leaves what is there alone.
        let before = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
        assert!(!create(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(MANIFEST)).unwrap(), before);
        std::fs::remove_dir_all(&dir).ok();
    }
    use serde_json::json;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("galeria-manifest-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const ORIGINAL: &str = "{\n \"about\": \"Caf\u{e9} scans\",\n \"counts\": {\n  \"in_rotation\": 1,\n  \"by_source\": {\n   \"nga\": 1\n  },\n  \"by_museum\": {\n   \"National Gallery of Art, Washington\": 1\n  },\n  \"flagged_dark\": 0\n },\n \"items\": [\n  {\n   \"file\": \"a.jpg\",\n   \"id\": \"nga:1\",\n   \"aspect\": 1.43,\n   \"brightness\": 102.5,\n   \"tags\": [],\n   \"tone_flags\": [],\n   \"museum\": \"National Gallery of Art, Washington\",\n   \"source\": \"nga\",\n   \"note\": null\n  }\n ],\n \"unlabelled_preexisting\": [],\n \"count\": 1\n}";

    #[test]
    fn append_keeps_everything_else_byte_for_byte() {
        let dir = temp();
        std::fs::write(dir.join(MANIFEST), ORIGINAL).unwrap();
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        let entry = json!({ "file": "b.jpg", "id": "cma:2", "title": "New", "museum": "Cleveland Museum of Art, Cleveland", "source": "cma", "tone_flags": ["dark"] });
        let backups = dir.join("backups");
        let n = append(&dir, &[entry.as_object().unwrap().clone()], Some(&backups)).unwrap();
        assert_eq!(n, 1);
        let text = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
        // The original item block is untouched, indentation and unicode included.
        let item_block = &ORIGINAL[ORIGINAL.find("  {\n   \"file\": \"a.jpg\"").unwrap()
            ..ORIGINAL.find("\n ],").unwrap()];
        assert!(text.contains(item_block), "{text}");
        assert!(text.starts_with("{\n \"about\": \"Caf\u{e9} scans\","));
        assert!(!text.ends_with('\n'));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["counts"]["in_rotation"], 2);
        assert_eq!(v["counts"]["by_source"]["cma"], 1);
        assert_eq!(
            v["counts"]["by_museum"]["Cleveland Museum of Art, Cleveland"],
            1
        );
        assert_eq!(v["counts"]["flagged_dark"], 1);
        assert_eq!(v["items"][1]["title"], "New");
        // Same id again: nothing is written.
        assert_eq!(
            append(&dir, &[entry.as_object().unwrap().clone()], Some(&backups)).unwrap(),
            0
        );
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
        assert_eq!(
            std::fs::read_to_string(
                std::fs::read_dir(&backups)
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path()
            )
            .unwrap(),
            ORIGINAL
        );
        // No temp files are left behind.
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));

        // b.jpg was never created, so cleaning up removes exactly that entry.
        assert_eq!(remove_missing(&dir, None).unwrap(), 1);
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 1);
        assert_eq!(v["count"], 1);
        assert!(v["counts"]["by_source"].get("cma").is_none());

        // Removing by id clears the remaining entry and keeps a backup.
        let ids: HashSet<String> = ["nga:1".to_string()].into_iter().collect();
        assert_eq!(remove_ids(&dir, &ids, Some(&backups)).unwrap(), 1);
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST)).unwrap()).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 0);
        assert_eq!(v["count"], 0);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bare_arrays_and_bad_files() {
        let dir = temp();
        std::fs::write(dir.join(MANIFEST), "[\n  { \"file\": \"a.jpg\" }\n]\n").unwrap();
        let e = json!({ "file": "b.jpg", "id": "x:1" });
        assert_eq!(
            append(&dir, &[e.as_object().unwrap().clone()], None).unwrap(),
            1
        );
        let text = std::fs::read_to_string(dir.join(MANIFEST)).unwrap();
        assert!(
            text.ends_with("]\n") && text.contains("\n  {\n    \"file\": \"b.jpg\""),
            "{text}"
        );
        std::fs::write(dir.join(MANIFEST), "{ not json").unwrap();
        assert!(append(&dir, &[e.as_object().unwrap().clone()], None).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join(MANIFEST)).unwrap(),
            "{ not json",
            "a broken manifest is left alone"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
