//! Creation Engine plugin activation.
//!
//! Symlinking `.esp/.esm/.esl` into `Data/` is not enough — Bethesda games
//! only load a plugin if it is listed (and `*`-prefixed = active) in
//! `plugins.txt`, which lives inside the game's Proton prefix. This module
//! rewrites that file to match the deployed, enabled mods while leaving the
//! user's base/DLC masters and any non-managed entries untouched.

use crate::error::{Error, Result};
use crate::game::InstalledGame;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PLUGIN_EXTS: [&str; 3] = ["esp", "esm", "esl"];

/// Is this a Creation Engine plugin file?
pub fn is_plugin(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some(e) if PLUGIN_EXTS.contains(&e)
    )
}

/// Top-level plugin filenames inside a mod's (Data-rooted) tree.
pub fn plugins_in(mod_dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(mod_dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file() && is_plugin(&e.path()))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    out.sort_by_key(|n| master_rank(n));
    out
}

/// Load masters (`.esm`) before light masters (`.esl`) before normal (`.esp`).
fn master_rank(name: &str) -> u8 {
    match name
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("esm") => 0,
        Some("esl") => 1,
        _ => 2,
    }
}

/// Master (dependency) filenames declared in a plugin's file header, in
/// declaration order. Returns empty on unreadable/unrecognized files.
///
/// Supports the modern Creation Engine `TES4` header (Skyrim-era 24-byte and
/// Oblivion-era 20-byte record headers) and Morrowind's `TES3` header.
pub fn masters_of(path: &Path) -> Vec<String> {
    masters_of_impl(path).unwrap_or_default()
}

