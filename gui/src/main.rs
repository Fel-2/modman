// modeman — Linux-first game mod manager (GUI).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

use modeman_core::deploy::LinkMethod;
use modeman_core::fomod::{FomodConfig, GroupKind, PluginType, Selections};
use modeman_core::manager::InstallOutcome;
use modeman_core::{game, FileConflict, Manager, NexusRef};
use modeman_nexus::{ModList, NexusClient, NxmLink};
use modeman_platform::{
    gamebanana::GameBanana, modio::Modio, thunderstore::Thunderstore, ListSort, ModPlatform,
};

/// Source-agnostic browse row.
#[derive(Clone)]
struct BrowseEntry {
    id: String,
    name: String,
    author: String,
    summary: String,
    downloads: u64,
}

/// Source-agnostic downloadable file. `url` is set for free platforms; `None`
/// means a Nexus file that must be link-resolved (premium) at download time.
#[derive(Clone)]
struct BrowseFileEntry {
    id: String,
    name: String,
    version: String,
    size: u64,
    url: Option<String>,
}

/// Browse sources, by combo index.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Nexus,
    Thunderstore,
    Modio,
    GameBanana,
}

impl Source {
    fn from_index(i: usize) -> Source {
        match i {
            1 => Source::Thunderstore,
            2 => Source::Modio,
            3 => Source::GameBanana,
            _ => Source::Nexus,
        }
    }
    fn labels() -> [&'static str; 4] {
        ["Nexus", "Thunderstore", "mod.io", "GameBanana"]
    }
}

/// Build a free-platform client for a source (not Nexus).
fn make_platform(
    src: Source,
    modio_key: &str,
) -> std::result::Result<Box<dyn ModPlatform>, String> {
    match src {
        Source::Thunderstore => Thunderstore::new()
            .map(|p| Box::new(p) as Box<dyn ModPlatform>)
            .map_err(|e| e.to_string()),
        Source::Modio => Modio::new(modio_key)
            .map(|p| Box::new(p) as Box<dyn ModPlatform>)
            .map_err(|e| e.to_string()),
        Source::GameBanana => GameBanana::new()
            .map(|p| Box::new(p) as Box<dyn ModPlatform>)
            .map_err(|e| e.to_string()),
        Source::Nexus => Err("nexus uses its own client".into()),
    }
}
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// Messages from background worker threads to the UI loop.
enum Bg {
    NexusStatus(String),
    Downloaded {
        path: PathBuf,
        /// Domain to switch the active game to (Nexus); `None` installs into
        /// the currently-open game (platform browse).
        switch: Option<String>,
        /// Nexus (mod_id, file_id, version) if downloaded from Nexus.
        nexus: Option<(u64, u64, String)>,
    },
    BrowseList(Vec<BrowseEntry>),
    BrowseFileList {
        title: String,
        files: Vec<BrowseFileEntry>,
    },
    BrowseMsg(String),
}

/// A staged FOMOD install being configured in the wizard.
struct Wizard {
    slug: String,
    config: FomodConfig,
    selections: Selections,
    step: usize,
    src_root: PathBuf,
    /// Currently focused plugin (group, plugin) for the preview pane.
    sel: Option<(usize, usize)>,
}

/// Mutable app state shared across UI callbacks.
struct App {
    data_root: PathBuf,
    games: Vec<game::InstalledGame>,
    current: Option<usize>,
    manager: Option<Manager>,
    status: String,
    api_key: String,
    nexus_status: String,
    tx: Sender<Bg>,
    conflicts: Vec<FileConflict>,
    conflicts_open: bool,
    wizard: Option<Wizard>,
    browse_open: bool,
    browse_mods: Vec<BrowseEntry>,
    browse_files: Vec<BrowseFileEntry>,
    browse_title: String,
    browse_status: String,
    browse_sel_mod: Option<String>,
    browse_platform: usize,
    browse_game_id: String,
    modio_key: String,
}

