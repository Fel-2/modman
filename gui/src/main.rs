// modeman — Linux-first game mod manager (GUI).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

use modeman_core::deploy::LinkMethod;
use modeman_core::fomod::{FomodConfig, GroupKind, PluginType, Selections};
use modeman_core::manager::InstallOutcome;
use modeman_core::{game, FileConflict, Manager};
use modeman_nexus::{ModFile, ModInfo, ModList, NexusClient, NxmLink};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

/// Messages from background worker threads to the UI loop.
enum Bg {
    NexusStatus(String),
    Downloaded { path: PathBuf, domain: String },
    BrowseMods(Vec<ModInfo>),
    BrowseFiles { mod_name: String, files: Vec<ModFile> },
    BrowseMsg(String),
}

/// A staged FOMOD install being configured in the wizard.
struct Wizard {
    slug: String,
    config: FomodConfig,
    selections: Selections,
    step: usize,
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
    browse_mods: Vec<ModInfo>,
    browse_files: Vec<ModFile>,
    browse_title: String,
    browse_status: String,
    browse_sel_mod: Option<u64>,
}

impl App {
    fn new(tx: Sender<Bg>) -> Self {
        let data_root = Manager::default_data_root()
            .unwrap_or_else(|_| PathBuf::from("./modeman-data"));
        let api_key = std::fs::read_to_string(data_root.join("nexus-apikey.txt"))
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
        self.conflicts = self.manager.as_ref().map(|m| m.conflicts()).unwrap_or_default();
        self.conflicts_open = true;
        self.status = format!("{} conflicting file(s).", self.conflicts.len());
    }

    /// Begin a FOMOD wizard for a freshly staged install.
    fn start_wizard(&mut self, slug: String, config: FomodConfig) {
        let selections = config.default_selections();
        self.wizard = Some(Wizard { slug, config, selections, step: 0 });
    }

    /// Apply a wizard checkbox toggle, enforcing the group's cardinality.
    fn fomod_toggle(&mut self, gi: usize, pi: usize, checked: bool) {
        let Some(w) = self.wizard.as_mut() else { return };
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
    let game_names: Vec<SharedString> =
        app.games.iter().map(|g| g.spec.name.into()).collect();
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
    let bmods: Vec<BrowseMod> = app
        .browse_mods
        .iter()
        .map(|m| BrowseMod {
            id: m.mod_id as i32,
            name: m.name.as_str().into(),
            author: m.author.as_str().into(),
            summary: m.summary.as_str().into(),
        })
        .collect();
    ui.set_browse_mods(ModelRc::new(VecModel::from(bmods)));
    let bfiles: Vec<BrowseFile> = app
        .browse_files
        .iter()
        .map(|f| BrowseFile {
            id: f.file_id as i32,
            name: f.name.as_str().into(),
            info: format!(
                "v{}  ·  {} MB  ·  {}",
                f.version,
                f.size_kb / 1024,
                f.category_name.clone().unwrap_or_default()
            )
            .into(),
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
                            kind: plugin_type_str(p.effective_type(&[], std::path::Path::new(""))).into(),
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
                let name = mgr
                    .mods()
                    .iter()
                    .find(|m| m.slug == e.slug)
                    .map(|m| m.name.clone())
                    .unwrap_or_else(|| e.slug.clone());
                ModRow {
                    slug: e.slug.as_str().into(),
                    name: name.into(),
                    enabled: e.enabled,
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
                let _ = tx2.send(Bg::NexusStatus(format!(
                    "Downloading {filename}{pct}…"
                )));
            }
        });
        match result {
            Ok(_) => {
                let _ = tx.send(Bg::Downloaded { path: dest, domain: link.domain });
            }
            Err(e) => {
                let _ = tx.send(Bg::NexusStatus(format!("Download failed: {e}")));
            }
        }
    });
}

/// Worker: fetch a curated mod list for a game.
fn spawn_browse_list(tx: Sender<Bg>, key: String, domain: String, kind: ModList) {
    std::thread::spawn(move || {
        match NexusClient::new(key).and_then(|c| c.mod_list(&domain, kind)) {
            Ok(mods) => {
                let _ = tx.send(Bg::BrowseMsg(format!("{} mods.", mods.len())));
                let _ = tx.send(Bg::BrowseMods(mods));
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("List failed: {e}")));
            }
        }
    });
}

