//! Conflict detection: which enabled mods provide the same file, and which
//! one wins. Winner = last provider in load order (matches the symlink
//! deployer's "higher rank overwrites" rule).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileConflict {
    /// Path relative to the deploy root, forward-slashed.
    pub rel_path: String,
    /// Providing mod slugs, in load order (earliest first).
    pub providers: Vec<String>,
    /// Slug that wins (last in load order).
    pub winner: String,
}

/// Detect file conflicts across the given mods.
///
/// `sources` must be ordered by load order (earliest/lowest priority first);
/// the same order the deployer applies, so the last provider wins.
pub fn detect(sources: &[(String, PathBuf)]) -> Vec<FileConflict> {
    // rel_path -> providers in encounter (load) order.
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (slug, dir) in sources {
        for entry in WalkDir::new(dir).into_iter().flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(dir) else {
                continue;
            };
            let key = rel.to_string_lossy().replace('\\', "/");
            let providers = map.entry(key).or_default();
            if !providers.contains(slug) {
                providers.push(slug.clone());
            }
        }
    }

    map.into_iter()
        .filter(|(_, providers)| providers.len() > 1)
        .map(|(rel_path, providers)| {
            let winner = providers.last().cloned().unwrap_or_default();
            FileConflict {
                rel_path,
                providers,
                winner,
            }
        })
        .collect()
}

/// Helper: build ordered `(slug, dir)` sources from slugs + a directory map.
pub fn sources_for<'a>(
    ordered_slugs: impl IntoIterator<Item = &'a str>,
    dir_of: impl Fn(&str) -> Option<PathBuf>,
) -> Vec<(String, PathBuf)> {
    ordered_slugs
        .into_iter()
        .filter_map(|s| dir_of(s).map(|d| (s.to_string(), d)))
        .filter(|(_, d): &(String, PathBuf)| Path::new(d).is_dir())
        .collect()
}
