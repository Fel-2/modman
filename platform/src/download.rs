//! Shared streaming download used by all platforms.

use crate::{Error, Result};
use std::io::Write;
use std::path::Path;

pub(crate) fn stream_to(
    http: &reqwest::blocking::Client,
    platform: &'static str,
    url: &str,
    dest: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>),
) -> Result<()> {
    let mut resp = http.get(url).send()?;
    let status = resp.status();
    if !status.is_success() {
        return Err(Error::Api {
            platform,
            status: status.as_u16(),
            body: resp.text().unwrap_or_default(),
        });
    }
    let total = resp.content_length();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(dest)?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
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
