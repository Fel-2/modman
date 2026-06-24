//! Thunderstore (thunderstore.io) — free, no auth. Packages are plain zips,
//! addressed per community (game slug, e.g. "lethal-company", "valheim").

use crate::download::stream_to;
use crate::{Error, ListSort, ListedMod, ModPlatform, RemoteFile, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const NAME: &str = "thunderstore";
const BASE: &str = "https://thunderstore.io";

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    owner: String,
    #[serde(default)]
    date_created: String,
    #[serde(default)]
    date_updated: String,
    #[serde(default)]
    versions: Vec<Version>,
}

#[derive(Debug, Deserialize)]
struct Version {
    #[serde(default)]
    description: String,
    #[serde(default)]
    downloads: u64,
}

#[derive(Debug, Deserialize)]
struct PackageDetail {
    #[serde(default)]
    versions: Vec<DetailVersion>,
}

#[derive(Debug, Deserialize)]
struct DetailVersion {
    full_name: String,
    version_number: String,
    download_url: String,
    #[serde(default)]
    file_size: u64,
}

pub struct Thunderstore {
    http: reqwest::blocking::Client,
}

impl Thunderstore {
    pub fn new() -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("modeman/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Thunderstore { http })
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

impl ModPlatform for Thunderstore {
    fn name(&self) -> &'static str {
        NAME
    }

    fn list(&self, game: &str, sort: ListSort) -> Result<Vec<ListedMod>> {
        if game.is_empty() {
            return Err(Error::GameUnsupported(NAME));
        }
        let url = format!("{BASE}/c/{game}/api/v1/package/");
        let mut packages: Vec<Package> = self.get_json(&url)?;
        sort_packages(&mut packages, sort);
        Ok(packages.iter().take(100).map(to_listed).collect())
    }

    fn files(&self, _game: &str, mod_id: &str) -> Result<Vec<RemoteFile>> {
        // id is "namespace/name".
        let (ns, name) = mod_id
            .split_once('/')
            .ok_or_else(|| Error::Other(format!("bad thunderstore id: {mod_id}")))?;
        let url = format!("{BASE}/api/experimental/package/{ns}/{name}/");
        let detail: PackageDetail = self.get_json(&url)?;
        Ok(detail
            .versions
            .into_iter()
            .map(|v| RemoteFile {
                id: v.version_number.clone(),
                name: v.full_name,
                version: v.version_number,
                url: v.download_url,
                size: v.file_size,
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

fn to_listed(p: &Package) -> ListedMod {
    let latest = p.versions.first();
    ListedMod {
        id: format!("{}/{}", p.owner, p.name),
        name: p.name.clone(),
        author: p.owner.clone(),
        summary: latest.map(|v| v.description.clone()).unwrap_or_default(),
        downloads: p.versions.iter().map(|v| v.downloads).sum(),
    }
}

fn sort_packages(packages: &mut [Package], sort: ListSort) {
    match sort {
        ListSort::Top => packages.sort_by(|a, b| {
            let da: u64 = a.versions.iter().map(|v| v.downloads).sum();
            let db: u64 = b.versions.iter().map(|v| v.downloads).sum();
            db.cmp(&da)
        }),
        ListSort::Newest => packages.sort_by(|a, b| b.date_created.cmp(&a.date_created)),
        ListSort::Updated => packages.sort_by(|a, b| b.date_updated.cmp(&a.date_updated)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"name":"Alpha","owner":"Acme","date_created":"2024-01-01","date_updated":"2024-02-01",
       "versions":[{"description":"alpha mod","version_number":"1.0.0",
                    "download_url":"https://t.io/d/Acme/Alpha/1.0.0/","downloads":100,"file_size":2048}]},
      {"name":"Beta","owner":"Bob","date_created":"2024-03-01","date_updated":"2024-03-05",
       "versions":[{"description":"beta mod","version_number":"0.2.0",
                    "download_url":"https://t.io/d/Bob/Beta/0.2.0/","downloads":500,"file_size":4096}]}
    ]"#;

    #[test]
    fn parses_and_sorts() {
        let mut pkgs: Vec<Package> = serde_json::from_str(SAMPLE).unwrap();
        sort_packages(&mut pkgs, ListSort::Top);
        let listed: Vec<ListedMod> = pkgs.iter().map(to_listed).collect();
        // Beta (500 dl) sorts above Alpha (100).
        assert_eq!(listed[0].id, "Bob/Beta");
        assert_eq!(listed[0].downloads, 500);
        assert_eq!(listed[1].id, "Acme/Alpha");
        assert_eq!(listed[0].summary, "beta mod");
    }
}
