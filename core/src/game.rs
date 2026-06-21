//! Supported-game catalog and on-disk detection (Steam, GOG).

use crate::error::Result;
use crate::vdf;
use std::path::{Path, PathBuf};

/// Static description of a moddable game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSpec {
    /// Stable internal slug, e.g. "skyrimse".
    pub id: &'static str,
    pub name: &'static str,
    /// Steam app id (0 if not on Steam / manual-only).
    pub steam_appid: u32,
    /// Where deployed mod files go.
    pub deploy: DeployTarget,
    /// Whether to hoist a single wrapper folder during install. Bethesda-style
    /// loose-file mods → true; folder-per-mod games (RimWorld, Stardew) → false,
    /// because the wrapper folder IS the mod and must be preserved.
    pub flatten: bool,
    /// Engine family — drives load-order handling.
    pub engine: Engine,
    /// Folder name under `AppData/Local` (inside the Proton prefix) where the
    /// game's load-order file lives. Empty when not applicable.
    pub appdata: &'static str,
    /// How this game records load order.
    pub load_order: LoadOrderKind,
    /// Nexus Mods domain slug (used in API paths and `nxm://` links).
    /// Empty if the game is not on Nexus.
    pub nexus_domain: &'static str,
}

/// Where a game's deployed mods live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployTarget {
    /// Path relative to the install dir (`""` = install root).
    GameDir(&'static str),
    /// Path relative to `Documents/` inside the Proton prefix (Paradox games).
    PrefixDocs(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Creation Engine / Gamebryo — plugins.txt load order, Data/ root.
    Bethesda,
    /// REDengine — archive/pc/mod, redscript, etc.
    Cyberpunk,
    /// RimWorld — folder-per-mod under Mods/, ModsConfig.xml order.
    RimWorld,
    /// Stardew Valley — SMAPI mods under Mods/.
    Stardew,
    /// Paradox (Crusader Kings, etc) — descriptor + folder in Documents/.../mod.
    Paradox,
    /// Generic Unity game modded via BepInEx.
    Unity,
}

/// Mechanism a game uses to record which plugins load and in what order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOrderKind {
    /// Modern Creation Engine `plugins.txt` (`*name.esp` = active).
    PluginsTxt,
    /// Morrowind `Morrowind.ini` `[Game Files]` section.
    MorrowindIni,
    /// No plugin list (REDengine, etc).
    None,
}

/// Known games. Extend freely; detection keys off `steam_appid` (0 = manual).
pub const CATALOG: &[GameSpec] = &[
    use_gamedir(
        "skyrimse",
        "Skyrim Special Edition",
        489830,
        "Data",
        true,
        Engine::Bethesda,
        "Skyrim Special Edition",
        LoadOrderKind::PluginsTxt,
        "skyrimspecialedition",
    ),
    use_gamedir(
        "skyrim",
        "Skyrim",
        72850,
        "Data",
        true,
        Engine::Bethesda,
        "Skyrim",
        LoadOrderKind::PluginsTxt,
        "skyrim",
    ),
    use_gamedir(
        "fallout4",
        "Fallout 4",
        377160,
        "Data",
        true,
        Engine::Bethesda,
        "Fallout4",
        LoadOrderKind::PluginsTxt,
        "fallout4",
    ),
    use_gamedir(
        "falloutnv",
        "Fallout: New Vegas",
        22380,
        "Data",
        true,
        Engine::Bethesda,
        "FalloutNV",
        LoadOrderKind::PluginsTxt,
        "newvegas",
    ),
    use_gamedir(
        "fallout3",
        "Fallout 3",
        22300,
        "Data",
        true,
        Engine::Bethesda,
        "Fallout3",
        LoadOrderKind::PluginsTxt,
        "fallout3",
    ),
    use_gamedir(
        "oblivion",
        "Oblivion",
        22330,
        "Data",
        true,
        Engine::Bethesda,
        "Oblivion",
        LoadOrderKind::PluginsTxt,
        "oblivion",
    ),
    use_gamedir(
        "morrowind",
        "Morrowind",
        22320,
        "Data Files",
        true,
        Engine::Bethesda,
        "",
        LoadOrderKind::MorrowindIni,
        "morrowind",
    ),
    use_gamedir(
        "starfield",
        "Starfield",
        1716740,
        "Data",
        true,
        Engine::Bethesda,
        "Starfield",
        LoadOrderKind::PluginsTxt,
        "starfield",
    ),
    use_gamedir(
        "cyberpunk",
        "Cyberpunk 2077",
        1091500,
        "",
        true,
        Engine::Cyberpunk,
        "",
        LoadOrderKind::None,
        "cyberpunk2077",
    ),
    // Folder-per-mod games — wrapper folder is the mod, do not flatten.
    use_gamedir(
        "rimworld",
        "RimWorld",
        294100,
        "Mods",
        false,
        Engine::RimWorld,
        "",
        LoadOrderKind::None,
        "rimworld",
    ),
    use_gamedir(
        "stardew",
        "Stardew Valley",
        413150,
        "Mods",
        false,
        Engine::Stardew,
        "",
        LoadOrderKind::None,
        "stardewvalley",
    ),
    // Generic Unity / BepInEx — manual-add (no fixed appid).
    use_gamedir(
        "unity",
        "Generic Unity (BepInEx)",
        0,
        "BepInEx/plugins",
        false,
        Engine::Unity,
        "",
        LoadOrderKind::None,
        "",
    ),
    // Paradox — mods deploy to Documents/.../mod inside the prefix.
    GameSpec {
        id: "ck3",
        name: "Crusader Kings III",
        steam_appid: 1158310,
        deploy: DeployTarget::PrefixDocs("Paradox Interactive/Crusader Kings III/mod"),
        flatten: false,
        engine: Engine::Paradox,
        appdata: "",
        load_order: LoadOrderKind::None,
        nexus_domain: "crusaderkings3",
    },
    GameSpec {
        id: "ck2",
        name: "Crusader Kings II",
        steam_appid: 203770,
        deploy: DeployTarget::PrefixDocs("Paradox Interactive/Crusader Kings II/mod"),
        flatten: false,
        engine: Engine::Paradox,
        appdata: "",
        load_order: LoadOrderKind::None,
        nexus_domain: "crusaderkings2",
    },
];

