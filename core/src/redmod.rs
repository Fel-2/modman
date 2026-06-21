//! Cyberpunk 2077 REDmod deployment (experimental).
//!
//! Two Cyberpunk mod styles exist:
//! * **Legacy** — loose `archive/pc/mod/*.archive`, `r6/`, `red4ext/` files.
//!   These deploy fine with the normal symlink/hardlink deployer; nothing here
//!   is needed.
//! * **REDmod** — folders under `mods/<name>/` with an `info.json`. The game
//!   only loads these after `tools/redmod/bin/redMod.exe deploy` compiles them
//!   into `archive/pc/mod/`. That tool is a Windows exe, so it must run through
//!   Proton — which can only be done on the real machine, not in CI.
//!
//! This module locates the tool + a Proton runtime and builds the command;
//! actually running it is best-effort and gated behind an explicit call.

use crate::error::{Error, Result};
use crate::game::InstalledGame;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Outcome of a REDmod deploy attempt.
#[derive(Debug)]
pub enum RedmodStatus {
    /// No REDmod-style mods present; nothing to do.
    NoRedmodMods,
    /// `redMod.exe` not found under the install.
    ToolMissing,
    /// No Proton runtime located to run the tool.
    ProtonMissing,
    /// The tool ran; `success` reflects its exit status.
    Ran { success: bool, log: String },
}

/// Does this mod dir contain a REDmod module (`mods/<x>/info.json` or a
/// top-level `info.json`)?
pub fn is_redmod(mod_dir: &Path) -> bool {
    WalkDir::new(mod_dir)
        .max_depth(3)
        .into_iter()
        .flatten()
        .any(|e| {
            e.file_type().is_file()
                && e.file_name().to_string_lossy().eq_ignore_ascii_case("info.json")
        })
}

/// Path to the bundled `redMod.exe`, if present.
pub fn redmod_exe(game: &InstalledGame) -> Option<PathBuf> {
    let p = game.path.join("tools/redmod/bin/redMod.exe");
    p.is_file().then_some(p)
}

/// Find a Proton runtime's `proton` launcher in any Steam library.
pub fn find_proton(game: &InstalledGame) -> Option<PathBuf> {
    let common = game.steamapps_dir()?.join("common");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&common)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("Proton"))
                    .unwrap_or(false)
                && p.join("proton").is_file()
        })
        .collect();
    // Prefer "Experimental", then the lexicographically-latest version.
    candidates.sort();
    candidates
        .iter()
        .find(|p| p.to_string_lossy().contains("Experimental"))
        .cloned()
        .or_else(|| candidates.last().cloned())
        .map(|p| p.join("proton"))
}

/// The `proton` argv for `redMod deploy` (program excluded). Pure/testable.
pub fn deploy_args(redmod_exe: &Path, game_root: &Path) -> Vec<String> {
    vec![
        "run".to_string(),
        redmod_exe.display().to_string(),
        "deploy".to_string(),
        format!("-root={}", game_root.display()),
    ]
}

/// Attempt a REDmod deploy. Returns a status describing what happened; never
/// fails the surrounding deploy. **Untested on real hardware — experimental.**
pub fn run_deploy(game: &InstalledGame, mod_dirs: &[PathBuf]) -> Result<RedmodStatus> {
    if !mod_dirs.iter().any(|d| is_redmod(d)) {
        return Ok(RedmodStatus::NoRedmodMods);
    }
    let Some(exe) = redmod_exe(game) else {
        return Ok(RedmodStatus::ToolMissing);
    };
    let Some(proton) = find_proton(game) else {
        return Ok(RedmodStatus::ProtonMissing);
    };
    let Some(steamapps) = game.steamapps_dir() else {
        return Ok(RedmodStatus::ProtonMissing);
    };
    let compatdata = steamapps
        .join("compatdata")
        .join(game.spec.steam_appid.to_string());
    let steam_root = steamapps.parent().unwrap_or(&steamapps).to_path_buf();

    let args = deploy_args(&exe, &game.path);
    let output = std::process::Command::new(&proton)
        .args(&args)
        .env("STEAM_COMPAT_DATA_PATH", &compatdata)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &steam_root)
        .output()
        .map_err(|e| Error::other(format!("failed to launch proton: {e}")))?;

    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(RedmodStatus::Ran {
        success: output.status.success(),
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_deploy_args() {
        let args = deploy_args(
            Path::new("/games/cp/tools/redmod/bin/redMod.exe"),
            Path::new("/games/cp"),
        );
        assert_eq!(args[0], "run");
        assert_eq!(args[2], "deploy");
        assert_eq!(args[3], "-root=/games/cp");
        assert!(args[1].ends_with("redMod.exe"));
    }
}