fn masters_of_impl(path: &Path) -> Option<Vec<String>> {
    use std::io::Read;
    // Header data is small; anything bigger is malformed — don't slurp it.
    const MAX_HEADER: usize = 1 << 20;

    let mut f = std::fs::File::open(path).ok()?;
    let mut hdr = [0u8; 24];
    f.read_exact(&mut hdr).ok()?;
    let size = u32::from_le_bytes(hdr[4..8].try_into().ok()?) as usize;
    if size > MAX_HEADER {
        return None;
    }

    match &hdr[0..4] {
        b"TES3" => {
            // Record header is 16 bytes; we already hold 8 bytes of fields.
            let mut body = hdr[16..24].to_vec();
            f.take(size.saturating_sub(8) as u64)
                .read_to_end(&mut body)
                .ok()?;
            Some(parse_fields_tes3(&body))
        }
        b"TES4" => {
            // Skyrim-era record headers are 24 bytes, Oblivion-era 20. We've
            // read 24; keep the ambiguous 4 bytes and try both alignments.
            let mut body = hdr[20..24].to_vec();
            f.take(size as u64).read_to_end(&mut body).ok()?;
            // Fields start at 24 (body[4..]) on modern games, 20 (body[0..])
            // on Oblivion. Pick the alignment whose first field type is sane.
            if looks_like_field(&body[4..]) {
                Some(parse_fields_tes4(&body[4..]))
            } else if looks_like_field(&body) {
                Some(parse_fields_tes4(&body))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Does the buffer start with a plausible subrecord type (4 ASCII uppercase)?
fn looks_like_field(buf: &[u8]) -> bool {
    buf.len() >= 4
        && buf[..4]
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Walk TES4 subrecords (`type[4] size:u16 data`) collecting MAST strings.
fn parse_fields_tes4(mut buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut oversize: Option<usize> = None;
    while buf.len() >= 6 {
        let ty = &buf[0..4];
        let declared = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        let size = oversize.take().unwrap_or(declared);
        let data = &buf[6..];
        if data.len() < size {
            break;
        }
        if ty == b"XXXX" && size == 4 {
            // Large-field marker: the *next* field's real size.
            oversize = Some(u32::from_le_bytes(data[..4].try_into().unwrap()) as usize);
        } else if ty == b"MAST" {
            if let Some(name) = zstring(&data[..size]) {
                out.push(name);
            }
        }
        buf = &data[size..];
    }
    out
}

/// Walk TES3 subrecords (`type[4] size:u32 data`) collecting MAST strings.
fn parse_fields_tes3(mut buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    while buf.len() >= 8 {
        let ty = &buf[0..4];
        let size = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
        let data = &buf[8..];
        if data.len() < size {
            break;
        }
        if ty == b"MAST" {
            if let Some(name) = zstring(&data[..size]) {
                out.push(name);
            }
        }
        buf = &data[size..];
    }
    out
}

/// Null-terminated string field → trimmed String.
fn zstring(data: &[u8]) -> Option<String> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let s = String::from_utf8_lossy(&data[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Stable topological sort: every plugin loads after its declared masters
/// (when both are in the list). Relative order is otherwise preserved.
/// `masters` maps lowercase plugin filename → lowercase master filenames.
/// Cycles (malformed data) are broken at the earliest remaining entry.
pub fn sort_by_masters(
    order: &[String],
    masters: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<String> {
    let lower: Vec<String> = order.iter().map(|s| s.to_ascii_lowercase()).collect();
    let in_set: BTreeSet<&str> = lower.iter().map(|s| s.as_str()).collect();
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    let mut remaining: Vec<usize> = (0..order.len()).collect();
    let mut out = Vec::with_capacity(order.len());
    while !remaining.is_empty() {
        let pick = remaining
            .iter()
            .position(|&i| {
                masters.get(&lower[i]).is_none_or(|ms| {
                    ms.iter()
                        .all(|m| !in_set.contains(m.as_str()) || emitted.contains(m))
                })
            })
            .unwrap_or(0);
        let i = remaining.remove(pick);
        emitted.insert(lower[i].clone());
        out.push(order[i].clone());
    }
    out
}

/// Resolve the prefix `plugins.txt`, honoring an existing file's casing.
fn plugins_txt_path(appdata: &Path) -> PathBuf {
    if let Ok(rd) = std::fs::read_dir(appdata) {
        for e in rd.flatten() {
            if e.file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("plugins.txt")
            {
                return e.path();
            }
        }
    }
    appdata.join("plugins.txt")
}

/// Strip the `*` active marker and whitespace from a plugins.txt line.
fn line_name(line: &str) -> &str {
    line.trim().trim_start_matches('*').trim()
}

/// Rewrite `plugins.txt` so the managed set reflects the deployed mods.
///
/// * `active`  — plugin filenames to enable, in final order.
/// * `managed` — every plugin filename modeman controls (so stale entries
///   from a previous deploy are pruned even when now disabled).
///
/// Lines for plugins outside `managed` (base game, DLC, hand-added) are kept
/// verbatim and in place. Returns the file path written, or `None` if the
/// game has no plugin file (e.g. prefix not yet created and creation failed).
pub fn write_plugins_txt(
    game: &InstalledGame,
    active: &[String],
    managed: &[String],
) -> Result<Option<PathBuf>> {
    let Some(appdata) = game.prefix_appdata() else {
        return Ok(None);
    };
    std::fs::create_dir_all(&appdata).map_err(|e| Error::io(&appdata, e))?;
    let path = plugins_txt_path(&appdata);

    // One-time backup of the user's original.
    let backup = path.with_extension("txt.modeman-bak");
    if path.exists() && !backup.exists() {
        std::fs::copy(&path, &backup).map_err(|e| Error::io(&backup, e))?;
    }

    let managed_set: BTreeSet<String> = managed.iter().map(|s| s.to_ascii_lowercase()).collect();

    // Keep non-managed lines (base masters, DLC, user entries) untouched.
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for raw in existing.lines() {
        let name = line_name(raw).to_ascii_lowercase();
        let comment = name.is_empty() || raw.trim_start().starts_with('#');
        // Keep comments/blanks and non-managed entries; managed ones are
        // dropped here and re-emitted below in the correct order.
        if comment || !managed_set.contains(&name) {
            out.push(raw.to_string());
        }
    }

    // Append our active plugins, marked active.
    for name in active {
        out.push(format!("*{name}"));
    }

    let mut body = out.join("\n");
    body.push('\n');
    std::fs::write(&path, body).map_err(|e| Error::io(&path, e))?;
    Ok(Some(path))
}

/// Remove all managed plugins from `plugins.txt`, leaving base entries.
pub fn clear_plugins_txt(game: &InstalledGame, managed: &[String]) -> Result<()> {
    // Equivalent to writing with no active plugins.
    write_plugins_txt(game, &[], managed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn field16(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = ty.to_vec();
        v.extend_from_slice(&(data.len() as u16).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    fn field32(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = ty.to_vec();
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    /// Minimal modern (Skyrim-era, 24-byte header) TES4 plugin.
    fn tes4_modern(masters: &[&str]) -> Vec<u8> {
        let mut fields = field16(b"HEDR", &[0u8; 12]);
        for m in masters {
            let mut z = m.as_bytes().to_vec();
            z.push(0);
            fields.extend(field16(b"MAST", &z));
            fields.extend(field16(b"DATA", &[0u8; 8]));
        }
        let mut v = b"TES4".to_vec();
        v.extend_from_slice(&(fields.len() as u32).to_le_bytes()); // dataSize
        v.extend_from_slice(&[0u8; 12]); // flags, formid, vc
        v.extend_from_slice(&44u16.to_le_bytes()); // version
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend(fields);
        v
    }

    /// Oblivion-era TES4 plugin: 20-byte record header.
    fn tes4_oblivion(masters: &[&str]) -> Vec<u8> {
        let mut fields = field16(b"HEDR", &[0u8; 12]);
        for m in masters {
            let mut z = m.as_bytes().to_vec();
            z.push(0);
            fields.extend(field16(b"MAST", &z));
            fields.extend(field16(b"DATA", &[0u8; 8]));
        }
        let mut v = b"TES4".to_vec();
        v.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        v.extend_from_slice(&[0u8; 12]); // flags, formid, vc — header ends at 20
        v.extend(fields);
        v
    }

    /// Morrowind TES3 plugin: 16-byte record header, u32 field sizes.
    fn tes3(masters: &[&str]) -> Vec<u8> {
        let mut fields = field32(b"HEDR", &[0u8; 300]);
        for m in masters {
            let mut z = m.as_bytes().to_vec();
            z.push(0);
            fields.extend(field32(b"MAST", &z));
            fields.extend(field32(b"DATA", &[0u8; 8]));
        }
        let mut v = b"TES3".to_vec();
        v.extend_from_slice(&(fields.len() as u32).to_le_bytes());
        v.extend_from_slice(&[0u8; 8]); // unknown, flags
        v.extend(fields);
        v
    }

    fn write_tmp(name: &str, bytes: &[u8]) -> PathBuf {
        let p = std::env::temp_dir().join(format!("modeman-plug-{}-{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn parses_masters_modern() {
        let p = write_tmp("modern.esp", &tes4_modern(&["Skyrim.esm", "Update.esm"]));
        assert_eq!(masters_of(&p), vec!["Skyrim.esm", "Update.esm"]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parses_masters_oblivion_header() {
        let p = write_tmp("obl.esp", &tes4_oblivion(&["Oblivion.esm"]));
        assert_eq!(masters_of(&p), vec!["Oblivion.esm"]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parses_masters_tes3() {
        let p = write_tmp("mw.esp", &tes3(&["Morrowind.esm", "Tribunal.esm"]));
        assert_eq!(masters_of(&p), vec!["Morrowind.esm", "Tribunal.esm"]);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn garbage_yields_no_masters() {
        let p = write_tmp("junk.esp", b"not a plugin at all");
        assert!(masters_of(&p).is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn sorts_dependents_after_masters() {
        // c.esp depends on b.esp which depends on a.esm; user order is wrong.
        let order: Vec<String> = ["c.esp", "b.esp", "a.esm", "x.esp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut masters = HashMap::new();
        masters.insert("c.esp".into(), vec!["b.esp".into()]);
        masters.insert("b.esp".into(), vec!["a.esm".into()]);
        // x.esp depends on something outside the managed set — ignored.
        masters.insert("x.esp".into(), vec!["skyrim.esm".into()]);
        let sorted = sort_by_masters(&order, &masters);
        // a.esm unblocks b.esp which unblocks c.esp; x.esp's master is not
        // managed by us, so it never blocks.
        assert_eq!(sorted, vec!["a.esm", "b.esp", "c.esp", "x.esp"]);
    }

    #[test]
    fn sort_is_stable_without_deps() {
        let order: Vec<String> = ["z.esp", "m.esp", "a.esp"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let sorted = sort_by_masters(&order, &HashMap::new());
        assert_eq!(sorted, order, "no deps → order untouched");
    }

    #[test]
    fn sort_survives_cycles() {
        let order: Vec<String> = ["a.esp", "b.esp"].iter().map(|s| s.to_string()).collect();
        let mut masters = HashMap::new();
        masters.insert("a.esp".into(), vec!["b.esp".into()]);
        masters.insert("b.esp".into(), vec!["a.esp".into()]);
        let sorted = sort_by_masters(&order, &masters);
        assert_eq!(sorted.len(), 2, "cycle broken, nothing dropped");
    }
}
