//! High-level orchestration tying store, profiles, and deployment together,
//! with JSON persistence under the user data dir.

use crate::conflict::{self, FileConflict};
use crate::deploy::{DeployManifest, DeploySource, Deployer, LinkDeployer, LinkMethod};
use crate::error::{Error, Result};
use crate::fomod::{self, FomodConfig, FomodSession, Selections};
use crate::game::{Engine, InstalledGame, LoadOrderKind};
use crate::loadorder;
use crate::plugins;
use crate::profile::Profile;
use crate::store::{self, ModRecord};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Result of starting an install: either done, or a FOMOD wizard is required.
pub enum InstallOutcome {
    Installed(ModRecord),
    NeedsFomod {
        slug: String,
        name: String,
        config: Box<FomodConfig>,
    },
}

/// In-flight FOMOD install awaiting user selections (not persisted).
struct PendingFomod {
    session: FomodSession,
    staging: PathBuf,
    name: String,
    source: Option<String>,
}

/// Persisted per-game state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    pub mods: Vec<ModRecord>,
    pub profiles: Vec<Profile>,
    pub active_profile: String,
    /// Manifest of the currently-deployed files (None if nothing deployed).
    #[serde(default)]
    pub deployed: Option<DeployManifest>,
    /// How files are linked into the game dir.
    #[serde(default)]
    pub deploy_method: LinkMethod,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            mods: Vec::new(),
            profiles: vec![Profile::new("Default")],
            active_profile: "Default".to_string(),
            deployed: None,
            deploy_method: LinkMethod::default(),
        }
    }
}

pub struct Manager {
    data_root: PathBuf,
    game: InstalledGame,
    store_dir: PathBuf,
    state: GameState,
    deployer: LinkDeployer,
    pending_fomod: HashMap<String, PendingFomod>,
}

impl Manager {
    /// Default data root: `$XDG_DATA_HOME/modeman`.
    pub fn default_data_root() -> Result<PathBuf> {
        let base = dirs::data_dir()
            .ok_or_else(|| Error::other("no XDG data dir; set HOME"))?;
        Ok(base.join("modeman"))
    }

    /// Open (or initialize) the manager for a detected game.
    pub fn open(data_root: PathBuf, game: InstalledGame) -> Result<Self> {
        let store_dir = store::game_store_dir(&data_root, game.spec.id);
        std::fs::create_dir_all(store_dir.join("mods"))
            .map_err(|e| Error::io(store_dir.join("mods"), e))?;
        let state = load_state(&store_dir)?;
        let deployer = LinkDeployer::new(state.deploy_method);
        Ok(Manager {
            data_root,
            game,
            store_dir,
            state,
            deployer,
            pending_fomod: HashMap::new(),
        })
    }

    pub fn game(&self) -> &InstalledGame {
        &self.game
    }

    /// Root of all modeman data (parent of the per-game store).
    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn store_dir(&self) -> &Path {
        &self.store_dir
    }

    pub fn mods(&self) -> &[ModRecord] {
        &self.state.mods
    }

    pub fn is_deployed(&self) -> bool {
        self.state.deployed.is_some()
    }

    pub fn deploy_method(&self) -> LinkMethod {
        self.state.deploy_method
    }

    /// Change the link method. Re-deploys if something is currently deployed
    /// so the live tree uses the new method.
    pub fn set_deploy_method(&mut self, method: LinkMethod) -> Result<()> {
        if self.state.deploy_method == method {
            return Ok(());
        }
        let was_deployed = self.is_deployed();
        if was_deployed {
            self.clear()?;
        }
        self.state.deploy_method = method;
        self.deployer = LinkDeployer::new(method);
        self.save()?;
        if was_deployed {
            self.deploy()?;
        }
        Ok(())
    }

    fn state_path(&self) -> PathBuf {
        self.store_dir.join("state.json")
    }

    pub fn save(&self) -> Result<()> {
        let path = self.state_path();
        let json = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(&path, json).map_err(|e| Error::io(&path, e))?;
        Ok(())
    }

    // ---- profile access -------------------------------------------------

    pub fn active_profile(&self) -> &Profile {
        self.state
            .profiles
            .iter()
            .find(|p| p.name == self.state.active_profile)
            .unwrap_or(&self.state.profiles[0])
    }