impl App {
    fn new(tx: Sender<Bg>) -> Self {
        let data_root =
            Manager::default_data_root().unwrap_or_else(|_| PathBuf::from("./modeman-data"));
        let api_key = std::fs::read_to_string(data_root.join("nexus-apikey.txt"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let modio_key = std::fs::read_to_string(data_root.join("modio-apikey.txt"))
            .unwrap_or_default()
            .trim()
            .to_string();
        App {
            data_root,
            games: Vec::new(),
            current: None,
            manager: None,
            status: "Ready.".into(),
            api_key,
            nexus_status: "Not signed in.".into(),
            tx,
            conflicts: Vec::new(),
            conflicts_open: false,
            wizard: None,
            browse_open: false,
            browse_mods: Vec::new(),
            browse_files: Vec::new(),
            browse_title: "Browse".into(),
            browse_status: String::new(),
            browse_sel_mod: None,
            browse_platform: 0,
            browse_game_id: String::new(),
            modio_key,
        }
    }

    /// Nexus domain for the current game, if any.
    fn nexus_domain(&self) -> Option<String> {
        self.manager
            .as_ref()
            .map(|m| m.game().spec.nexus_domain)
            .filter(|d| !d.is_empty())
            .map(String::from)
    }

    fn source(&self) -> Source {
        Source::from_index(self.browse_platform)
    }

    /// The game id/slug to query for the current source.
    fn browse_game(&self) -> String {
        match self.source() {
            Source::Nexus => self.nexus_domain().unwrap_or_default(),
            _ => self.browse_game_id.clone(),
        }
    }

    fn save_modio_key(&mut self, key: &str) {
        self.modio_key = key.trim().to_string();
        let _ = std::fs::create_dir_all(&self.data_root);
        let _ = std::fs::write(self.data_root.join("modio-apikey.txt"), &self.modio_key);
    }

    /// Kick off a browse listing for the current source + game.
    fn trigger_list(&mut self, sort: ListSort) {
        let game = self.browse_game();
        if game.is_empty() {
            self.browse_status = "Enter a game id / community slug for this source.".into();
            return;
        }
        self.browse_status = "Loading…".into();
        match self.source() {
            Source::Nexus => {
                if self.api_key.is_empty() {
                    self.browse_status = "Set a Nexus API key first.".into();
                    return;
                }
                spawn_nexus_list(self.tx.clone(), self.api_key.clone(), game, sort);
            }
            src => spawn_pl_list(self.tx.clone(), src, self.modio_key.clone(), game, sort),
        }
    }

    /// Kick off fetching a mod's files for the current source.
    fn trigger_files(&mut self, mod_id: String) {
        let game = self.browse_game();
        self.browse_sel_mod = Some(mod_id.clone());
        self.browse_status = "Loading files…".into();
        match self.source() {
            Source::Nexus => match mod_id.parse::<u64>() {
                Ok(id) if !self.api_key.is_empty() => {
                    spawn_nexus_files(self.tx.clone(), self.api_key.clone(), game, id)
                }
                _ => self.browse_status = "Set a Nexus API key first.".into(),
            },
            src => spawn_pl_files(self.tx.clone(), src, self.modio_key.clone(), game, mod_id),
        }
    }

    fn cache_dir(&self) -> PathBuf {
        self.data_root.join(".cache")
    }

    fn save_key(&mut self, key: &str) {
        self.api_key = key.trim().to_string();
        let _ = std::fs::create_dir_all(&self.data_root);
        let path = self.data_root.join("nexus-apikey.txt");
        match std::fs::write(&path, &self.api_key) {
            Ok(_) => self.nexus_status = "API key saved.".into(),
            Err(e) => self.nexus_status = format!("Could not save key: {e}"),
        }
    }

    /// Detect installed games; open the first (or keep current if still present).
    fn rescan(&mut self) {
        self.games = game::detect_all();
        if self.games.is_empty() {
            self.current = None;
            self.manager = None;
            self.status = "No supported games detected (Steam libraries scanned).".into();
            return;
        }
        let idx = self.current.filter(|&i| i < self.games.len()).unwrap_or(0);
        self.open_game(idx);
        self.status = format!("Detected {} game(s).", self.games.len());
    }

    fn open_game(&mut self, idx: usize) {
        let Some(g) = self.games.get(idx).cloned() else {
            return;
        };
        match Manager::open(self.data_root.clone(), g) {
            Ok(m) => {
                self.current = Some(idx);
                self.manager = Some(m);
            }
            Err(e) => {
                self.status = format!("Failed to open game: {e}");
                self.manager = None;
            }
        }
    }

    /// Switch to the installed game matching a Nexus domain. Returns success.
    fn open_game_by_domain(&mut self, domain: &str) -> bool {
        if let Some(idx) = self
            .games
            .iter()
            .position(|g| g.spec.nexus_domain == domain)
        {
            self.open_game(idx);
            true
        } else {
            false
        }
    }

    fn check_conflicts(&mut self) {
        self.conflicts = self
            .manager
            .as_ref()
            .map(|m| m.conflicts())
            .unwrap_or_default();
        self.conflicts_open = true;
        self.status = format!("{} conflicting file(s).", self.conflicts.len());
    }

    /// Begin a FOMOD wizard for a freshly staged install.
    fn start_wizard(&mut self, slug: String, config: FomodConfig, src_root: PathBuf) {
        let selections = config.default_selections();
        self.wizard = Some(Wizard {
            slug,
            config,
            selections,
            step: 0,
            src_root,
            sel: None,
        });
    }

    /// Apply a wizard checkbox toggle, enforcing the group's cardinality.
    fn fomod_toggle(&mut self, gi: usize, pi: usize, checked: bool) {
        let Some(w) = self.wizard.as_mut() else {
            return;
        };
        let step = w.step;
        let Some(kind) = w
            .config
            .steps
            .get(step)
            .and_then(|s| s.groups.get(gi))
            .map(|g| g.kind)
        else {
            return;
        };
        let Some(sel) = w.selections.get_mut(step).and_then(|s| s.get_mut(gi)) else {
            return;
        };
        match kind {
            GroupKind::ExactlyOne => {
                if checked {
                    for (i, s) in sel.iter_mut().enumerate() {
                        *s = i == pi;
                    }
                }
            }
            GroupKind::AtMostOne => {
                if checked {
                    for (i, s) in sel.iter_mut().enumerate() {
                        *s = i == pi;
                    }
                } else if let Some(s) = sel.get_mut(pi) {
                    *s = false;
                }
            }
            GroupKind::All => sel.iter_mut().for_each(|s| *s = true),
            _ => {
                if let Some(s) = sel.get_mut(pi) {
                    *s = checked;
                }
            }
        }
    }

    fn fomod_select(&mut self, gi: usize, pi: usize) {
        if let Some(w) = self.wizard.as_mut() {
            w.sel = Some((gi, pi));
        }
    }

    fn fomod_step(&mut self, delta: isize) {
        if let Some(w) = self.wizard.as_mut() {
            let last = w.config.steps.len().saturating_sub(1);
            let next = (w.step as isize + delta).clamp(0, last as isize) as usize;
            w.step = next;
        }
    }

    fn fomod_install(&mut self) {
        let Some(w) = self.wizard.take() else { return };
        if let Some(mgr) = self.manager.as_mut() {
            match mgr.finish_fomod(&w.slug, &w.selections) {
                Ok(rec) => self.status = format!("Installed '{}' (FOMOD).", rec.name),
                Err(e) => self.status = format!("FOMOD install failed: {e}"),
            }
        }
    }

    fn fomod_cancel(&mut self) {
        if let Some(w) = self.wizard.take() {
            if let Some(mgr) = self.manager.as_mut() {
                mgr.cancel_fomod(&w.slug);
            }
            self.status = "FOMOD install cancelled.".into();
        }
    }
}

fn human_size(bytes: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn plugin_type_str(k: PluginType) -> &'static str {
    match k {
        PluginType::Required => "required",
        PluginType::Recommended => "recommended",
        PluginType::Optional => "optional",
        PluginType::NotUsable => "not usable",
        PluginType::CouldBeUsable => "could be usable",
    }
}

fn group_kind_str(k: GroupKind) -> &'static str {
    match k {
        GroupKind::ExactlyOne => "select one",
        GroupKind::AtMostOne => "at most one",
        GroupKind::AtLeastOne => "at least one",
        GroupKind::Any => "any",
        GroupKind::All => "all",
    }
}

/// Push current state into the UI.
fn refresh(ui: &MainWindow, app: &App) {
    let game_names: Vec<SharedString> = app.games.iter().map(|g| g.spec.name.into()).collect();
    ui.set_games(ModelRc::new(VecModel::from(game_names)));
    ui.set_active_game(app.current.unwrap_or(0) as i32);
    ui.set_status(app.status.as_str().into());
    ui.set_api_key(app.api_key.as_str().into());
    ui.set_nexus_status(app.nexus_status.as_str().into());

    // Conflicts overlay.
    let crows: Vec<ConflictRow> = app
        .conflicts
        .iter()
        .map(|c| ConflictRow {
            path: c.rel_path.as_str().into(),
            winner: c.winner.as_str().into(),
            providers: c.providers.join(", ").into(),
        })
        .collect();
    ui.set_conflicts(ModelRc::new(VecModel::from(crows)));
    ui.set_conflicts_open(app.conflicts_open);

    // Nexus browse overlay.
    ui.set_browse_open(app.browse_open);
    ui.set_browse_title(app.browse_title.as_str().into());
    ui.set_browse_status(app.browse_status.as_str().into());
    let platforms: Vec<SharedString> = Source::labels().iter().map(|s| (*s).into()).collect();
    ui.set_browse_platforms(ModelRc::new(VecModel::from(platforms)));
    ui.set_browse_platform(app.browse_platform as i32);
    ui.set_browse_game_id(app.browse_game().as_str().into());
    ui.set_browse_needs_key(app.source() == Source::Modio);
    let bmods: Vec<BrowseMod> = app
        .browse_mods
        .iter()
        .map(|m| {
            let summary = if m.downloads > 0 {
                format!("{}  ·  {} downloads", m.summary, m.downloads)
            } else {
                m.summary.clone()
            };
            BrowseMod {
                id: m.id.as_str().into(),
                name: m.name.as_str().into(),
                author: m.author.as_str().into(),
                summary: summary.into(),
            }
        })
        .collect();
    ui.set_browse_mods(ModelRc::new(VecModel::from(bmods)));
    let bfiles: Vec<BrowseFile> = app
        .browse_files
        .iter()
        .map(|f| BrowseFile {
            id: f.id.as_str().into(),
            name: f.name.as_str().into(),
            info: format!("v{}  ·  {}", f.version, human_size(f.size)).into(),
        })
        .collect();
    ui.set_browse_files(ModelRc::new(VecModel::from(bfiles)));

    // FOMOD wizard overlay.
    if let Some(w) = &app.wizard {
        ui.set_fomod_active(true);
        ui.set_fomod_module(w.config.module_name.as_str().into());
        ui.set_fomod_step_count(w.config.steps.len() as i32);
        ui.set_fomod_step_index(w.step as i32);
        if let Some(step) = w.config.steps.get(w.step) {
            ui.set_fomod_step_name(step.name.as_str().into());
            let groups: Vec<FomodGroup> = step
                .groups
                .iter()
                .enumerate()
                .map(|(gi, g)| {
                    let plugins: Vec<FomodPlugin> = g
                        .plugins
                        .iter()
                        .enumerate()
                        .map(|(pi, p)| FomodPlugin {
                            name: p.name.as_str().into(),
                            description: p.description.as_str().into(),
                            selected: w
                                .selections
                                .get(w.step)
                                .and_then(|s| s.get(gi))
                                .and_then(|v| v.get(pi))
                                .copied()
                                .unwrap_or(false),
                            kind: plugin_type_str(p.effective_type(&[], std::path::Path::new("")))
                                .into(),
                        })
                        .collect();
                    FomodGroup {
                        name: g.name.as_str().into(),
                        kind: group_kind_str(g.kind).into(),
                        plugins: ModelRc::new(VecModel::from(plugins)),
                    }
                })
                .collect();
            ui.set_fomod_groups(ModelRc::new(VecModel::from(groups)));

            // Preview pane for the focused plugin (image + description).
            let mut desc = String::new();
            let mut image = slint::Image::default();
            if let Some((gi, pi)) = w.sel {
                if let Some(p) = step.groups.get(gi).and_then(|g| g.plugins.get(pi)) {
                    desc = p.description.clone();
                    if let Some(rel) = &p.image {
                        let path = w.src_root.join(rel.replace('\\', "/"));
                        if let Ok(img) = slint::Image::load_from_path(&path) {
                            image = img;
                        }
                    }
                }
            }
            ui.set_fomod_desc(desc.as_str().into());
            ui.set_fomod_image(image);
        }
    } else {
        ui.set_fomod_active(false);
        ui.set_fomod_groups(ModelRc::new(VecModel::<FomodGroup>::default()));
    }

    if let Some(mgr) = &app.manager {
        ui.set_game_path(mgr.game().path.display().to_string().into());

        let names = mgr.profile_names();
        let active = names
            .iter()
            .position(|n| n == &mgr.active_profile().name)
            .unwrap_or(0);
        let profiles: Vec<SharedString> = names.iter().map(|s| s.as_str().into()).collect();
        ui.set_profiles(ModelRc::new(VecModel::from(profiles)));
        ui.set_active_profile(active as i32);

        let prof = mgr.active_profile();
        let rows: Vec<ModRow> = prof
            .order
            .iter()
            .map(|e| {
                let rec = mgr.mods().iter().find(|m| m.slug == e.slug);
                let name = rec
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| e.slug.clone());
                let size = rec.map(|m| human_size(m.size_bytes)).unwrap_or_default();
                ModRow {
                    slug: e.slug.as_str().into(),
                    name: name.into(),
                    enabled: e.enabled,
                    size: size.into(),
                }
            })
            .collect();
        ui.set_mods(ModelRc::new(VecModel::from(rows)));
        ui.set_deployed(mgr.is_deployed());
        ui.set_deploy_method(match mgr.deploy_method() {
            LinkMethod::Symlink => 0,
            LinkMethod::Hardlink => 1,
        });
    } else {
        ui.set_game_path("—".into());
        ui.set_profiles(ModelRc::new(VecModel::<SharedString>::default()));
        ui.set_mods(ModelRc::new(VecModel::<ModRow>::default()));
        ui.set_deployed(false);
    }
}

