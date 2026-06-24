//! Free mod-platform integrations behind one trait, so the GUI can browse and
//! download from any of them uniformly. Unlike Nexus (whose in-app download
//! API is premium-only), these allow full free downloads.
//!
//! Each platform addresses games by its own slug/id, supplied per call by the
//! caller (mapped from the game catalog).

mod download;
pub mod gamebanana;
pub mod modio;
pub mod thunderstore;

use std::path::Path;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{platform} api returned {status}: {body}")]
    Api {
        platform: &'static str,
        status: u16,
        body: String,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("this game is not available on {0}")]
    GameUnsupported(&'static str),
    #[error("{0}")]
    Other(String),
}

/// How to sort/scope a browse listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSort {
    Top,
    Newest,
    Updated,
}

/// A mod as shown in a browse list.
#[derive(Debug, Clone)]
pub struct ListedMod {
    /// Platform-specific id (passed back to `files`).
    pub id: String,
    pub name: String,
    pub author: String,
    pub summary: String,
    pub downloads: u64,
}

/// A downloadable file/version of a mod.
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub size: u64,
}

/// A browsable, downloadable mod source.
pub trait ModPlatform {
    /// Stable platform name, e.g. "thunderstore".
    fn name(&self) -> &'static str;
    /// Browse mods for a platform-specific `game` slug/id.
    fn list(&self, game: &str, sort: ListSort) -> Result<Vec<ListedMod>>;
    /// Downloadable files for a mod.
    fn files(&self, game: &str, mod_id: &str) -> Result<Vec<RemoteFile>>;
    /// Stream a file to `dest`.
    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()>;
}
