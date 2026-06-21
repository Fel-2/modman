//! FOMOD scripted-installer support (`fomod/ModuleConfig.xml`).
//!
//! Many Bethesda mods ship a wizard: stepped pages of option groups whose
//! selections decide which files land in `Data/`, optionally gated by flags.
//! This module parses that config, evaluates the conditions, and copies the
//! chosen files out of the extracted archive into a deploy-ready tree.
//!
//! Source paths in the XML use Windows conventions (backslashes, arbitrary
//! case) that won't match a case-sensitive Linux extraction, so every source
//! is resolved case-insensitively against the real files on disk.

use crate::error::{Error, Result};
use roxmltree::{Document, Node};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Selection state: `[step][group][plugin] = chosen`.
pub type Selections = Vec<Vec<Vec<bool>>>;

#[derive(Debug, Clone)]
pub struct FomodConfig {
    pub module_name: String,
    pub required: Vec<FileItem>,
    pub steps: Vec<InstallStep>,
    pub conditional: Vec<ConditionalInstall>,
}

#[derive(Debug, Clone)]
pub struct FileItem {
    pub source: String,
    pub destination: String,
    pub is_folder: bool,
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct InstallStep {
    pub name: String,
    pub visible: Option<Composite>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub kind: GroupKind,
    pub plugins: Vec<Plugin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    ExactlyOne,
    AtMostOne,
    AtLeastOne,
    Any,
    All,
}

#[derive(Debug, Clone)]
pub struct Plugin {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub files: Vec<FileItem>,
    /// Flags set when this plugin is selected.
    pub flags: Vec<(String, String)>,
    /// Type used when no conditional pattern matches.
    pub default_type: PluginType,
    /// Conditional types: first pattern whose dependencies hold wins.
    pub type_patterns: Vec<(Composite, PluginType)>,
}

impl Plugin {
    /// The plugin's effective type given the current flags (and optional
    /// in-progress install dir for file dependencies).
    pub fn effective_type(&self, flags: &[(String, String)], dest: &std::path::Path) -> PluginType {
        for (deps, ty) in &self.type_patterns {
            if eval(deps, flags, dest) {
                return *ty;
            }
        }
        self.default_type
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginType {
    Required,
    Optional,
    Recommended,
    NotUsable,
    CouldBeUsable,
}

#[derive(Debug, Clone)]
pub struct ConditionalInstall {
    pub deps: Composite,
    pub files: Vec<FileItem>,
}

/// A composite dependency (`And`/`Or` of leaf deps and nested composites).
#[derive(Debug, Clone)]
pub struct Composite {
    pub or: bool,
    pub deps: Vec<Dependency>,
    pub nested: Vec<Composite>,
}

#[derive(Debug, Clone)]
pub enum Dependency {
    Flag { name: String, value: String },
    /// File state dependency: `Active` / `Inactive` / `Missing`.
    File { file: String, state: String },
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Locate a `fomod/ModuleConfig.xml` under `root` (case-insensitive). Returns
/// `(config_path, source_root)` where `source_root` is the dir holding the
/// `fomod/` folder — file sources are relative to it.
pub fn find_config(root: &Path) -> Option<(PathBuf, PathBuf)> {
    for entry in WalkDir::new(root).max_depth(4).into_iter().flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("fomod")
        {
            if let Some(cfg) = read_dir_ci(entry.path(), "moduleconfig.xml") {
                let source_root = entry.path().parent()?.to_path_buf();
                return Some((cfg, source_root));
            }
        }
    }
    None
}

fn read_dir_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
            .then(|| e.path())
    })
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

pub fn parse(config_path: &Path) -> Result<FomodConfig> {
    let text = std::fs::read_to_string(config_path).map_err(|e| Error::io(config_path, e))?;
    let doc = Document::parse(&text).map_err(|e| Error::Other(format!("fomod xml: {e}")))?;
    let root = doc.root_element();

    let module_name = child(&root, "moduleName")
        .and_then(|n| n.text())
        .unwrap_or("FOMOD")
        .trim()
        .to_string();

    let required = child(&root, "requiredInstallFiles")
        .map(|n| parse_files(&n))
        .unwrap_or_default();

    let mut steps = Vec::new();
    if let Some(install_steps) = child(&root, "installSteps") {
        for step in children(&install_steps, "installStep") {
            steps.push(parse_step(&step));
        }
    }

    let mut conditional = Vec::new();
    if let Some(cfi) = child(&root, "conditionalFileInstalls") {
        if let Some(patterns) = child(&cfi, "patterns") {
            for pat in children(&patterns, "pattern") {
                let deps = child(&pat, "dependencies")
                    .map(|d| parse_composite(&d))
                    .unwrap_or(Composite { or: false, deps: vec![], nested: vec![] });
                let files = child(&pat, "files").map(|f| parse_files(&f)).unwrap_or_default();
                conditional.push(ConditionalInstall { deps, files });
            }
        }
    }

    Ok(FomodConfig { module_name, required, steps, conditional })
}

fn parse_step(step: &Node) -> InstallStep {
    let name = step.attribute("name").unwrap_or("").to_string();
    let visible = child(step, "visible").map(|v| parse_composite(&v));
    let mut groups = Vec::new();
    if let Some(g) = child(step, "optionalFileGroups") {
        for grp in children(&g, "group") {
            groups.push(parse_group(&grp));
        }
    }
    InstallStep { name, visible, groups }
}

fn parse_group(grp: &Node) -> Group {
    let name = grp.attribute("name").unwrap_or("").to_string();
    let kind = match grp.attribute("type").unwrap_or("SelectAny") {
        "SelectExactlyOne" => GroupKind::ExactlyOne,
        "SelectAtMostOne" => GroupKind::AtMostOne,
        "SelectAtLeastOne" => GroupKind::AtLeastOne,
        "SelectAll" => GroupKind::All,
        _ => GroupKind::Any,
    };
    let mut plugins = Vec::new();
    if let Some(ps) = child(grp, "plugins") {
        for p in children(&ps, "plugin") {
            plugins.push(parse_plugin(&p));
        }
    }
    Group { name, kind, plugins }
}

fn parse_plugin(p: &Node) -> Plugin {
    let name = p.attribute("name").unwrap_or("").to_string();
    let description = child(p, "description")
        .and_then(|d| d.text())
        .unwrap_or("")
        .trim()
        .to_string();
    let image = child(p, "image").and_then(|i| i.attribute("path")).map(String::from);
    let files = child(p, "files").map(|f| parse_files(&f)).unwrap_or_default();

    let mut flags = Vec::new();
    if let Some(cf) = child(p, "conditionFlags") {
        for flag in children(&cf, "flag") {
            let fname = flag.attribute("name").unwrap_or("").to_string();
            let val = flag.text().unwrap_or("").trim().to_string();
            flags.push((fname, val));
        }
    }

    let (default_type, type_patterns) = parse_plugin_type(p);
    Plugin { name, description, image, files, flags, default_type, type_patterns }
}

fn type_from_name(name: &str) -> PluginType {
    match name {
        "Required" => PluginType::Required,
        "Recommended" => PluginType::Recommended,
        "NotUsable" => PluginType::NotUsable,
        "CouldBeUsable" => PluginType::CouldBeUsable,
        _ => PluginType::Optional,
    }
}

/// Parse a `<typeDescriptor>`: either a fixed `<type>` or a
/// `<dependencyType>` with a default plus conditional `<pattern>`s.
fn parse_plugin_type(p: &Node) -> (PluginType, Vec<(Composite, PluginType)>) {
    let Some(td) = child(p, "typeDescriptor") else {
        return (PluginType::Optional, Vec::new());
    };
    if let Some(t) = child(&td, "type") {
        return (type_from_name(t.attribute("name").unwrap_or("Optional")), Vec::new());
    }
    let Some(dt) = child(&td, "dependencyType") else {
        return (PluginType::Optional, Vec::new());
    };
    let default_type = type_from_name(
        child(&dt, "defaultType")
            .and_then(|t| t.attribute("name"))
            .unwrap_or("Optional"),
    );
    let mut patterns = Vec::new();
    if let Some(ps) = child(&dt, "patterns") {
        for pat in children(&ps, "pattern") {
            let deps = child(&pat, "dependencies")
                .map(|d| parse_composite(&d))
                .unwrap_or(Composite { or: false, deps: vec![], nested: vec![] });
            let ty = type_from_name(
                child(&pat, "type").and_then(|t| t.attribute("name")).unwrap_or("Optional"),
            );
            patterns.push((deps, ty));
        }
    }
    (default_type, patterns)
}

fn parse_files(node: &Node) -> Vec<FileItem> {
    let mut out = Vec::new();
    for child in node.children().filter(|c| c.is_element()) {
        let is_folder = match child.tag_name().name() {
            "file" => false,
            "folder" => true,
            _ => continue,
        };
        let source = child.attribute("source").unwrap_or("").to_string();
        let destination = child.attribute("destination").unwrap_or("").to_string();
        let priority = child
            .attribute("priority")
            .and_then(|p| p.parse().ok())
            .unwrap_or(0);
        out.push(FileItem { source, destination, is_folder, priority });
    }
    out
}

fn parse_composite(node: &Node) -> Composite {
    let or = node.attribute("operator").map(|o| o.eq_ignore_ascii_case("or")).unwrap_or(false);
    let mut deps = Vec::new();
    let mut nested = Vec::new();
    for c in node.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "flagDependency" => {
                deps.push(Dependency::Flag {
                    name: c.attribute("flag").unwrap_or("").to_string(),
                    value: c.attribute("value").unwrap_or("").to_string(),
                });
            }
            "fileDependency" => {
                deps.push(Dependency::File {
                    file: c.attribute("file").unwrap_or("").to_string(),
                    state: c.attribute("state").unwrap_or("Active").to_string(),
                });
            }
            "dependencies" => nested.push(parse_composite(&c)),
            _ => {}
        }
    }
    Composite { or, deps, nested }
}

