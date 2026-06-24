//! mod.io (api.mod.io/v1) — official free platform. Read/browse/download work
//! with just an API key (free to obtain; no premium tier like Nexus). Games are
//! addressed by their numeric mod.io `game_id`.

use crate::download::stream_to;
use crate::{Error, ListSort, ListedMod, ModPlatform, RemoteFile, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const NAME: &str = "mod.io";
const BASE: &str = "https://api.mod.io/v1";

#[derive(Debug, Deserialize)]
struct ModsResponse {
    data: Vec<ModObj>,
}

#[derive(Debug, Deserialize)]
struct ModObj {
    id: u64,
    name: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    submitted_by: Option<Submitter>,
    #[serde(default)]
    stats: Option<Stats>,
}

#[derive(Debug, Deserialize)]
struct Submitter {
    #[serde(default)]
    username: String,
}

#[derive(Debug, Deserialize)]
struct Stats {
    #[serde(default)]
    downloads_total: u64,
}

#[derive(Debug, Deserialize)]
struct FilesResponse {
    data: Vec<FileObj>,
}

#[derive(Debug, Deserialize)]
struct FileObj {
    id: u64,
    #[serde(default)]
    version: Option<String>,
    filename: String,
    #[serde(default)]
    filesize: u64,
    download: Download,
}

#[derive(Debug, Deserialize)]
struct Download {
    binary_url: String,
}

pub struct Modio {
    http: reqwest::blocking::Client,
    api_key: String,
}

impl Modio {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::Other("mod.io API key required".into()));
        }
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("modeman/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Modio { http, api_key })
    }

    /// Append the api_key as a query param (read auth).
    fn with_key(&self, url: &str) -> String {
        let sep = if url.contains('?') { '&' } else { '?' };
        format!("{url}{sep}api_key={}", self.api_key)
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let resp = self.http.get(self.with_key(url)).send()?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Api {
                platform: NAME,
                status: status.as_u16(),
                body: resp.text().unwrap_or_default(),
            });
        }
        Ok(resp.json()?)
    }
}

impl ModPlatform for Modio {
    fn name(&self) -> &'static str {
        NAME
    }

    fn list(&self, game: &str, sort: ListSort) -> Result<Vec<ListedMod>> {
        if game.is_empty() {
            return Err(Error::GameUnsupported(NAME));
        }
        let sort_q = match sort {
            ListSort::Top => "-popular",
            ListSort::Newest => "-id",
            ListSort::Updated => "-date_updated",
        };
        let url = format!("{BASE}/games/{game}/mods?_limit=100&_sort={sort_q}");
        let resp: ModsResponse = self.get_json(&url)?;
        Ok(resp
            .data
            .into_iter()
            .map(|m| ListedMod {
                id: m.id.to_string(),
                name: m.name,
                author: m.submitted_by.map(|s| s.username).unwrap_or_default(),
                summary: m.summary,
                downloads: m.stats.map(|s| s.downloads_total).unwrap_or(0),
            })
            .collect())
    }

    fn files(&self, game: &str, mod_id: &str) -> Result<Vec<RemoteFile>> {
        let url = format!("{BASE}/games/{game}/mods/{mod_id}/files");
        let resp: FilesResponse = self.get_json(&url)?;
        Ok(resp
            .data
            .into_iter()
            .map(|f| RemoteFile {
                id: f.id.to_string(),
                name: f.filename,
                version: f.version.unwrap_or_default(),
                // binary_url needs the api_key to download.
                url: self.with_key(&f.download.binary_url),
                size: f.filesize,
            })
            .collect())
    }

    fn download(
        &self,
        url: &str,
        dest: &Path,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        stream_to(&self.http, NAME, url, dest, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mods_and_files() {
        let mods = r#"{"data":[
          {"id":7,"name":"Cool Mod","summary":"does cool stuff",
           "submitted_by":{"username":"alice"},"stats":{"downloads_total":4200}}
        ]}"#;
        let r: ModsResponse = serde_json::from_str(mods).unwrap();
        assert_eq!(r.data[0].id, 7);
        assert_eq!(r.data[0].submitted_by.as_ref().unwrap().username, "alice");
        assert_eq!(r.data[0].stats.as_ref().unwrap().downloads_total, 4200);

        let files = r#"{"data":[
          {"id":11,"version":"1.3","filename":"cool.zip","filesize":99,
           "download":{"binary_url":"https://api.mod.io/v1/games/1/mods/7/files/11/download"}}
        ]}"#;
        let f: FilesResponse = serde_json::from_str(files).unwrap();
        assert_eq!(f.data[0].filename, "cool.zip");
        assert!(f.data[0].download.binary_url.contains("/download"));
    }
}