/// Worker: validate an API key, report the account.
fn spawn_validate(tx: Sender<Bg>, key: String) {
    std::thread::spawn(move || {
        let msg = match NexusClient::new(key).and_then(|c| c.validate()) {
            Ok(u) => format!(
                "Signed in as {} ({}).",
                u.name,
                if u.is_premium { "premium" } else { "free" }
            ),
            Err(e) => format!("Validation failed: {e}"),
        };
        let _ = tx.send(Bg::NexusStatus(msg));
    });
}

/// Worker: resolve an nxm link, download the archive into `cache`.
fn spawn_nxm_download(tx: Sender<Bg>, key: String, cache: PathBuf, link_str: String) {
    std::thread::spawn(move || {
        let link = match NxmLink::parse(&link_str) {
            Ok(l) => l,
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("Bad nxm link: {e}")));
                return;
            }
        };
        let client = match NexusClient::new(key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("{e}")));
                return;
            }
        };
        let dl = match client.resolve_nxm(&link) {
            Ok(d) => d,
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("Resolve failed: {e}")));
                return;
            }
        };
        let filename = filename_from_uri(&dl.uri)
            .unwrap_or_else(|| format!("{}-{}.archive", link.mod_id, link.file_id));
        let dest = cache.join(&filename);
        let _ = tx.send(Bg::NexusStatus(format!("Downloading {filename}…")));
        let tx2 = tx.clone();
        let mut last = 0u64;
        let result = client.download_to(&dl.uri, &dest, |done, total| {
            // Throttle progress updates to ~1 per MiB.
            if done.saturating_sub(last) >= 1 << 20 {
                last = done;
                let pct = total
                    .map(|t| format!(" ({}%)", done * 100 / t.max(1)))
                    .unwrap_or_default();
                let _ = tx2.send(Bg::NexusStatus(format!("Downloading {filename}{pct}…")));
            }
        });
        match result {
            Ok(_) => {
                let _ = tx.send(Bg::Downloaded {
                    path: dest,
                    switch: Some(link.domain),
                    nexus: Some((link.mod_id, link.file_id, String::new())),
                });
            }
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("Download failed: {e}")));
            }
        }
    });
}

