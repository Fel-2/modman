//! Parser for `nxm://` protocol links emitted by the Nexus "Mod Manager
//! Download" button.
//!
//! Shape:
//! `nxm://<domain>/mods/<mod_id>/files/<file_id>?key=<k>&expires=<ts>&user_id=<id>`
//! The query part is present for free-account downloads and absent for
//! premium direct links.

use crate::error::{Error, Result};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmLink {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// One-time download key (free accounts).
    pub key: Option<String>,
    /// Unix expiry of the key.
    pub expires: Option<u64>,
    pub user_id: Option<u64>,
}

impl NxmLink {
    pub fn parse(s: &str) -> Result<Self> {
        let url = Url::parse(s.trim()).map_err(|e| Error::BadLink(e.to_string()))?;
        if url.scheme() != "nxm" {
            return Err(Error::BadLink(format!("scheme is '{}', not 'nxm'", url.scheme())));
        }
        let domain = url
            .host_str()
            .ok_or_else(|| Error::BadLink("missing game domain".into()))?
            .to_string();

        let segs: Vec<&str> = url
            .path_segments()
            .map(|s| s.collect())
            .unwrap_or_default();
        // Expect: ["mods", "<id>", "files", "<id>"]
        let mod_id = match segs.as_slice() {
            ["mods", m, "files", _] => parse_id(m, "mod_id")?,
            _ => return Err(Error::BadLink(format!("unexpected path: /{}", segs.join("/")))),
        };
        let file_id = parse_id(segs[3], "file_id")?;

        let mut key = None;
        let mut expires = None;
        let mut user_id = None;
        for (k, v) in url.query_pairs() {
            match k.as_ref() {
                "key" => key = Some(v.into_owned()),
                "expires" => expires = v.parse().ok(),
                "user_id" => user_id = v.parse().ok(),
                _ => {}
            }
        }

        Ok(NxmLink { domain, mod_id, file_id, key, expires, user_id })
    }
}

fn parse_id(s: &str, what: &str) -> Result<u64> {
    s.parse()
        .map_err(|_| Error::BadLink(format!("bad {what}: '{s}'")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_free_link() {
        let l = NxmLink::parse(
            "nxm://skyrimspecialedition/mods/12345/files/67890?key=AbC123&expires=1700000000&user_id=999",
        )
        .unwrap();
        assert_eq!(l.domain, "skyrimspecialedition");
        assert_eq!(l.mod_id, 12345);
        assert_eq!(l.file_id, 67890);
        assert_eq!(l.key.as_deref(), Some("AbC123"));
        assert_eq!(l.expires, Some(1700000000));
        assert_eq!(l.user_id, Some(999));
    }

    #[test]
    fn parses_premium_link() {
        let l = NxmLink::parse("nxm://cyberpunk2077/mods/42/files/100").unwrap();
        assert_eq!(l.domain, "cyberpunk2077");
        assert_eq!(l.mod_id, 42);
        assert_eq!(l.file_id, 100);
        assert!(l.key.is_none());
    }

    #[test]
    fn rejects_non_nxm() {
        assert!(NxmLink::parse("https://example.com/mods/1/files/2").is_err());
        assert!(NxmLink::parse("nxm://game/mods/abc/files/2").is_err());
    }
}
