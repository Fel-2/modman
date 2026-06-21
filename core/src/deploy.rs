//! Deployment: materialize the active profile's mod files into the game's
//! deploy root. Strategy is behind a trait so VFS/overlay can be added later.
//!
//! Files that already exist in the game dir and are *not* ours (vanilla loose
//! files, or files dropped by another tool) are backed up to `*.modeman-orig`
//! before we link over them, and restored on clear — so deploying and clearing
//! is always non-destructive to the base game.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const BACKUP_SUFFIX: &str = ".modeman-orig";

/// How files are placed into the game dir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LinkMethod {
    /// Symbolic links — robust, cross-filesystem, cheap. Default.
    #[default]
    Symlink,
    /// Hard links — appear as real files (better compat with some loaders),
    /// but require the mod store and game dir to share a filesystem.
    Hardlink,
}

/// Records what a deploy created so it can be cleanly reverted.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DeployManifest {
    /// Relative paths (under deploy root) of links we created.
    pub links: Vec<String>,
    /// Relative dirs we created (deepest first for removal).
    pub dirs: Vec<String>,
    /// Relative paths whose pre-existing real file we moved to `*.modeman-orig`.
    #[serde(default)]
    pub backups: Vec<String>,
}

/// One mod's source tree plus its load-order rank (higher wins conflicts).
pub struct DeploySource {
    pub root: PathBuf,
    pub rank: usize,
}

pub trait Deployer {
    /// Link all sources into `target_root`. Returns a manifest for revert.
    fn deploy(&self, sources: &[DeploySource], target_root: &Path) -> Result<DeployManifest>;
    /// Revert a previous deploy described by `manifest`.
    fn clear(&self, target_root: &Path, manifest: &DeployManifest) -> Result<()>;
}

/// Links mod files into the game dir using [`LinkMethod`]. Last source in load
/// order wins on path conflict.
pub struct LinkDeployer {
    pub method: LinkMethod,
}

impl LinkDeployer {
    pub fn new(method: LinkMethod) -> Self {
        LinkDeployer { method }
    }
}

impl Deployer for LinkDeployer {
    fn deploy(&self, sources: &[DeploySource], target_root: &Path) -> Result<DeployManifest> {
        let mut manifest = DeployManifest::default();
        let mut created_dirs: Vec<String> = Vec::new();

        // Sort by rank so higher-rank mods overwrite earlier links.
        let mut ordered: Vec<&DeploySource> = sources.iter().collect();
        ordered.sort_by_key(|s| s.rank);

        for src in ordered {
            for entry in WalkDir::new(&src.root).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                if path == src.root {
                    continue;
                }
                let rel = path
                    .strip_prefix(&src.root)
                    .map_err(|e| Error::other(e.to_string()))?;
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let dst = target_root.join(rel);

                if entry.file_type().is_dir() {
                    if !dst.exists() {
                        std::fs::create_dir_all(&dst).map_err(|e| Error::io(&dst, e))?;
                        created_dirs.push(rel_str);
                    }
                    continue;
                }

                if let Some(parent) = dst.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
                    }
                }

                // Clear the destination, preserving any vanilla file we'd shadow.
                if dst.is_symlink() {
                    // Our own link from earlier in this deploy — just replace it.
                    std::fs::remove_file(&dst).map_err(|e| Error::io(&dst, e))?;
                } else if dst.exists() {
                    let bak = backup_path(&dst);
                    if !bak.exists() {
                        std::fs::rename(&dst, &bak).map_err(|e| Error::io(&dst, e))?;
                        manifest.backups.push(rel_str.clone());
                    } else {
                        std::fs::remove_file(&dst).map_err(|e| Error::io(&dst, e))?;
                    }
                }

                make_link(self.method, path, &dst)?;
                if !manifest.links.contains(&rel_str) {
                    manifest.links.push(rel_str);
                }
            }
        }

        created_dirs.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
        manifest.dirs = created_dirs;
        Ok(manifest)
    }

    fn clear(&self, target_root: &Path, manifest: &DeployManifest) -> Result<()> {
        for rel in &manifest.links {
            let p = target_root.join(rel);
            if p.is_symlink() || p.exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
        // Restore backed-up vanilla files now that our links are gone.
        for rel in &manifest.backups {
            let dst = target_root.join(rel);
            let bak = backup_path(&dst);
            if bak.exists() {
                let _ = std::fs::rename(&bak, &dst);
            }
        }
        // Remove created dirs if now empty (deepest first).
        let mut dirs = manifest.dirs.clone();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.matches('/').count()));
        for rel in dirs {
            let p = target_root.join(&rel);
            let _ = std::fs::remove_dir(&p); // ignores non-empty / missing
        }
        Ok(())
    }
}

fn backup_path(dst: &Path) -> PathBuf {
    let mut name: OsString = dst.file_name().unwrap_or_default().to_os_string();
    name.push(BACKUP_SUFFIX);
    dst.with_file_name(name)
}

fn make_link(method: LinkMethod, src: &Path, dst: &Path) -> Result<()> {
    match method {
        LinkMethod::Symlink => symlink(src, dst),
        LinkMethod::Hardlink => std::fs::hard_link(src, dst).map_err(|e| Error::io(dst, e)),
    }
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst).map_err(|e| Error::io(dst, e))
}

#[cfg(not(unix))]
fn symlink(_src: &Path, _dst: &Path) -> Result<()> {
    Err(Error::other("symlink deploy only supported on unix"))
}