/// Worker: check installed Nexus mods for newer files.
fn spawn_update_check(tx: Sender<Bg>, key: String, mods: Vec<(String, NexusRef)>) {
    std::thread::spawn(move || {
        let client = match NexusClient::new(key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("{e}")));
                return;
            }
        };
        let mut outdated = Vec::new();
        for (slug, nx) in &mods {
            if let Ok(files) = client.files(&nx.domain, nx.mod_id) {
                let latest = files.iter().map(|f| f.file_id).max().unwrap_or(0);
                if latest > nx.file_id {
                    outdated.push(slug.clone());
                }
            }
        }
        let msg = if outdated.is_empty() {
            format!("All {} Nexus mod(s) up to date.", mods.len())
        } else {
            format!(
                "{} update(s) available: {}",
                outdated.len(),
                outdated.join(", ")
            )
        };
        let _ = tx.send(Bg::NexusStatus(msg));
    });
}

fn sort_to_modlist(sort: ListSort) -> ModList {
    match sort {
        ListSort::Top => ModList::Trending,
        ListSort::Newest => ModList::LatestAdded,
        ListSort::Updated => ModList::LatestUpdated,
    }
}

// ---- Nexus browse workers (emit source-agnostic results) ----

fn spawn_nexus_list(tx: Sender<Bg>, key: String, domain: String, sort: ListSort) {
    std::thread::spawn(move || {
        match NexusClient::new(key).and_then(|c| c.mod_list(&domain, sort_to_modlist(sort))) {
            Ok(mods) => {
                let entries = mods
                    .into_iter()
                    .map(|m| BrowseEntry {
                        id: m.mod_id.to_string(),
                        name: m.name,
                        author: m.author,
                        summary: m.summary,
                        downloads: 0,
                    })
                    .collect();
                let _ = tx.send(Bg::BrowseList(entries));
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("List failed: {e}")));
            }
        }
    });
}