    fn active_profile_mut(&mut self) -> &mut Profile {
        let name = self.state.active_profile.clone();
        let idx = self
            .state
            .profiles
            .iter()
            .position(|p| p.name == name)
            .unwrap_or(0);
        &mut self.state.profiles[idx]
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.state.profiles.iter().map(|p| p.name.clone()).collect()
    }

    pub fn set_active_profile(&mut self, name: &str) -> Result<()> {
        if !self.state.profiles.iter().any(|p| p.name == name) {
            return Err(Error::ProfileNotFound(name.to_string()));
        }
        self.state.active_profile = name.to_string();
        self.save()
    }

    pub fn create_profile(&mut self, name: &str) -> Result<()> {
        if self.state.profiles.iter().any(|p| p.name == name) {
            return Err(Error::other(format!("profile '{name}' already exists")));
        }
        let mut p = Profile::new(name);
        // New profile tracks all known mods, enabled by default.
        for m in &self.state.mods {
            p.ensure(&m.slug);
        }
        self.state.profiles.push(p);
        self.save()
    }

    // ---- mod operations -------------------------------------------------

    /// Start installing a mod archive. Plain archives finish immediately and
    /// return `Installed`; FOMOD installers return `NeedsFomod` and stay staged
    /// until [`finish_fomod`](Self::finish_fomod) (or [`cancel_fomod`]).
    pub fn install_archive(&mut self, archive_path: &Path) -> Result<InstallOutcome> {
        let staged = store::extract_staging(&self.store_dir, archive_path, &self.state.mods)?;
        let source = store::source_label(archive_path);

        if let Some((cfg_path, src_root)) = fomod::find_config(&staged.dir) {
            let session = FomodSession::load(&cfg_path, src_root)?;
            let config = Box::new(session.config.clone());
            let slug = staged.slug.clone();
            let name = staged.name.clone();
            self.pending_fomod.insert(
                slug.clone(),
                PendingFomod { session, staging: staged.dir, name: name.clone(), source },
            );
            return Ok(InstallOutcome::NeedsFomod { slug, name, config });
        }

        // Plain archive: promote staging to the final mod dir.
        let final_dir = store::mod_dir(&self.store_dir, &staged.slug);
        store::finalize_direct(&staged.dir, &final_dir, self.game.spec.flatten)?;
        // Prefer the mod's real declared name over the archive filename.
        let name = match self.game.spec.engine {
            Engine::Stardew | Engine::Unity => {
                loadorder::smapi::manifest_name(&final_dir).unwrap_or(staged.name)
            }
            Engine::RimWorld => loadorder::rimworld::mod_name(&final_dir).unwrap_or(staged.name),
            _ => staged.name,
        };
        let record = ModRecord { slug: staged.slug, name, source };
        self.register_record(record.clone())?;
        Ok(InstallOutcome::Installed(record))
    }

    /// Complete a staged FOMOD install with the user's selections.
    pub fn finish_fomod(&mut self, slug: &str, selections: &Selections) -> Result<ModRecord> {
        let pending = self
            .pending_fomod
            .remove(slug)
            .ok_or_else(|| Error::other(format!("no pending FOMOD for '{slug}'")))?;
        let final_dir = store::mod_dir(&self.store_dir, slug);
        let _ = std::fs::remove_dir_all(&final_dir);
        let res = pending.session.install(selections, &final_dir);
        store::discard_staging(&pending.staging);
        res?;
        let record = ModRecord {
            slug: slug.to_string(),
            name: pending.name,
            source: pending.source,
        };
        self.register_record(record.clone())?;
        Ok(record)
    }

    /// Abandon a staged FOMOD install.
    pub fn cancel_fomod(&mut self, slug: &str) {
        if let Some(p) = self.pending_fomod.remove(slug) {
            store::discard_staging(&p.staging);
        }
    }

    fn register_record(&mut self, record: ModRecord) -> Result<()> {
        self.state.mods.push(record.clone());
        for p in &mut self.state.profiles {
            p.ensure(&record.slug);
        }
        self.save()
    }

    /// Run a Cyberpunk REDmod deploy (compiles `mods/` into `archive/pc/mod`).
    /// Experimental: requires the bundled `redMod.exe` + a Proton runtime, and
    /// is only meaningful on a real install. No-op for other engines.
    pub fn redmod_deploy(&self) -> Result<crate::redmod::RedmodStatus> {
        if self.game.spec.engine != Engine::Cyberpunk {
            return Ok(crate::redmod::RedmodStatus::NoRedmodMods);
        }
        let dirs: Vec<PathBuf> = self
            .active_profile()
            .enabled_in_order()
            .filter_map(|slug| {
                self.state
                    .mods
                    .iter()
                    .find(|m| m.slug == slug)
                    .map(|m| m.dir(&self.store_dir))
            })
            .collect();
        crate::redmod::run_deploy(&self.game, &dirs)
    }

