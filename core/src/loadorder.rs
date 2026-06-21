//! Per-engine load-order writers beyond Creation Engine `plugins.txt`
//! (which lives in [`crate::plugins`]).
//!
//! Each game records load order differently and in a different prefix file.
//! These writers rewrite only the entries modeman manages, preserving the
//! user's base game / DLC / hand-added entries.

pub mod rimworld {
    //! RimWorld `ModsConfig.xml`: ordered `<li>packageId</li>` under
    //! `<activeMods>`, in `AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon
    //! Studios/Config/`. Mods are identified by the `packageId` declared in
    //! `About/About.xml`.

    use crate::error::{Error, Result};
    use crate::game::InstalledGame;
    use roxmltree::Document;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    const CONFIG_SUB: &str = "Ludeon Studios/RimWorld by Ludeon Studios/Config";

    /// Read a mod's `packageId` from its `About/About.xml` (searched shallowly).
    /// Returned lowercased, as RimWorld treats packageIds case-insensitively.
    pub fn package_id(mod_dir: &Path) -> Option<String> {
        let about = find_about(mod_dir)?;
        let text = std::fs::read_to_string(&about).ok()?;
        let doc = Document::parse(&text).ok()?;
        let pid = doc
            .root_element()
            .children()
            .find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case("packageId"))
            .and_then(|n| n.text())?;
        Some(pid.trim().to_ascii_lowercase())
    }

    /// Human-readable mod name from `About/About.xml` `<name>`.
    pub fn mod_name(mod_dir: &Path) -> Option<String> {
        let about = find_about(mod_dir)?;
        let text = std::fs::read_to_string(&about).ok()?;
        let doc = Document::parse(&text).ok()?;
        let name = doc
            .root_element()
            .children()
            .find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case("name"))
            .and_then(|n| n.text())?;
        let t = name.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    fn find_about(mod_dir: &Path) -> Option<PathBuf> {
        for entry in WalkDir::new(mod_dir).max_depth(3).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry.file_name().to_string_lossy().eq_ignore_ascii_case("about.xml")
                && entry
                    .path()
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().eq_ignore_ascii_case("about"))
                    .unwrap_or(false)
            {
                return Some(entry.path().to_path_buf());
            }
        }
        None
    }

    /// Rewrite `<activeMods>` so managed mods reflect the deploy. `active` is the
    /// ordered list of enabled packageIds; `managed` is every packageId modeman
    /// controls (so stale ones are pruned). Non-managed entries (Core, DLC,
    /// hand-added) keep their position.
    pub fn write(game: &InstalledGame, active: &[String], managed: &[String]) -> Result<Option<PathBuf>> {
        let Some(dir) = game.prefix_locallow(CONFIG_SUB) else {
            return Ok(None);
        };
        let path = dir.join("ModsConfig.xml");
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;

        let backup = path.with_extension("xml.modeman-bak");
        if path.exists() && !backup.exists() {
            std::fs::copy(&path, &backup).map_err(|e| Error::io(&backup, e))?;
        }

        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let (version, existing_active, known) = parse(&existing);
        let managed_set: BTreeSet<String> = managed.iter().map(|s| s.to_ascii_lowercase()).collect();

        // Keep non-managed entries in place, then append managed active in order.
        let mut out: Vec<String> = existing_active
            .into_iter()
            .filter(|e| !managed_set.contains(&e.to_ascii_lowercase()))
            .collect();
        for pid in active {
            if !out.iter().any(|e| e.eq_ignore_ascii_case(pid)) {
                out.push(pid.clone());
            }
        }
        // Ensure Core is present and first.
        if !out.iter().any(|e| e.eq_ignore_ascii_case("ludeon.rimworld")) {
            out.insert(0, "ludeon.rimworld".to_string());
        }

        std::fs::write(&path, render(&version, &out, &known)).map_err(|e| Error::io(&path, e))?;
        Ok(Some(path))
    }

    /// Remove managed mods from the active list.
    pub fn clear(game: &InstalledGame, managed: &[String]) -> Result<()> {
        write(game, &[], managed)?;
        Ok(())
    }

    /// Extract `(version, activeMods, knownExpansions)` from an existing file.
    fn parse(xml: &str) -> (String, Vec<String>, Vec<String>) {
        let Ok(doc) = Document::parse(xml) else {
            return ("1.5".into(), Vec::new(), Vec::new());
        };
        let root = doc.root_element();
        let version = root
            .children()
            .find(|c| c.is_element() && c.tag_name().name() == "version")
            .and_then(|n| n.text())
            .unwrap_or("1.5")
            .trim()
            .to_string();
        let list = |tag: &str| -> Vec<String> {
            root.children()
                .find(|c| c.is_element() && c.tag_name().name() == tag)
                .map(|node| {
                    node.children()
                        .filter(|c| c.is_element() && c.tag_name().name() == "li")
                        .filter_map(|c| c.text().map(|t| t.trim().to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        (version, list("activeMods"), list("knownExpansions"))
    }

    fn render(version: &str, active: &[String], known: &[String]) -> String {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<ModsConfigData>\n");
        s.push_str(&format!("  <version>{version}</version>\n"));
        s.push_str("  <activeMods>\n");
        for pid in active {
            s.push_str(&format!("    <li>{pid}</li>\n"));
        }
        s.push_str("  </activeMods>\n");
        if !known.is_empty() {
            s.push_str("  <knownExpansions>\n");
            for k in known {
                s.push_str(&format!("    <li>{k}</li>\n"));
            }
            s.push_str("  </knownExpansions>\n");
        }
        s.push_str("</ModsConfigData>\n");
        s
    }
}

pub mod paradox {
    //! Paradox games (Crusader Kings): `dlc_load.json` in the game's Documents
    //! folder lists `enabled_mods` as `mod/<descriptor>.mod` paths, in load
    //! order. Each mod ships a `.mod` descriptor at the `mod/` root.
    //!
    //! Note: the modern launcher also keeps playsets in an SQLite DB and may
    //! override `dlc_load.json`; writing it is the portable approach and works
    //! when the launcher is not managing the set.

    use crate::error::{Error, Result};
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};

    /// `mod/<file>.mod` references for descriptors at a mod's root.
    pub fn descriptors(mod_dir: &Path) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(mod_dir) else {
            return Vec::new();
        };
        rd.flatten()
            .filter(|e| {
                e.path().is_file()
                    && e.path()
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|x| x.eq_ignore_ascii_case("mod"))
                        .unwrap_or(false)
            })
            .filter_map(|e| e.file_name().to_str().map(|n| format!("mod/{n}")))
            .collect()
    }

    /// Rewrite `enabled_mods` in `dlc_load.json` (in `json_dir`). Preserves
    /// `disabled_dlcs` and any non-managed enabled entries.
    pub fn write(json_dir: &Path, active: &[String], managed: &[String]) -> Result<PathBuf> {
        std::fs::create_dir_all(json_dir).map_err(|e| Error::io(json_dir, e))?;
        let path = json_dir.join("dlc_load.json");

        let backup = path.with_extension("json.modeman-bak");
        if path.exists() && !backup.exists() {
            std::fs::copy(&path, &backup).map_err(|e| Error::io(&backup, e))?;
        }

        let mut val: Value = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({ "enabled_mods": [], "disabled_dlcs": [] }));

        let managed_lower: Vec<String> = managed.iter().map(|s| s.to_ascii_lowercase()).collect();
        let existing: Vec<String> = val
            .get("enabled_mods")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let mut out: Vec<String> = existing
            .into_iter()
            .filter(|e| !managed_lower.contains(&e.to_ascii_lowercase()))
            .collect();
        for m in active {
            if !out.iter().any(|e| e.eq_ignore_ascii_case(m)) {
                out.push(m.clone());
            }
        }

        if !val.is_object() {
            val = json!({ "disabled_dlcs": [] });
        }
        val["enabled_mods"] = json!(out);
        if val.get("disabled_dlcs").is_none() {
            val["disabled_dlcs"] = json!([]);
        }

        let body = serde_json::to_string_pretty(&val)?;
        std::fs::write(&path, body).map_err(|e| Error::io(&path, e))?;
        Ok(path)
    }

    pub fn clear(json_dir: &Path, managed: &[String]) -> Result<()> {
        write(json_dir, &[], managed)?;
        Ok(())
    }
}

pub mod smapi {
    //! Stardew Valley / SMAPI: no load-order file (SMAPI resolves dependency
    //! order from each mod's `manifest.json`). This just reads the display name.

    use serde_json::Value;
    use std::path::Path;
    use walkdir::WalkDir;

    /// Mod display name from a shallow `manifest.json` `Name` field.
    pub fn manifest_name(mod_dir: &Path) -> Option<String> {
        for entry in WalkDir::new(mod_dir).max_depth(3).into_iter().flatten() {
            if entry.file_type().is_file()
                && entry.file_name().to_string_lossy().eq_ignore_ascii_case("manifest.json")
            {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if let Some(name) = v.get("Name").and_then(|n| n.as_str()) {
                            let t = name.trim();
                            if !t.is_empty() {
                                return Some(t.to_string());
                            }
                        }
                    }
                }
            }
        }
        None
    }
}