fn spawn_nexus_files(tx: Sender<Bg>, key: String, domain: String, mod_id: u64) {
    std::thread::spawn(move || {
        let client = match NexusClient::new(key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("{e}")));
                return;
            }
        };
        let name = client
            .mod_info(&domain, mod_id)
            .map(|m| m.name)
            .unwrap_or_default();
        match client.files(&domain, mod_id) {
            Ok(files) => {
                let files = files
                    .into_iter()
                    .map(|f| BrowseFileEntry {
                        id: f.file_id.to_string(),
                        name: f.name,
                        version: f.version,
                        size: f.size_kb * 1024,
                        url: None, // Nexus: resolved at download time (premium).
                    })
                    .collect();
                let _ = tx.send(Bg::BrowseFileList { title: name, files });
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("Files failed: {e}")));
            }
        }
    });
}

fn spawn_nexus_download(
    tx: Sender<Bg>,
    key: String,
    cache: PathBuf,
    domain: String,
    mod_id: u64,
    file_id: u64,
) {
    std::thread::spawn(move || {
        let client = match NexusClient::new(key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("{e}")));
                return;
            }
        };
        let dl = match client.download_links(&domain, mod_id, file_id, None) {
            Ok(mut links) if !links.is_empty() => links.remove(0),
            Ok(_) => {
                let _ = tx.send(Bg::BrowseMsg(
                    "No links — free Nexus accounts must use the site's Mod Manager Download button."
                        .into(),
                ));
                return;
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!(
                    "In-app download needs Nexus Premium (or use the nxm button): {e}"
                )));
                return;
            }
        };
        let filename =
            filename_from_uri(&dl.uri).unwrap_or_else(|| format!("{mod_id}-{file_id}.archive"));
        let dest = cache.join(&filename);
        let _ = tx.send(Bg::BrowseMsg(format!("Downloading {filename}…")));
        match client.download_to(&dl.uri, &dest, |_, _| {}) {
            Ok(_) => {
                let _ = tx.send(Bg::Downloaded {
                    path: dest,
                    switch: Some(domain),
                    nexus: Some((mod_id, file_id, String::new())),
                });
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("Download failed: {e}")));
            }
        }
    });
}

// ---- Free-platform browse workers (Thunderstore / mod.io / GameBanana) ----

fn spawn_pl_list(tx: Sender<Bg>, src: Source, modio_key: String, game: String, sort: ListSort) {
    std::thread::spawn(move || {
        let pl = match make_platform(src, &modio_key) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(e));
                return;
            }
        };
        match pl.list(&game, sort) {
            Ok(mods) => {
                let entries = mods
                    .into_iter()
                    .map(|m| BrowseEntry {
                        id: m.id,
                        name: m.name,
                        author: m.author,
                        summary: m.summary,
                        downloads: m.downloads,
                    })
                    .collect();
                let _ = tx.send(Bg::BrowseList(entries));
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("List failed: {e}")));
            }
        }
    });
}

