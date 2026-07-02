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
    /// GameBanana numeric game id (0 if not on GameBanana).
    pub gamebanana_id: u32,
    /// Thunderstore community slug (empty if not on Thunderstore).
    pub thunderstore_slug: &'static str,
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
        4724,
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
        4724,
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
        5518,
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
        0,
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
        0,
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
        827,
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
        1446,
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
        19063,
    ),
    with_thunderstore(
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
            8722,
        ),
        // Live-verified community slug (thunderstore.io/c/cyberpunk2077).
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
        6762,
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
        5937,
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
        0,
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
        gamebanana_id: 22600,
        thunderstore_slug: "",
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
        gamebanana_id: 0,
        thunderstore_slug: "",
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
    gamebanana_id: u32,
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
        gamebanana_id,
        thunderstore_slug: "",
    }
}

/// Same as [`use_gamedir`] but with a Thunderstore community slug.
#[allow(clippy::too_many_arguments)]
const fn with_thunderstore(base: GameSpec, slug: &'static str) -> GameSpec {
    GameSpec {
        thunderstore_slug: slug,
        ..base
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
    /// Wine/Proton prefix root (the directory holding `drive_c`), for installs
    /// not managed by Steam (Heroic, Lutris). `None` = derive the prefix from
    /// the Steam library layout (`steamapps/compatdata/<appid>/pfx`).
    pub prefix: Option<PathBuf>,
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

    /// The prefix root holding `drive_c`: the explicit override (Heroic /
    /// Lutris installs) or the Steam `compatdata/<appid>/pfx` layout.
    fn prefix_root(&self) -> Option<PathBuf> {
        if let Some(p) = &self.prefix {
            return Some(p.clone());
        }
        let steamapps = self.steamapps_dir()?;
        Some(
            steamapps
                .join("compatdata")
                .join(self.spec.steam_appid.to_string())
                .join("pfx"),
        )
    }

    /// A directory under the prefix's user home. Proton uses `steamuser`;
    /// plain Wine (Heroic/Lutris runners) uses the real username — prefer
    /// whichever exists, defaulting to `steamuser`.
    fn prefix_user_dir(&self, rel: &str) -> Option<PathBuf> {
        let users = self.prefix_root()?.join("drive_c/users");
        let steamuser = users.join("steamuser");
        let home = if steamuser.is_dir() {
            steamuser
        } else {
            std::env::var("USER")
                .ok()
                .map(|u| users.join(u))
                .filter(|p| p.is_dir())
                .unwrap_or(steamuser)
        };
        Some(home.join(rel))
    }

    /// Path to `AppData/Local/<appdata>` inside the game's Wine/Proton prefix.
    /// `None` if the game has no plugin appdata or the prefix can't be located.
    /// GOG builds use a suffixed folder (e.g. "Fallout4 GOG") — prefer an
    /// existing variant over the base name.
    pub fn prefix_appdata(&self) -> Option<PathBuf> {
        if self.spec.appdata.is_empty() {
            return None;
        }
        let local = self.prefix_user_dir("AppData/Local")?;
        let base = local.join(self.spec.appdata);
        if !base.is_dir() {
            for variant in [
                format!("{} GOG", self.spec.appdata),
                format!("{} EPIC", self.spec.appdata),
                format!("{} MS", self.spec.appdata),
            ] {
                let p = local.join(&variant);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
        Some(base)
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
                    prefix: None,
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
    Ok(InstalledGame {
        spec,
        path,
        prefix: None,
    })
}

// ---- manually registered installs (non-Steam / undetected) ----------------

#[derive(serde::Serialize, serde::Deserialize)]
struct ManualEntry {
    id: String,
    path: PathBuf,
}

fn manual_games_path(data_root: &Path) -> PathBuf {
    data_root.join("manual-games.json")
}

fn load_manual_entries(data_root: &Path) -> Vec<ManualEntry> {
    std::fs::read_to_string(manual_games_path(data_root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_manual_entries(data_root: &Path, entries: &[ManualEntry]) -> Result<()> {
    std::fs::create_dir_all(data_root).map_err(|e| crate::Error::io(data_root, e))?;
    let path = manual_games_path(data_root);
    let json = serde_json::to_string_pretty(entries)?;
    std::fs::write(&path, json).map_err(|e| crate::Error::io(&path, e))
}

/// Manually registered installs (`<data_root>/manual-games.json`), skipping
/// entries whose directory no longer exists.
pub fn manual_games(data_root: &Path) -> Vec<InstalledGame> {
    load_manual_entries(data_root)
        .into_iter()
        .filter_map(|e| from_manual_path(&e.id, e.path).ok())
        .collect()
}

/// Validate and persist a manual install; returns the registered game.
/// Re-registering the same path updates its game id.
pub fn add_manual_game(data_root: &Path, id: &str, path: PathBuf) -> Result<InstalledGame> {
    let game = from_manual_path(id, path)?;
    let mut entries = load_manual_entries(data_root);
    entries.retain(|e| e.path != game.path);
    entries.push(ManualEntry {
        id: id.to_string(),
        path: game.path.clone(),
    });
    save_manual_entries(data_root, &entries)?;
    Ok(game)
}

/// Drop a manual registration by install path. No-op if not registered.
pub fn remove_manual_game(data_root: &Path, path: &Path) -> Result<()> {
    let mut entries = load_manual_entries(data_root);
    entries.retain(|e| e.path != path);
    save_manual_entries(data_root, &entries)
}