/// Worker: fetch a mod's name + downloadable files.
fn spawn_browse_files(tx: Sender<Bg>, key: String, domain: String, mod_id: u64) {
    std::thread::spawn(move || {
        let client = match NexusClient::new(key) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("{e}")));
                return;
            }
        };
        let name = client.mod_info(&domain, mod_id).map(|m| m.name).unwrap_or_default();
        match client.files(&domain, mod_id) {
            Ok(files) => {
                let _ = tx.send(Bg::BrowseFiles { mod_name: name, files });
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!("Files failed: {e}")));
            }
        }
    });
}

/// Worker: download a browsed file (premium accounts only via the API).
fn spawn_browse_download(
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
                    "No links — free accounts must use the site's Mod Manager Download button.".into(),
                ));
                return;
            }
            Err(e) => {
                let _ = tx.send(Bg::BrowseMsg(format!(
                    "Download needs Nexus Premium (or use the nxm button): {e}"
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
                let _ = tx.send(Bg::Downloaded { path: dest, domain });
                let _ = tx.send(Bg::BrowseMsg("Downloaded — installing…".into()));
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
        Bg::Downloaded { path, domain } => {
            if !a.open_game_by_domain(&domain) {
                a.nexus_status =
                    format!("Downloaded, but '{domain}' is not installed/detected.");
                return;
            }
            let outcome = a.manager.as_mut().map(|mgr| mgr.install_archive(&path));
            match outcome {
                Some(Ok(InstallOutcome::Installed(rec))) => {
                    a.nexus_status = format!("Installed '{}' from Nexus.", rec.name)
                }
                Some(Ok(InstallOutcome::NeedsFomod { slug, name, config })) => {
                    a.nexus_status = format!("Configure '{name}' (FOMOD).");
                    a.start_wizard(slug, *config);
                }
                Some(Err(e)) => a.nexus_status = format!("Install failed: {e}"),
                None => {}
            }
            let _ = std::fs::remove_file(&path);
        }
        Bg::BrowseMods(v) => a.browse_mods = v,
        Bg::BrowseFiles { mod_name, files } => {
            a.browse_title = format!("Browse — {mod_name}");
            a.browse_status = format!("{} file(s).", files.len());
            a.browse_files = files;
        }
        Bg::BrowseMsg(s) => a.browse_status = s,
    }
}

fn main() -> Result<(), slint::PlatformError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
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
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut got = false;
            while let Ok(msg) = rx.try_recv() {
                handle_bg(&app, msg);
                got = true;
            }
            if got {
                refresh(&ui, &app.borrow());
            }
        });
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

    on!(on_rescan, |a| { a.rescan(); });

    on!(on_select_game, |a, idx: i32| {
        a.open_game(idx.max(0) as usize);
    });

    on!(on_select_profile, |a, idx: i32| {
        let names = a.manager.as_ref().map(|m| m.profile_names()).unwrap_or_default();
        if let (Some(name), Some(mgr)) = (names.get(idx.max(0) as usize), a.manager.as_mut()) {
            if let Err(e) = mgr.set_active_profile(name) {
                a.status = format!("{e}");
            }
        }
    });

    on!(on_new_profile, |a, name: SharedString| {
        let name = name.trim().to_string();
        if name.is_empty() { return; }
        if let Some(mgr) = a.manager.as_mut() {
            match mgr.create_profile(&name).and_then(|_| mgr.set_active_profile(&name)) {
                Ok(_) => a.status = format!("Created profile '{name}'."),
                Err(e) => a.status = format!("{e}"),
            }
        }
    });

    on!(on_install, |a| {
        let picked = rfd::FileDialog::new()
            .add_filter("Mod archives", &["zip", "7z", "rar", "tar", "gz", "bz2", "xz", "zst"])
            .set_title("Select mod archive")
            .pick_file();
        let Some(path) = picked else { return; };
        let outcome = match a.manager.as_mut() {
            Some(mgr) => mgr.install_archive(&path),
            None => return,
        };
        match outcome {
            Ok(InstallOutcome::Installed(rec)) => a.status = format!("Installed '{}'.", rec.name),
            Ok(InstallOutcome::NeedsFomod { slug, name, config }) => {
                a.status = format!("Configure '{name}' (FOMOD).");
                a.start_wizard(slug, *config);
            }
            Err(e) => a.status = format!("Install failed: {e}"),
        }
    });

    on!(on_remove, |a, idx: i32| {
        let slug = a.manager.as_ref()
            .and_then(|m| m.active_profile().order.get(idx.max(0) as usize).map(|e| e.slug.clone()));
        if let (Some(slug), Some(mgr)) = (slug, a.manager.as_mut()) {
            match mgr.remove_mod(&slug) {
                Ok(_) => a.status = format!("Removed '{slug}'."),
                Err(e) => a.status = format!("{e}"),
            }
        }
    });

    on!(on_toggle, |a, idx: i32, checked: bool| {
        let slug = a.manager.as_ref()
            .and_then(|m| m.active_profile().order.get(idx.max(0) as usize).map(|e| e.slug.clone()));
        if let (Some(slug), Some(mgr)) = (slug, a.manager.as_mut()) {
            if let Err(e) = mgr.set_enabled(&slug, checked) {
                a.status = format!("{e}");
            }
        }
    });

    on!(on_move_up, |a, idx: i32| {
        let i = idx.max(0) as usize;
        if let Some(mgr) = a.manager.as_mut() {
            if i > 0 { let _ = mgr.move_mod(i, i - 1); }
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
        let method = if idx == 1 { LinkMethod::Hardlink } else { LinkMethod::Symlink };
        if let Some(mgr) = a.manager.as_mut() {
            match mgr.set_deploy_method(method) {
                Ok(_) => a.status = format!("Deploy method: {}", if idx == 1 { "hardlink" } else { "symlink" }),
                Err(e) => a.status = format!("Could not change method: {e}"),
            }
        }
    });

    on!(on_check_conflicts, |a| { a.check_conflicts(); });
    on!(on_close_conflicts, |a| { a.conflicts_open = false; });

    on!(on_browse_open_cb, |a| {
        a.browse_open = true;
        match (a.nexus_domain(), a.api_key.is_empty()) {
            (Some(domain), false) => {
                a.browse_title = format!("Browse — {domain}");
                a.browse_status = "Loading trending…".into();
                spawn_browse_list(a.tx.clone(), a.api_key.clone(), domain, ModList::Trending);
            }
            (None, _) => a.browse_status = "This game has no Nexus domain.".into(),
            (_, true) => a.browse_status = "Set a Nexus API key first.".into(),
        }
    });
    on!(on_browse_close, |a| { a.browse_open = false; });
    on!(on_browse_list, |a, kind: i32| {
        if let (Some(domain), false) = (a.nexus_domain(), a.api_key.is_empty()) {
            let k = match kind {
                1 => ModList::LatestAdded,
                2 => ModList::LatestUpdated,
                _ => ModList::Trending,
            };
            a.browse_status = "Loading…".into();
            spawn_browse_list(a.tx.clone(), a.api_key.clone(), domain, k);
        }
    });
    on!(on_browse_mod, |a, id: i32| {
        if let (Some(domain), false) = (a.nexus_domain(), a.api_key.is_empty()) {
            a.browse_sel_mod = Some(id as u64);
            a.browse_status = "Loading files…".into();
            spawn_browse_files(a.tx.clone(), a.api_key.clone(), domain, id as u64);
        }
    });
    on!(on_browse_download, |a, file_id: i32| {
        match (a.nexus_domain(), a.browse_sel_mod, a.api_key.is_empty()) {
            (Some(domain), Some(mod_id), false) => {
                let cache = a.cache_dir();
                a.browse_status = "Resolving download…".into();
                spawn_browse_download(a.tx.clone(), a.api_key.clone(), cache, domain, mod_id, file_id as u64);
            }
            _ => a.browse_status = "Pick a mod first (and set API key).".into(),
        }
    });

    on!(on_fomod_toggle, |a, gi: i32, pi: i32, checked: bool| {
        a.fomod_toggle(gi.max(0) as usize, pi.max(0) as usize, checked);
    });
    on!(on_fomod_next, |a| { a.fomod_step(1); });
    on!(on_fomod_back, |a| { a.fomod_step(-1); });
    on!(on_fomod_install, |a| { a.fomod_install(); });
    on!(on_fomod_cancel, |a| { a.fomod_cancel(); });

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
        if link.is_empty() { return; }
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
