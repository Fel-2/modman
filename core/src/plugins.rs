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
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("esm") => 0,
        Some("esl") => 1,
        _ => 2,
    }
}

/// Resolve the prefix `plugins.txt`, honoring an existing file's casing.
fn plugins_txt_path(appdata: &Path) -> PathBuf {
    if let Ok(rd) = std::fs::read_dir(appdata) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case("plugins.txt") {
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

    let managed_set: BTreeSet<String> =
        managed.iter().map(|s| s.to_ascii_lowercase()).collect();

    // Keep non-managed lines (base masters, DLC, user entries) untouched.
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for raw in existing.lines() {
        let name = line_name(raw).to_ascii_lowercase();
        if name.is_empty() || raw.trim_start().starts_with('#') {
            out.push(raw.to_string());
        } else if !managed_set.contains(&name) {
            out.push(raw.to_string());
        }
        // managed entries are dropped; re-emitted below in correct order.
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
