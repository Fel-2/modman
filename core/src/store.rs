//! Per-game mod storage: extracted mod trees live under the data dir,
//! never inside the game install. Deployment links from here into the game.

use crate::archive;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Reference back to a mod's Nexus origin, for update checks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NexusRef {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    #[serde(default)]
    pub version: String,
}

/// A mod installed into the store (extracted, not yet necessarily deployed).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModRecord {
    /// Filesystem-safe unique id within the game.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// Origin (archive filename, or "nexus:<id>" once integrated).
    #[serde(default)]
    pub source: Option<String>,
    /// Total size of the installed files, bytes.
    #[serde(default)]
    pub size_bytes: u64,
    /// Nexus origin metadata, when installed from Nexus.
    #[serde(default)]
    pub nexus: Option<NexusRef>,
}

impl ModRecord {
    /// Directory holding this mod's extracted files.
    pub fn dir(&self, game_store: &Path) -> PathBuf {
        game_store.join("mods").join(&self.slug)
    }
}

/// Sum the size of all files under a directory.
pub fn dir_size(dir: &Path) -> u64 {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Turn an arbitrary name into a filesystem-safe slug.
pub fn slugify(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    s.trim_matches('-').to_string()
}

/// Root directory for one game's store: `$DATA/modeman/games/<id>`.
pub fn game_store_dir(data_root: &Path, game_id: &str) -> PathBuf {
    data_root.join("games").join(game_id)
}

/// Pick a filesystem-safe slug not already used by `existing`.
pub fn unique_slug(existing: &[ModRecord], base: &str) -> String {
    let base = if base.is_empty() { "mod" } else { base };
    let mut unique = base.to_string();
    let mut n = 1;
    while existing.iter().any(|m| m.slug == unique) {
        n += 1;
        unique = format!("{base}-{n}");
    }
    unique
}

/// An archive extracted to a temporary staging area, pending finalization.
/// FOMOD installers inspect/transform this before it becomes a real mod.
pub struct Staged {
    pub slug: String,
    pub name: String,
    pub dir: PathBuf,
}

/// Extract an archive into `<store>/.staging/<slug>` without flattening, so a
/// scripted installer can read the raw tree. Use [`finalize_direct`] for plain
/// archives, or run a FOMOD session into the final dir.
///
/// `reuse_slug` reinstalls over an existing mod (in-place update) instead of
/// allocating a fresh slug.
pub fn extract_staging(
    game_store: &Path,
    archive_path: &Path,
    existing: &[ModRecord],
    reuse_slug: Option<&str>,
) -> Result<Staged> {
    if !archive::is_supported(archive_path) {
        return Err(Error::Archive(format!(
            "unsupported archive type: {}",
            archive_path.display()
        )));
    }
    let name = archive_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mod")
        .to_string();
    let slug = match reuse_slug {
        Some(s) => s.to_string(),
        None => unique_slug(existing, &slugify(&name)),
    };

    let dir = game_store.join(".staging").join(&slug);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    }
    archive::extract(archive_path, &dir)?;
    Ok(Staged { slug, name, dir })
}

/// Final mod directory for a slug.
pub fn mod_dir(game_store: &Path, slug: &str) -> PathBuf {
    game_store.join("mods").join(slug)
}

/// Promote a plain (non-FOMOD) staging dir to the final mod dir. When
/// `flatten` is set, a single wrapper folder is hoisted; folder-per-mod games
/// (RimWorld, Stardew) pass `false` to keep the mod's own folder intact.
pub fn finalize_direct(staging: &Path, final_dir: &Path, flatten: bool) -> Result<()> {
    if final_dir.exists() {
        std::fs::remove_dir_all(final_dir).map_err(|e| Error::io(final_dir, e))?;
    }
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::rename(staging, final_dir).map_err(|e| Error::io(staging, e))?;
    if flatten {
        flatten_single_wrapper(final_dir)?;
    }
    Ok(())
}

/// Remove a staging dir (e.g. after a FOMOD install or cancel).
pub fn discard_staging(staging: &Path) {
    let _ = std::fs::remove_dir_all(staging);
}

/// Build the `source` label for a record from the archive filename.
pub fn source_label(archive_path: &Path) -> Option<String> {
    archive_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from)
}

/// Many archives wrap everything in a single top-level folder. If the
/// extracted dir contains exactly one entry and it's a directory, hoist
/// its contents up one level so deploy roots align.
fn flatten_single_wrapper(dir: &Path) -> Result<()> {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| Error::io(dir, e))?
        .flatten()
        .collect();
    if entries.len() != 1 {
        return Ok(());
    }
    let only = entries[0].path();
    if !only.is_dir() {
        return Ok(());
    }
    // Avoid hoisting a real game subdir (e.g. "Data") — keep those.
    let name = only.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if is_known_root(name) {
        return Ok(());
    }
    let tmp = dir.with_extension("hoist-tmp");
    std::fs::rename(&only, &tmp).map_err(|e| Error::io(&only, e))?;
    // `dir` is now empty; replace with the hoisted contents.
    std::fs::remove_dir(dir).map_err(|e| Error::io(dir, e))?;
    std::fs::rename(&tmp, dir).map_err(|e| Error::io(&tmp, e))?;
    Ok(())
}

/// Names that are themselves real in-game folders (so a mod rooted at one of
/// these is already correctly structured and must NOT be hoisted).
fn is_known_root(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        // deploy roots / engine dirs
        "data" | "data files" | "archive" | "r6" | "red4ext" | "bin" | "engine" | "mods"
        // Creation Engine Data subfolders
        | "textures" | "meshes" | "scripts" | "source" | "sound" | "music" | "voices"
        | "interface" | "materials" | "shadersfx" | "seq" | "strings" | "video" | "grass"
        | "lodsettings" | "facegen" | "actors" | "effects" | "programs" | "vis"
        // common script-extender / tool folders
        | "skse" | "skse64" | "f4se" | "nvse" | "fose" | "obse" | "sfse"
        | "mcm" | "dialogueviews" | "calientetools" | "fomod"
        // Cyberpunk subfolders
        | "redscript" | "tweaks" | "plugins" | "scripts_hot"
    )
}

/// Delete a mod's files from the store.
pub fn remove(game_store: &Path, record: &ModRecord) -> Result<()> {
    let dir = record.dir(game_store);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    }
    Ok(())
}