/// Const helper for the common `GameDir` deploy target.
#[allow(clippy::too_many_arguments)]
const fn use_gamedir(
    id: &'static str,
    name: &'static str,
    steam_appid: u32,
    root: &'static str,
    flatten: bool,
    engine: Engine,
    appdata: &'static str,
    load_order: LoadOrderKind,
    nexus_domain: &'static str,
) -> GameSpec {
    GameSpec {
        id,
        name,
        steam_appid,
        deploy: DeployTarget::GameDir(root),
        flatten,
        engine,
        appdata,
        load_order,
        nexus_domain,
    }
}

pub fn spec_by_id(id: &str) -> Option<&'static GameSpec> {
    CATALOG.iter().find(|g| g.id == id)
}

fn spec_by_appid(appid: u32) -> Option<&'static GameSpec> {
    CATALOG.iter().find(|g| g.steam_appid == appid)
}

/// A detected install on this machine.
#[derive(Debug, Clone)]
pub struct InstalledGame {
    pub spec: &'static GameSpec,
    /// Absolute path to the game install directory.
    pub path: PathBuf,
}

impl InstalledGame {
    /// Absolute directory mods are deployed into. May fail for prefix-based
    /// targets when the Proton prefix cannot be located.
    pub fn deploy_root(&self) -> crate::Result<PathBuf> {
        match self.spec.deploy {
            DeployTarget::GameDir(root) => Ok(if root.is_empty() {
                self.path.clone()
            } else {
                self.path.join(root)
            }),
            DeployTarget::PrefixDocs(sub) => self.prefix_documents(sub).ok_or_else(|| {
                crate::Error::GameNotInstalled(format!(
                    "{}: Proton prefix / Documents not found (run the game once)",
                    self.spec.name
                ))
            }),
        }
    }

    /// The Steam library's `steamapps` dir, derived from the install path
    /// (`<lib>/steamapps/common/<installdir>`).
    pub fn steamapps_dir(&self) -> Option<PathBuf> {
        // path -> common -> steamapps
        self.path.parent()?.parent().map(|p| p.to_path_buf())
    }

    /// `Documents/<sub>` inside the game's Proton prefix.
    pub fn prefix_documents(&self, sub: &str) -> Option<PathBuf> {
        let docs = self.prefix_user_dir("Documents")?;
        Some(if sub.is_empty() { docs } else { docs.join(sub) })
    }

