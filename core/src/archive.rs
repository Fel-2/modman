//! Archive extraction via libarchive (zip, 7z, rar-read, tar, ...).

use crate::error::{Error, Result};
use compress_tools::{uncompress_archive, Ownership};
use std::fs::File;
use std::path::Path;

/// Extract `archive` fully into `dest` (created if missing).
pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    let file = File::open(archive).map_err(|e| Error::io(archive, e))?;
    uncompress_archive(file, dest, Ownership::Ignore)
        .map_err(|e| Error::Archive(format!("{}: {e}", archive.display())))?;
    Ok(())
}

/// True if the extension looks like a supported archive.
pub fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst")
    )
}
