//! GameBanana (gamebanana.com/apiv11) — free, no auth for browse/download.
//! Large catalog including Cyberpunk 2077 and many Source/fighting games.
//! Games are addressed by their numeric GameBanana game id.
//!
//! Field names follow GameBanana's `_`-prefixed JSON. Shapes are parsed
//! defensively; not yet validated against live responses.

use crate::download::stream_to;
use crate::{Error, ListSort, ListedMod, ModPlatform, RemoteFile, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const NAME: &str = "gamebanana";
const BASE: &str = "https://gamebanana.com/apiv11";

#[derive(Debug, Deserialize)]
struct Subfeed {
    #[serde(rename = "_aRecords", default)]
    records: Vec<Record>,
}

#[derive(Debug, Deserialize)]
struct Record {
    #[serde(rename = "_idRow")]
    id: u64,
    #[serde(rename = "_sName", default)]
    name: String,
    #[serde(rename = "_aSubmitter", default)]
    submitter: Option<Submitter>,
    #[serde(rename = "_sText", default)]
    text: String,
    #[serde(rename = "_nViewCount", default)]
    views: u64,
    #[serde(rename = "_sModelName", default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct Submitter {
    #[serde(rename = "_sName", default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct DownloadPage {
    #[serde(rename = "_aFiles", default)]
    files: Vec<GbFile>,
}

#[derive(Debug, Deserialize)]
struct GbFile {
    #[serde(rename = "_idRow")]
    id: u64,
    #[serde(rename = "_sFile", default)]
    file: String,
    #[serde(rename = "_nFilesize", default)]
    filesize: u64,
    #[serde(rename = "_sDownloadUrl", default)]
    download_url: String,
    #[serde(rename = "_sVersion", default)]
    version: String,
}

pub struct GameBanana {
    http: reqwest::blocking::Client,
}

impl GameBanana {
    pub fn new() -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("modeman/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(GameBanana { http })
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let resp = self.http.get(url).send()?;
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

impl ModPlatform for GameBanana {
    fn name(&self) -> &'static str {
        NAME
    }

    fn list(&self, game: &str, _sort: ListSort) -> Result<Vec<ListedMod>> {
        if game.is_empty() {
            return Err(Error::GameUnsupported(NAME));
        }
        // Subfeed returns the game's recent submissions across models.
        let url = format!("{BASE}/Game/{game}/Subfeed?_nPage=1");
        let feed: Subfeed = self.get_json(&url)?;
        Ok(records_to_listed(feed.records))
    }

    /// Server-side search (live-verified: `Util/Search/Results`).
    fn search(&self, game: &str, query: &str) -> Result<Vec<ListedMod>> {
        if game.is_empty() {
            return Err(Error::GameUnsupported(NAME));
        }
        let url = format!(
            "{BASE}/Util/Search/Results?_sModelName=Mod&_sOrder=best_match&_idGameRow={game}&_sSearchString={}",
            crate::urlencode(query)
        );
        let feed: Subfeed = self.get_json(&url)?;
        Ok(records_to_listed(feed.records))
    }

    fn files(&self, _game: &str, mod_id: &str) -> Result<Vec<RemoteFile>> {
        let url = format!("{BASE}/Mod/{mod_id}/DownloadPage");
        let page: DownloadPage = self.get_json(&url)?;
        Ok(page
            .files
            .into_iter()
            .map(|f| RemoteFile {
                id: f.id.to_string(),
                name: f.file,
                version: f.version,
                url: f.download_url,
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

fn records_to_listed(records: Vec<Record>) -> Vec<ListedMod> {
    records
        .into_iter()
        .filter(|r| r.model.is_empty() || r.model == "Mod")
        .map(|r| ListedMod {
            id: r.id.to_string(),
            name: r.name,
            author: r.submitter.map(|s| s.name).unwrap_or_default(),
            summary: r.text,
            downloads: r.views,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_subfeed_and_downloadpage() {
        let feed = r#"{"_aRecords":[
          {"_idRow":555,"_sName":"Neon Skin","_aSubmitter":{"_sName":"vee"},
           "_sText":"glow","_nViewCount":9000,"_sModelName":"Mod"}
        ]}"#;
        let s: Subfeed = serde_json::from_str(feed).unwrap();
        let listed: Vec<ListedMod> = s
            .records
            .into_iter()
            .map(|r| ListedMod {
                id: r.id.to_string(),
                name: r.name,
                author: r.submitter.map(|x| x.name).unwrap_or_default(),
                summary: r.text,
                downloads: r.views,
            })
            .collect();
        assert_eq!(listed[0].id, "555");
        assert_eq!(listed[0].author, "vee");
        assert_eq!(listed[0].downloads, 9000);

        let dp = r#"{"_aFiles":[
          {"_idRow":987,"_sFile":"skin.zip","_nFilesize":12345,
           "_sDownloadUrl":"https://gamebanana.com/dl/987","_sVersion":"1.0"}
        ]}"#;
        let d: DownloadPage = serde_json::from_str(dp).unwrap();
        assert_eq!(d.files[0].file, "skin.zip");
        assert_eq!(d.files[0].download_url, "https://gamebanana.com/dl/987");
    }
}