    /// `AppData/LocalLow/<sub>` inside the prefix (Unity games like RimWorld).
    pub fn prefix_locallow(&self, sub: &str) -> Option<PathBuf> {
        let base = self.prefix_user_dir("AppData/LocalLow")?;
        Some(if sub.is_empty() { base } else { base.join(sub) })
    }

    /// A directory under the prefix's `steamuser` home.
    fn prefix_user_dir(&self, rel: &str) -> Option<PathBuf> {
        let steamapps = self.steamapps_dir()?;
        Some(
            steamapps
                .join("compatdata")
                .join(self.spec.steam_appid.to_string())
                .join("pfx/drive_c/users/steamuser")
                .join(rel),
        )
    }

    /// Path to `AppData/Local/<appdata>` inside the game's Proton prefix.
    /// `None` if the game has no plugin appdata or the prefix can't be located.
    pub fn prefix_appdata(&self) -> Option<PathBuf> {
        if self.spec.appdata.is_empty() {
            return None;
        }
        let steamapps = self.steamapps_dir()?;
        let dir = steamapps
            .join("compatdata")
            .join(self.spec.steam_appid.to_string())
            .join("pfx/drive_c/users/steamuser/AppData/Local")
            .join(self.spec.appdata);
        Some(dir)
    }
}

/// Detect all catalog games installed via Steam.
pub fn detect_all() -> Vec<InstalledGame> {
    let mut found = Vec::new();
    for lib in steam_library_paths() {
        scan_library(&lib, &mut found);
    }
    found
}

/// Candidate Steam root directories on Linux (native + Flatpak).
fn steam_roots() -> Vec<PathBuf> {
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Vec::new(),
    };
    [
        home.join(".steam/steam"),
        home.join(".local/share/Steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
    .into_iter()
    .filter(|p| p.is_dir())
    .collect()
}

/// Resolve all Steam library folders (each holds `steamapps/`).
fn steam_library_paths() -> Vec<PathBuf> {
    let mut libs = Vec::new();
    for root in steam_roots() {
        let vdf_path = root.join("steamapps/libraryfolders.vdf");
        let Ok(text) = std::fs::read_to_string(&vdf_path) else {
            continue;
        };
        let Ok(doc) = vdf::parse(&text) else { continue };
        let Some(lf) = doc.get("libraryfolders") else {
            continue;
        };
        if let Some(pairs) = lf.as_obj() {
            for (_idx, entry) in pairs {
                if let Some(path) = entry.get("path").and_then(|v| v.as_str()) {
                    let p = PathBuf::from(path);
                    if p.is_dir() && !libs.contains(&p) {
                        libs.push(p);
                    }
                }
            }
        }
    }
    // The root itself is always an implicit library.
    for root in steam_roots() {
        if !libs.contains(&root) {
            libs.push(root);
        }
    }
    libs
}

/// Scan one library's `steamapps` for catalog appmanifests.
fn scan_library(lib: &Path, out: &mut Vec<InstalledGame>) {
    let steamapps = lib.join("steamapps");
    let Ok(entries) = std::fs::read_dir(&steamapps) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("appmanifest_") else {
            continue;
        };
        let Some(appid_str) = rest.strip_suffix(".acf") else {
            continue;
        };
        let Ok(appid) = appid_str.parse::<u32>() else {
            continue;
        };
        let Some(spec) = spec_by_appid(appid) else {
            continue;
        };
        if let Some(install) = resolve_install_dir(&steamapps, &entry.path()) {
            if out.iter().all(|g| g.path != install) {
                out.push(InstalledGame {
                    spec,
                    path: install,
                });
            }
        }
    }
}

/// Read an appmanifest to find `installdir` and build the install path.
fn resolve_install_dir(steamapps: &Path, manifest: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let doc = vdf::parse(&text).ok()?;
    let installdir = doc
        .get("AppState")
        .and_then(|s| s.get("installdir"))
        .and_then(|v| v.as_str())?;
    let path = steamapps.join("common").join(installdir);
    path.is_dir().then_some(path)
}

/// Manual detection helper: validate a user-supplied install path for a game id.
pub fn from_manual_path(id: &str, path: PathBuf) -> Result<InstalledGame> {
    let spec = spec_by_id(id).ok_or_else(|| crate::Error::UnknownGame(id.to_string()))?;
    if !path.is_dir() {
        return Err(crate::Error::GameNotInstalled(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    Ok(InstalledGame { spec, path })
}
