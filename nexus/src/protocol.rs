//! `nxm://` protocol handler registration (freedesktop).
//!
//! Clicking "Mod Manager Download" on Nexus fires an `nxm://` URL. The OS
//! routes it to whichever app claims the `x-scheme-handler/nxm` MIME type.
//! We install a `.desktop` entry pointing at the modeman binary with
//! `--nxm %u`, then set it as the default handler.

use crate::error::{Error, Result};
use std::path::Path;
use std::process::Command;

const DESKTOP_ID: &str = "modeman-nxm.desktop";

/// Render the `.desktop` file contents for the given modeman executable.
pub fn desktop_entry(exec_path: &Path) -> String {
    let exec = exec_path.display();
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=modeman (nxm handler)\n\
         Exec={exec} --nxm %u\n\
         NoDisplay=true\n\
         MimeType=x-scheme-handler/nxm;\n\
         Categories=Game;\n"
    )
}

/// Install the handler for the current user and register it as default for
/// `nxm://`. Idempotent. Returns the path of the written `.desktop` file.
pub fn install_protocol_handler(exec_path: &Path) -> Result<std::path::PathBuf> {
    let base = dirs_data_home()?;
    let apps = base.join("applications");
    std::fs::create_dir_all(&apps)?;
    let desktop = apps.join(DESKTOP_ID);
    std::fs::write(&desktop, desktop_entry(exec_path))?;

    // Best-effort registration; not fatal if the helpers are missing.
    let _ = Command::new("xdg-mime")
        .args(["default", DESKTOP_ID, "x-scheme-handler/nxm"])
        .status();
    let _ = Command::new("update-desktop-database").arg(&apps).status();

    Ok(desktop)
}

fn dirs_data_home() -> Result<std::path::PathBuf> {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return Ok(std::path::PathBuf::from(x));
        }
    }
    let home = std::env::var("HOME").map_err(|_| Error::Other("HOME not set".into()))?;
    Ok(std::path::PathBuf::from(home).join(".local/share"))
}