fn spawn_pl_files(tx: Sender<Bg>, src: Source, modio_key: String, game: String, mod_id: String) {
    std::thread::spawn(move || {
        let pl = match make_platform(src, &modio_key) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(e));
                return;
            }
        };
        match pl.files(&game, &mod_id) {
            Ok(files) => {
                let files = files
                    .into_iter()
                    .map(|f| BrowseFileEntry {
                        id: f.id,
                        name: f.name,
                        version: f.version,
                        size: f.size,
                        url: Some(f.url),
                    })
                    .collect();
                let _ = tx.send(Bg::BrowseFileList {
                    title: mod_id,
                    files,
                });
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("Files failed: {e}")));
            }
        }
    });
}

fn spawn_pl_download(tx: Sender<Bg>, src: Source, modio_key: String, cache: PathBuf, url: String) {
    std::thread::spawn(move || {
        let pl = match make_platform(src, &modio_key) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(e));
                return;
            }
        };
        let filename = filename_from_uri(&url).unwrap_or_else(|| "download.zip".into());
        let dest = cache.join(&filename);
        let _ = tx.send(Bg::BrowseMsg(format!("Downloading {filename}…")));
        match pl.download(&url, &dest, &mut |_, _| {}) {
            Ok(_) => {
                let _ = tx.send(Bg::Downloaded {
                    path: dest,
                    switch: None,
                    nexus: None,
                });
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("Download failed: {e}")));
            }
        }
    });
}

fn filename_from_uri(uri: &str) -> Option<String> {
    let no_query = uri.split('?').next()?;
    let name = no_query.rsplit('/').next()?;
    if name.is_empty() {
        return None;
    }
    // URL-decode %20 etc minimally.
    Some(name.replace("%20", " "))
}

/// Apply one background message on the UI thread.
fn handle_bg(app: &Rc<RefCell<App>>, msg: Bg) {
    let mut a = app.borrow_mut();
    match msg {
        Bg::NexusStatus(s) => a.nexus_status = s,
        Bg::Downloaded {
            path,
            switch,
            nexus,
        } => {
            // Nexus downloads may target another game; switch to it. Platform
            // downloads install into the currently-open game.
            if let Some(domain) = &switch {
                if !a.open_game_by_domain(domain) {
                    a.browse_status = format!("Downloaded, but '{domain}' is not detected.");
                    return;
                }
            }
            if a.manager.is_none() {
                a.browse_status = "Downloaded, but no game is open to install into.".into();
                return;
            }
            let outcome = a.manager.as_mut().map(|mgr| mgr.install_archive(&path));
            match outcome {
                Some(Ok(InstallOutcome::Installed(rec))) => {
                    if let (Some((mod_id, file_id, version)), Some(domain), Some(mgr)) =
                        (nexus, switch.as_ref(), a.manager.as_mut())
                    {
                        let _ = mgr.set_nexus_ref(
                            &rec.slug,
                            NexusRef {
                                domain: domain.clone(),
                                mod_id,
                                file_id,
                                version,
                            },
                        );
                    }
                    a.browse_status = format!("Installed '{}'.", rec.name);
                }
                Some(Ok(InstallOutcome::NeedsFomod {
                    slug,
                    name,
                    config,
                    src_root,
                })) => {
                    a.browse_status = format!("Configure '{name}' (FOMOD).");
                    a.start_wizard(slug, *config, src_root);
                }
                Some(Err(e)) => a.browse_status = format!("Install failed: {e}"),
                None => {}
            }
            let _ = std::fs::remove_file(&path);
        }
        Bg::BrowseList(v) => {
            a.browse_status = format!("{} mods.", v.len());
            a.browse_mods = v;
        }
        Bg::BrowseFileList { title, files } => {
            a.browse_title = format!("Browse — {title}");
            a.browse_status = format!("{} file(s).", files.len());
            a.browse_files = files;
        }
        Bg::BrowseMsg(s) => a.browse_status = s,
    }
}