    /// Experimental VFS: a bubblewrap launch-wrapper string to paste into the
    /// game's Steam launch options. Overlays the active profile's mods over the
    /// game dir for the game process only, keeping the install pristine.
    pub fn vfs_launch_option(&self) -> Option<String> {
        let dirs: Vec<PathBuf> = self
            .active_profile()
            .enabled_in_order()
            .filter_map(|slug| {
                self.state
                    .mods
                    .iter()
                    .find(|m| m.slug == slug)
                    .map(|m| m.dir(&self.store_dir))
            })
            .collect();
        crate::vfs::launch_option_for(&self.game, &self.store_dir, &dirs)
    }

    /// File conflicts among the active profile's enabled mods, in load order.
    pub fn conflicts(&self) -> Vec<FileConflict> {
        let slugs: Vec<String> =
            self.active_profile().enabled_in_order().map(String::from).collect();
        let sources = conflict::sources_for(slugs.iter().map(|s| s.as_str()), |s| {
            self.state
                .mods
                .iter()
                .find(|m| m.slug == s)
                .map(|m| m.dir(&self.store_dir))
        });
        conflict::detect(&sources)
    }

    /// Remove a mod from the store and all profiles.
    pub fn remove_mod(&mut self, slug: &str) -> Result<()> {
        let idx = self
            .state
            .mods
            .iter()
            .position(|m| m.slug == slug)
            .ok_or_else(|| Error::ModNotFound(slug.to_string()))?;
        let record = self.state.mods.remove(idx);
        store::remove(&self.store_dir, &record)?;
        for p in &mut self.state.profiles {
            p.remove(slug);
        }
        self.save()
    }

    pub fn set_enabled(&mut self, slug: &str, enabled: bool) -> Result<()> {
        self.active_profile_mut().set_enabled(slug, enabled);
        self.save()
    }

    pub fn move_mod(&mut self, from: usize, to: usize) -> Result<()> {
        self.active_profile_mut().move_to(from, to);
        self.save()
    }

    // ---- deployment -----------------------------------------------------

    /// Deploy the active profile into the game dir. Reverts any prior deploy
    /// first so the live tree always matches the current profile exactly.
    pub fn deploy(&mut self) -> Result<()> {
        self.clear()?;
        let target = self.game.deploy_root()?;
        std::fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;

        let slugs: Vec<String> = self
            .active_profile()
            .enabled_in_order()
            .map(String::from)
            .collect();

        let mut sources = Vec::new();
        for (rank, slug) in slugs.iter().enumerate() {
            let Some(rec) = self.state.mods.iter().find(|m| &m.slug == slug) else {
                continue;
            };
            let dir = rec.dir(&self.store_dir);
            if dir.is_dir() {
                sources.push(DeploySource { root: dir, rank });
            }
        }

        let manifest = self.deployer.deploy(&sources, &target)?;
        self.state.deployed = Some(manifest);

        // Activate Creation Engine plugins so the game actually loads them.
        if self.game.spec.load_order == LoadOrderKind::PluginsTxt {
            let active = self.active_plugins_in_order(&slugs);
            let managed = self.all_managed_plugins();
            if let Err(e) = plugins::write_plugins_txt(&self.game, &active, &managed) {
                tracing::warn!("plugins.txt update failed: {e}");
            }
        }
        // RimWorld: write ModsConfig.xml active-mod order from packageIds.
        if self.game.spec.engine == Engine::RimWorld {
            let active = self.rimworld_active_in_order(&slugs);
            let managed = self.rimworld_all_packages();
            if let Err(e) = loadorder::rimworld::write(&self.game, &active, &managed) {
                tracing::warn!("ModsConfig.xml update failed: {e}");
            }
        }
        // Paradox: write dlc_load.json + reflect into the launcher playset DB.
        if self.game.spec.engine == Engine::Paradox {
            if let Some(json_dir) = target.parent() {
                let active = self.paradox_active_in_order(&slugs);
                let managed = self.paradox_all_descriptors();
                if let Err(e) = loadorder::paradox::write(json_dir, &active, &managed) {
                    tracing::warn!("dlc_load.json update failed: {e}");
                }
                let db = json_dir.join("launcher-v2.sqlite");
                if let Err(e) = crate::paradoxdb::sync_file(&db, &active, &managed) {
                    tracing::warn!("launcher playset DB update failed: {e}");
                }
            }
        }
        self.save()
    }