// ---------------------------------------------------------------------------
// Selection helpers + evaluation
// ---------------------------------------------------------------------------

impl FomodConfig {
    /// Default selections: Required/Recommended plugins on, plus the first
    /// plumbing for `ExactlyOne`/`AtLeastOne`/`All` groups so the result is
    /// already valid.
    pub fn default_selections(&self) -> Selections {
        self.steps
            .iter()
            .map(|step| {
                step.groups
                    .iter()
                    .map(|g| {
                        let no_dest = std::path::Path::new("");
                        let mut sel: Vec<bool> = g
                            .plugins
                            .iter()
                            .map(|p| {
                                matches!(
                                    p.effective_type(&[], no_dest),
                                    PluginType::Required | PluginType::Recommended
                                )
                            })
                            .collect();
                        match g.kind {
                            GroupKind::All => sel.iter_mut().for_each(|s| *s = true),
                            GroupKind::ExactlyOne | GroupKind::AtLeastOne => {
                                if !sel.iter().any(|s| *s) {
                                    if let Some(first) = sel.first_mut() {
                                        *first = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                        sel
                    })
                    .collect()
            })
            .collect()
    }

    /// Validate a selection against each group's cardinality rule.
    pub fn validate(&self, sel: &Selections) -> Result<()> {
        for (si, step) in self.steps.iter().enumerate() {
            for (gi, g) in step.groups.iter().enumerate() {
                let n = sel
                    .get(si)
                    .and_then(|s| s.get(gi))
                    .map(|v| v.iter().filter(|x| **x).count())
                    .unwrap_or(0);
                let ok = match g.kind {
                    GroupKind::ExactlyOne => n == 1,
                    GroupKind::AtMostOne => n <= 1,
                    GroupKind::AtLeastOne => n >= 1,
                    GroupKind::All => n == g.plugins.len(),
                    GroupKind::Any => true,
                };
                if !ok {
                    return Err(Error::Other(format!(
                        "group '{}' needs {:?}, got {n} selected",
                        g.name, g.kind
                    )));
                }
            }
        }
        Ok(())
    }
}

/// A configured FOMOD ready to install.
pub struct FomodSession {
    pub config: FomodConfig,
    src_root: PathBuf,
}

impl FomodSession {
    pub fn load(config_path: &Path, src_root: PathBuf) -> Result<Self> {
        Ok(FomodSession { config: parse(config_path)?, src_root })
    }

    /// Run the install with the given selections, writing into `dest`.
    pub fn install(&self, sel: &Selections, dest: &Path) -> Result<()> {
        self.config.validate(sel)?;
        std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;

        let mut items: Vec<FileItem> = self.config.required.clone();
        let mut flags: Vec<(String, String)> = Vec::new();

        // Walk steps in order, honoring visibility, accumulating flags + files.
        for (si, step) in self.config.steps.iter().enumerate() {
            if let Some(vis) = &step.visible {
                if !eval(vis, &flags, dest) {
                    continue;
                }
            }
            for (gi, g) in step.groups.iter().enumerate() {
                for (pi, plugin) in g.plugins.iter().enumerate() {
                    let chosen = sel
                        .get(si)
                        .and_then(|s| s.get(gi))
                        .and_then(|v| v.get(pi))
                        .copied()
                        .unwrap_or(false);
                    if chosen {
                        flags.extend(plugin.flags.iter().cloned());
                        items.extend(plugin.files.iter().cloned());
                    }
                }
            }
        }

        // Conditional installs evaluated with the final flag set.
        for cond in &self.config.conditional {
            if eval(&cond.deps, &flags, dest) {
                items.extend(cond.files.iter().cloned());
            }
        }

        items.sort_by_key(|i| i.priority);
        for item in &items {
            self.copy_item(item, dest)?;
        }
        Ok(())
    }

    fn copy_item(&self, item: &FileItem, dest_root: &Path) -> Result<()> {
        let Some(src) = resolve_ci(&self.src_root, &item.source) else {
            tracing::warn!("fomod source not found: {}", item.source);
            return Ok(());
        };
        let dest_rel = normalize_rel(&item.destination);

        if item.is_folder || src.is_dir() {
            let base = dest_root.join(&dest_rel);
            for entry in WalkDir::new(&src).into_iter().flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                let rel = entry.path().strip_prefix(&src).unwrap();
                let target = base.join(rel);
                copy_file(entry.path(), &target)?;
            }
        } else {
            // File: if destination is empty use the source filename, else
            // write to the named destination path.
            let target = if item.destination.is_empty() {
                dest_root.join(src.file_name().unwrap_or_default())
            } else {
                dest_root.join(&dest_rel)
            };
            copy_file(&src, &target)?;
        }
        Ok(())
    }
}

/// Evaluate a composite dependency against current flags. File deps are
/// approximated by checking the in-progress destination tree.
fn eval(c: &Composite, flags: &[(String, String)], dest: &Path) -> bool {
    let mut results = Vec::new();
    for d in &c.deps {
        results.push(match d {
            Dependency::Flag { name, value } => flags
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v == value)
                .unwrap_or(value.is_empty()),
            Dependency::File { file, state } => {
                let exists = resolve_ci(dest, file).is_some();
                match state.as_str() {
                    "Missing" => !exists,
                    _ => exists, // Active / Inactive both imply present here
                }
            }
        });
    }
    for n in &c.nested {
        results.push(eval(n, flags, dest));
    }
    if results.is_empty() {
        return true;
    }
    if c.or {
        results.into_iter().any(|x| x)
    } else {
        results.into_iter().all(|x| x)
    }
}

// ---------------------------------------------------------------------------
// Path / copy utilities
// ---------------------------------------------------------------------------

/// Resolve a Windows-style relative path under `root`, matching each path
/// component case-insensitively against real entries on disk.
fn resolve_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for comp in rel.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".") {
        let mut next = None;
        if let Ok(rd) = std::fs::read_dir(&cur) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().eq_ignore_ascii_case(comp) {
                    next = Some(e.path());
                    break;
                }
            }
        }
        cur = next?;
    }
    Some(cur)
}

/// Turn a Windows-ish relative path into a clean `PathBuf`.
fn normalize_rel(s: &str) -> PathBuf {
    let mut p = PathBuf::new();
    for comp in s.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".") {
        p.push(comp);
    }
    p
}

fn copy_file(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::copy(src, dst).map_err(|e| Error::io(dst, e))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// node helpers
// ---------------------------------------------------------------------------

fn child<'a>(node: &Node<'a, 'a>, tag: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|c| c.is_element() && c.tag_name().name() == tag)
}

fn children<'a>(node: &'a Node<'a, 'a>, tag: &'a str) -> impl Iterator<Item = Node<'a, 'a>> {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == tag)
}