fn main() -> Result<(), slint::PlatformError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    // `modeman --register-nxm`: install the nxm:// protocol handler, then exit.
    if args.iter().any(|a| a == "--register-nxm") {
        match std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|exe| {
                modeman_nexus::install_protocol_handler(&exe).map_err(|e| e.to_string())
            }) {
            Ok(path) => println!("Registered nxm:// handler at {}", path.display()),
            Err(e) => eprintln!("Failed to register handler: {e}"),
        }
        return Ok(());
    }

    // Optional CLI: `modeman --nxm nxm://...` (protocol handler entry point).
    let cli_nxm: Option<String> = args
        .iter()
        .position(|a| a == "--nxm")
        .and_then(|i| args.get(i + 1).cloned());

    let ui = MainWindow::new()?;
    let (tx, rx): (Sender<Bg>, Receiver<Bg>) = std::sync::mpsc::channel();
    let app = Rc::new(RefCell::new(App::new(tx.clone())));
    app.borrow_mut().rescan();
    refresh(&ui, &app.borrow());

    // Drain worker results on the UI thread.
    let timer = slint::Timer::default();
    {
        let app = app.clone();
        let weak = ui.as_weak();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(200),
            move || {
                let Some(ui) = weak.upgrade() else { return };
                let mut got = false;
                while let Ok(msg) = rx.try_recv() {
                    handle_bg(&app, msg);
                    got = true;
                }
                if got {
                    refresh(&ui, &app.borrow());
                }
            },
        );
    }

    // Helper to run a mutation then refresh.
    macro_rules! on {
        ($cb:ident, |$a:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            let app = app.clone();
            let weak = ui.as_weak();
            ui.$cb(move |$($arg : $ty),*| {
                let ui = weak.unwrap();
                {
                    let mut $a = app.borrow_mut();
                    $body
                }
                refresh(&ui, &app.borrow());
            });
        }};
    }

    on!(on_rescan, |a| {
        a.rescan();
    });

    on!(on_select_game, |a, idx: i32| {
        a.open_game(idx.max(0) as usize);
    });

    on!(on_select_profile, |a, idx: i32| {
        let names = a
            .manager
            .as_ref()
            .map(|m| m.profile_names())
            .unwrap_or_default();
        if let (Some(name), Some(mgr)) = (names.get(idx.max(0) as usize), a.manager.as_mut()) {
            if let Err(e) = mgr.set_active_profile(name) {
                a.status = format!("{e}");
            }
        }
    });

    on!(on_new_profile, |a, name: SharedString| {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        if let Some(mgr) = a.manager.as_mut() {
            match mgr
                .create_profile(&name)
                .and_then(|_| mgr.set_active_profile(&name))
            {
                Ok(_) => a.status = format!("Created profile '{name}'."),
                Err(e) => a.status = format!("{e}"),
            }
        }
    });

    on!(on_install, |a| {
        let picked = rfd::FileDialog::new()
            .add_filter(
                "Mod archives",
                &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst"],
            )
            .set_title("Select mod archive")
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        let outcome = match a.manager.as_mut() {
            Some(mgr) => mgr.install_archive(&path),
            None => return,
        };
        match outcome {
            Ok(InstallOutcome::Installed(rec)) => a.status = format!("Installed '{}'.", rec.name),
            Ok(InstallOutcome::NeedsFomod {
                slug,
                name,
                config,
                src_root,
            }) => {
                a.status = format!("Configure '{name}' (FOMOD).");
                a.start_wizard(slug, *config, src_root);
            }
            Err(e) => a.status = format!("Install failed: {e}"),
        }
    });

    on!(on_remove, |a, idx: i32| {
        let slug = a.manager.as_ref().and_then(|m| {
            m.active_profile()
                .order
                .get(idx.max(0) as usize)
                .map(|e| e.slug.clone())
        });
        if let (Some(slug), Some(mgr)) = (slug, a.manager.as_mut()) {
            match mgr.remove_mod(&slug) {
                Ok(_) => a.status = format!("Removed '{slug}'."),
                Err(e) => a.status = format!("{e}"),
            }
        }
    });

    on!(on_toggle, |a, idx: i32, checked: bool| {
        let slug = a.manager.as_ref().and_then(|m| {
            m.active_profile()
                .order
                .get(idx.max(0) as usize)
                .map(|e| e.slug.clone())
        });
        if let (Some(slug), Some(mgr)) = (slug, a.manager.as_mut()) {
            if let Err(e) = mgr.set_enabled(&slug, checked) {
                a.status = format!("{e}");
            }
        }
    });

    on!(on_move_up, |a, idx: i32| {
        let i = idx.max(0) as usize;
        if let Some(mgr) = a.manager.as_mut() {
            if i > 0 {
                let _ = mgr.move_mod(i, i - 1);
            }
        }
    });

    on!(on_move_down, |a, idx: i32| {
        let i = idx.max(0) as usize;
        if let Some(mgr) = a.manager.as_mut() {
            let _ = mgr.move_mod(i, i + 1);
        }
    });

    on!(on_deploy, |a| {
        if let Some(mgr) = a.manager.as_mut() {
            match mgr.deploy() {
                Ok(_) => a.status = "Deployed active profile.".into(),
                Err(e) => a.status = format!("Deploy failed: {e}"),
            }
        }
    });

    on!(on_clear_deploy, |a| {
        if let Some(mgr) = a.manager.as_mut() {
            match mgr.clear() {
                Ok(_) => a.status = "Cleared deployment.".into(),
                Err(e) => a.status = format!("Clear failed: {e}"),
            }
        }
    });

    on!(on_set_deploy_method, |a, idx: i32| {
        let method = if idx == 1 {
            LinkMethod::Hardlink
        } else {
            LinkMethod::Symlink
        };
        if let Some(mgr) = a.manager.as_mut() {
            match mgr.set_deploy_method(method) {
                Ok(_) => {
                    a.status = format!(
                        "Deploy method: {}",
                        if idx == 1 { "hardlink" } else { "symlink" }
                    )
                }
                Err(e) => a.status = format!("Could not change method: {e}"),
            }
        }
    });

    on!(on_check_conflicts, |a| {
        a.check_conflicts();
    });
    on!(on_close_conflicts, |a| {
        a.conflicts_open = false;
    });

    on!(on_browse_open_cb, |a| {
        a.browse_open = true;
        a.trigger_list(ListSort::Top);
    });
    on!(on_check_updates, |a| {
        match (a.nexus_domain().is_some(), a.api_key.is_empty()) {
            (_, true) => a.nexus_status = "Set a Nexus API key first.".into(),
            _ => {
                let mods = a
                    .manager
                    .as_ref()
                    .map(|m| m.nexus_mods())
                    .unwrap_or_default();
                if mods.is_empty() {
                    a.nexus_status = "No Nexus-sourced mods to check.".into();
                } else {
                    a.nexus_status = "Checking for updates…".into();
                    spawn_update_check(a.tx.clone(), a.api_key.clone(), mods);
                }
            }
        }
    });

    on!(on_browse_close, |a| {
        a.browse_open = false;
    });
    on!(on_browse_list, |a, kind: i32| {
        let sort = match kind {
            1 => ListSort::Newest,
            2 => ListSort::Updated,
            _ => ListSort::Top,
        };
        a.trigger_list(sort);
    });
    on!(on_browse_mod, |a, id: SharedString| {
        a.trigger_files(id.to_string());
    });
    on!(on_browse_set_platform, |a, idx: i32| {
        a.browse_platform = idx.max(0) as usize;
        a.browse_mods.clear();
        a.browse_files.clear();
        a.browse_status = format!("Source: {}", Source::labels()[a.browse_platform.min(3)]);
    });
    on!(on_browse_set_game_id, |a, t: SharedString| {
        a.browse_game_id = t.trim().to_string();
    });
    on!(on_browse_set_key, |a, t: SharedString| {
        a.save_modio_key(&t);
    });
    on!(on_browse_download, |a, file_id: SharedString| {
        let fid = file_id.to_string();
        let entry = a.browse_files.iter().find(|f| f.id == fid).cloned();
        let Some(entry) = entry else { return };
        let cache = a.cache_dir();
        match a.source() {
            Source::Nexus => {
                let domain = a.nexus_domain();
                let mod_id = a
                    .browse_sel_mod
                    .as_ref()
                    .and_then(|s| s.parse::<u64>().ok());
                let fid_u = fid.parse::<u64>().ok();
                if let (Some(domain), Some(mod_id), Some(fid_u), false) =
                    (domain, mod_id, fid_u, a.api_key.is_empty())
                {
                    a.browse_status = "Resolving download…".into();
                    spawn_nexus_download(
                        a.tx.clone(),
                        a.api_key.clone(),
                        cache,
                        domain,
                        mod_id,
                        fid_u,
                    );
                } else {
                    a.browse_status = "Pick a mod first (and set API key).".into();
                }
            }
            src => {
                if let Some(url) = entry.url {
                    a.browse_status = "Downloading…".into();
                    spawn_pl_download(a.tx.clone(), src, a.modio_key.clone(), cache, url);
                }
            }
        }
    });

    on!(on_fomod_toggle, |a, gi: i32, pi: i32, checked: bool| {
        a.fomod_toggle(gi.max(0) as usize, pi.max(0) as usize, checked);
    });
    on!(on_fomod_select, |a, gi: i32, pi: i32| {
        a.fomod_select(gi.max(0) as usize, pi.max(0) as usize);
    });
    on!(on_fomod_next, |a| {
        a.fomod_step(1);
    });
    on!(on_fomod_back, |a| {
        a.fomod_step(-1);
    });
    on!(on_fomod_install, |a| {
        a.fomod_install();
    });
    on!(on_fomod_cancel, |a| {
        a.fomod_cancel();
    });

    on!(on_save_key, |a, key: SharedString| {
        a.save_key(&key);
    });

    on!(on_validate_key, |a, key: SharedString| {
        a.save_key(&key);
        a.nexus_status = "Validating…".into();
        spawn_validate(a.tx.clone(), a.api_key.clone());
    });

    on!(on_nxm_install, |a, link: SharedString| {
        let link = link.trim().to_string();
        if link.is_empty() {
            return;
        }
        if a.api_key.is_empty() {
            a.nexus_status = "Set a Nexus API key first.".into();
            return;
        }
        let cache = a.cache_dir();
        a.nexus_status = "Starting download…".into();
        spawn_nxm_download(a.tx.clone(), a.api_key.clone(), cache, link);
    });

    // Kick off a CLI-provided nxm download once the UI is up.
    if let Some(link) = cli_nxm {
        let a = app.borrow();
        if a.api_key.is_empty() {
            // can't download without a key; surface in UI
            drop(a);
            app.borrow_mut().nexus_status = "nxm link received, but no API key set.".into();
            refresh(&ui, &app.borrow());
        } else {
            let cache = a.cache_dir();
            let key = a.api_key.clone();
            let tx = a.tx.clone();
            drop(a);
            spawn_nxm_download(tx, key, cache, link);
        }
    }

    let _timer = timer; // keep alive for the app's lifetime
    ui.run()
}