    /// Revert the live deployment, leaving the game dir clean.
    pub fn clear(&mut self) -> Result<()> {
        if let Some(manifest) = self.state.deployed.take() {
            let target = self.game.deploy_root()?;
            self.deployer.clear(&target, &manifest)?;
        }
        if self.game.spec.load_order == LoadOrderKind::PluginsTxt {
            let managed = self.all_managed_plugins();
            if let Err(e) = plugins::clear_plugins_txt(&self.game, &managed) {
                tracing::warn!("plugins.txt clear failed: {e}");
            }
        }
        if self.game.spec.engine == Engine::RimWorld {
            let managed = self.rimworld_all_packages();
            if let Err(e) = loadorder::rimworld::clear(&self.game, &managed) {
                tracing::warn!("ModsConfig.xml clear failed: {e}");
            }
        }
        if self.game.spec.engine == Engine::Paradox {
            if let Some(json_dir) = self.game.deploy_root().ok().and_then(|r| r.parent().map(Path::to_path_buf)) {
                let managed = self.paradox_all_descriptors();
                if let Err(e) = loadorder::paradox::clear(&json_dir, &managed) {
                    tracing::warn!("dlc_load.json clear failed: {e}");
                }
            }
        }
        self.save()
    }

    /// Enabled Paradox `.mod` descriptor refs in load order (deduped).
    fn paradox_active_in_order(&self, slugs: &[String]) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for slug in slugs {
            if let Some(rec) = self.state.mods.iter().find(|m| &m.slug == slug) {
                for d in loadorder::paradox::descriptors(&rec.dir(&self.store_dir)) {
                    if seen.insert(d.to_ascii_lowercase()) {
                        out.push(d);
                    }
                }
            }
        }
        out
    }

    /// Every Paradox descriptor ref modeman controls.
    fn paradox_all_descriptors(&self) -> Vec<String> {
        let mut out = Vec::new();
        for rec in &self.state.mods {
            out.extend(loadorder::paradox::descriptors(&rec.dir(&self.store_dir)));
        }
        out
    }

    /// Enabled RimWorld packageIds in load order (deduped).
    fn rimworld_active_in_order(&self, slugs: &[String]) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for slug in slugs {
            if let Some(rec) = self.state.mods.iter().find(|m| &m.slug == slug) {
                if let Some(pid) = loadorder::rimworld::package_id(&rec.dir(&self.store_dir)) {
                    if seen.insert(pid.clone()) {
                        out.push(pid);
                    }
                }
            }
        }
        out
    }

    /// Every RimWorld packageId modeman controls.
    fn rimworld_all_packages(&self) -> Vec<String> {
        self.state
            .mods
            .iter()
            .filter_map(|rec| loadorder::rimworld::package_id(&rec.dir(&self.store_dir)))
            .collect()
    }

    /// Plugin filenames provided by the given enabled mod slugs, in order,
    /// de-duplicated (first occurrence wins).
    fn active_plugins_in_order(&self, slugs: &[String]) -> Vec<String> {
        let mut seen = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for slug in slugs {
            let Some(rec) = self.state.mods.iter().find(|m| &m.slug == slug) else {
                continue;
            };
            for p in plugins::plugins_in(&rec.dir(&self.store_dir)) {
                if seen.insert(p.to_ascii_lowercase()) {
                    out.push(p);
                }
            }
        }
        out
    }

    /// Every plugin filename modeman controls across all installed mods.
    fn all_managed_plugins(&self) -> Vec<String> {
        let mut out = Vec::new();
        for rec in &self.state.mods {
            out.extend(plugins::plugins_in(&rec.dir(&self.store_dir)));
        }
        out
    }
}

fn load_state(store_dir: &Path) -> Result<GameState> {
    let path = store_dir.join("state.json");
    if !path.exists() {
        let state = GameState::default();
        let json = serde_json::to_string_pretty(&state)?;
        std::fs::write(&path, json).map_err(|e| Error::io(&path, e))?;
        return Ok(state);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
    let state = serde_json::from_str(&text)?;
    Ok(state)
}
