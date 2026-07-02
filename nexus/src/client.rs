//! Blocking Nexus Mods REST client (`https://api.nexusmods.com/v1`).
//!
//! Blocking on purpose: callers run it on a worker thread and post results
//! back to the UI loop, which keeps the API simple and avoids pulling a full
//! async runtime into the GUI.

use crate::error::{Error, Result};
use crate::nxm::NxmLink;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const API_BASE: &str = "https://api.nexusmods.com/v1";
const V2_GRAPHQL: &str = "https://api.nexusmods.com/v2/graphql";
const USER_AGENT: &str = concat!("modeman/", env!("CARGO_PKG_VERSION"), " (+linux)");

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub user_id: u64,
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
}

/// A curated mod listing supported by the Nexus v1 API.
#[derive(Debug, Clone, Copy)]
pub enum ModList {
    Trending,
    LatestAdded,
    LatestUpdated,
}

impl ModList {
    fn path(self) -> &'static str {
        match self {
            ModList::Trending => "trending",
            ModList::LatestAdded => "latest_added",
            ModList::LatestUpdated => "latest_updated",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModInfo {
    pub mod_id: u64,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub picture_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModFile {
    pub file_id: u64,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category_name: Option<String>,
    /// Server filename of the archive.
    pub file_name: String,
    #[serde(default)]
    pub size_kb: u64,
}

#[derive(Debug, Deserialize)]
struct FilesResponse {
    files: Vec<ModFile>,
}

/// A CDN download option for a file.
#[derive(Debug, Clone, Deserialize)]
pub struct DownloadLink {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

pub struct NexusClient {
    http: reqwest::blocking::Client,
    api_key: String,
}

impl NexusClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(Error::NoApiKey);
        }
        let http = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(NexusClient { http, api_key })
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let resp = self
            .http
            .get(url)
            .header("apikey", &self.api_key)
            .header("Accept", "application/json")
            .send()?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(Error::Api {
                status: status.as_u16(),
                body,
            });
        }
        Ok(resp.json()?)
    }

    /// Validate the API key; returns the account it belongs to.
    pub fn validate(&self) -> Result<User> {
        self.get_json(&format!("{API_BASE}/users/validate.json"))
    }

    pub fn mod_info(&self, domain: &str, mod_id: u64) -> Result<ModInfo> {
        self.get_json(&format!("{API_BASE}/games/{domain}/mods/{mod_id}.json"))
    }

    /// A curated mod list for a game. `kind` ∈ {`trending`, `latest_added`,
    /// `latest_updated`}. (Nexus v1 has no keyword search.)
    pub fn mod_list(&self, domain: &str, kind: ModList) -> Result<Vec<ModInfo>> {
        self.get_json(&format!(
            "{API_BASE}/games/{domain}/mods/{}.json",
            kind.path()
        ))
    }

    /// Keyword search via the v2 GraphQL API (v1 has no search). Sorted by
    /// total downloads. Works without authentication.
    pub fn search(&self, domain: &str, query: &str) -> Result<Vec<ModInfo>> {
        #[derive(Deserialize)]
        struct Uploader {
            #[serde(default)]
            name: String,
        }
        #[derive(Deserialize)]
        struct Node {
            #[serde(rename = "modId")]
            mod_id: u64,
            name: String,
            #[serde(default)]
            summary: String,
            #[serde(default)]
            version: String,
            #[serde(default)]
            author: String,
            #[serde(default)]
            uploader: Option<Uploader>,
        }
        #[derive(Deserialize)]
        struct Mods {
            nodes: Vec<Node>,
        }
        #[derive(Deserialize)]
        struct Data {
            mods: Mods,
        }
        #[derive(Deserialize)]
        struct GqlError {
            message: String,
        }
        #[derive(Deserialize)]
        struct Resp {
            data: Option<Data>,
            #[serde(default)]
            errors: Vec<GqlError>,
        }

        let body = serde_json::json!({
            "query": "query($filter: ModsFilter, $count: Int, $sort: [ModsSort!]) {\
                mods(filter: $filter, count: $count, sort: $sort) {\
                    nodes { modId name summary version author uploader { name } } } }",
            "variables": {
                "count": 50,
                "sort": [{ "downloads": { "direction": "DESC" } }],
                "filter": {
                    "gameDomainName": [{ "value": domain, "op": "EQUALS" }],
                    "name": [{ "value": query, "op": "WILDCARD" }]
                }
            }
        });
        // No auth header: v2 GraphQL accepts anonymous queries, and it
        // rejects v1 personal API keys (it wants OAuth JWTs instead).
        let resp = self.http.post(V2_GRAPHQL).json(&body).send()?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Api {
                status: status.as_u16(),
                body: resp.text().unwrap_or_default(),
            });
        }
        let parsed: Resp = resp.json()?;
        if let Some(e) = parsed.errors.first() {
            return Err(Error::Other(format!("graphql: {}", e.message)));
        }
        Ok(parsed
            .data
            .map(|d| d.mods.nodes)
            .unwrap_or_default()
            .into_iter()
            .map(|n| ModInfo {
                mod_id: n.mod_id,
                name: n.name,
                summary: n.summary,
                version: n.version,
                author: if n.author.is_empty() {
                    n.uploader.map(|u| u.name).unwrap_or_default()
                } else {
                    n.author
                },
                picture_url: None,
            })
            .collect())
    }

    pub fn files(&self, domain: &str, mod_id: u64) -> Result<Vec<ModFile>> {
        let r: FilesResponse = self.get_json(&format!(
            "{API_BASE}/games/{domain}/mods/{mod_id}/files.json"
        ))?;
        Ok(r.files)
    }

    /// Resolve CDN download links for a file.
    ///
    /// Free accounts must pass the `nxm` link (its `key`/`expires` authorize
    /// the download). Premium accounts may pass `None`.
    pub fn download_links(
        &self,
        domain: &str,
        mod_id: u64,
        file_id: u64,
        nxm: Option<&NxmLink>,
    ) -> Result<Vec<DownloadLink>> {
        let mut url =
            format!("{API_BASE}/games/{domain}/mods/{mod_id}/files/{file_id}/download_link.json");
        if let Some(l) = nxm {
            if let (Some(key), Some(exp)) = (&l.key, l.expires) {
                url.push_str(&format!("?key={key}&expires={exp}"));
            }
        }
        self.get_json(&url)
    }

    /// Convenience: resolve links for an `nxm://` link and pick the first CDN.
    pub fn resolve_nxm(&self, link: &NxmLink) -> Result<DownloadLink> {
        let links = self.download_links(&link.domain, link.mod_id, link.file_id, Some(link))?;
        links
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("no download links returned".into()))
    }

    /// Stream a download to `dest`. `progress(downloaded, total_opt)` is
    /// called periodically; `total` is `None` when the server omits length.
    pub fn download_to(
        &self,
        uri: &str,
        dest: &Path,
        mut progress: impl FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let mut resp = self.http.get(uri).send()?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            return Err(Error::Api {
                status: status.as_u16(),
                body,
            });
        }
        let total = resp.content_length();
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(dest)?;
        let mut buf = [0u8; 64 * 1024];
        let mut done: u64 = 0;
        loop {
            let n = std::io::Read::read(&mut resp, &mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        file.flush()?;
        Ok(())
    }
}
