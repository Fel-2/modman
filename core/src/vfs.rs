//! Experimental virtual-filesystem deployment via a launch wrapper.
//!
//! A global overlay mount over the live game dir needs privileges and conflicts
//! with Proton's mount namespace. The robust Linux approach is to overlay the
//! mods over the game dir *only for the game process*, using bubblewrap's
//! overlay support — the on-disk game directory stays completely pristine.
//!
//! modeman generates the wrapper command; the user sets it as the game's Steam
//! launch option (`<wrapper> %command%`). Building the command is pure and
//! tested here; running it happens at game launch on the user's machine.

use crate::game::InstalledGame;
use std::path::{Path, PathBuf};

/// Working dirs for the writable overlay layer (game writes — saves, configs —
/// land here instead of polluting the install).
pub struct OverlayDirs {
    pub upper: PathBuf,
    pub work: PathBuf,
}

impl OverlayDirs {
    /// Default overlay scratch under the game's store dir.
    pub fn under(store_dir: &Path) -> Self {
        OverlayDirs {
            upper: store_dir.join(".overlay/upper"),
            work: store_dir.join(".overlay/work"),
        }
    }
}

/// Build the `bwrap` argv that overlays `mod_dirs` over the game's deploy root.
///
/// `mod_dirs` are in load order (lowest priority first); the game dir is the
/// base layer, mods stack on top, later mods winning. The overlay is writable
/// so the game can still save. Returns argv without a trailing `%command%`.
pub fn bwrap_args(deploy_root: &Path, mod_dirs: &[PathBuf], overlay: &OverlayDirs) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "bwrap".into(),
        "--dev-bind".into(),
        "/".into(),
        "/".into(),
        // Base layer: the pristine game dir itself.
        "--overlay-src".into(),
        deploy_root.display().to_string(),
    ];
    // Higher-priority mods last.
    for dir in mod_dirs {
        args.push("--overlay-src".into());
        args.push(dir.display().to_string());
    }
    args.push("--overlay".into());
    args.push(overlay.upper.display().to_string());
    args.push(overlay.work.display().to_string());
    args.push(deploy_root.display().to_string());
    args
}

/// A ready-to-paste Steam launch option string ending in `%command%`.
pub fn steam_launch_option(
    deploy_root: &Path,
    mod_dirs: &[PathBuf],
    overlay: &OverlayDirs,
) -> String {
    let mut s = shell_join(&bwrap_args(deploy_root, mod_dirs, overlay));
    s.push_str(" -- %command%");
    s
}

/// Resolve the overlay launch option for a game + ordered mod dirs, if the game
/// uses an in-place game-dir deploy target. Returns `None` for prefix-docs
/// games (Paradox) where an overlay wrapper does not apply.
pub fn launch_option_for(
    game: &InstalledGame,
    store_dir: &Path,
    mod_dirs: &[PathBuf],
) -> Option<String> {
    let root = game.deploy_root().ok()?;
    let overlay = OverlayDirs::under(store_dir);
    Some(steam_launch_option(&root, mod_dirs, &overlay))
}

/// Minimal shell quoting for display/paste.
fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() || a.contains([' ', '\t', '"', '\'']) {
                format!("'{}'", a.replace('\'', "'\\''"))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_stacks_mods_over_game_dir() {
        let overlay = OverlayDirs {
            upper: PathBuf::from("/store/.overlay/upper"),
            work: PathBuf::from("/store/.overlay/work"),
        };
        let mods = vec![
            PathBuf::from("/store/mods/a"),
            PathBuf::from("/store/mods/b"),
        ];
        let args = bwrap_args(Path::new("/game/Data"), &mods, &overlay);

        // Base layer is the game dir, then mods in order, then the overlay dest.
        let base_pos = args.iter().position(|a| a == "/game/Data").unwrap();
        let a_pos = args.iter().position(|a| a == "/store/mods/a").unwrap();
        let b_pos = args.iter().position(|a| a == "/store/mods/b").unwrap();
        assert!(
            base_pos < a_pos && a_pos < b_pos,
            "base lowest, mods stack up"
        );
        assert!(args.contains(&"--overlay".to_string()));

        let opt = steam_launch_option(Path::new("/game/Data"), &mods, &overlay);
        assert!(opt.starts_with("bwrap "));
        assert!(opt.ends_with("-- %command%"));
    }
}
