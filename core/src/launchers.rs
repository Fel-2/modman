//! Non-Steam launcher detection: Heroic (GOG / Epic / sideloaded) and Lutris.
//!
//! These launchers install games outside Steam libraries, so the Steam
//! `compatdata` prefix layout doesn't apply — each detection also resolves the
//! game's Wine prefix (Heroic `GamesConfig`, Lutris game YAML) and carries it
//! on [`InstalledGame::prefix`] so load-order files land in the right place.
//!
//! Everything here parses defensively: launcher config formats drift between
//! versions, so unknown shapes are skipped, never errors.

use crate::game::{spec_by_id, GameSpec, InstalledGame, CATALOG};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Detect games installed via Heroic and Lutris.
pub fn detect_all() -> Vec<InstalledGame> {
    let mut out = Vec::new();
    for root in heroic_roots() {
        detect_heroic_at(&root, &mut out);
    }
    for (data, config) in lutris_roots() {
        detect_lutris_at(&data, &config, &mut out);
    }
    out
}

/// Candidate Heroic config roots (native + Flatpak).
fn heroic_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        home.join(".config/heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ]
    .into_iter()
    .filter(|p| p.is_dir())
    .collect()
}

/// Candidate Lutris `(data_dir, config_dir)` pairs (native + Flatpak).
fn lutris_roots() -> Vec<(PathBuf, PathBuf)> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        (
            home.join(".local/share/lutris"),
            home.join(".config/lutris"),
        ),
        (
            home.join(".var/app/net.lutris.Lutris/data/lutris"),
            home.join(".var/app/net.lutris.Lutris/config/lutris"),
        ),
    ]
    .into_iter()
    .filter(|(d, _)| d.is_dir())
    .collect()
}

// ---- Heroic ----------------------------------------------------------------

/// Scan one Heroic config root: GOG installs, Epic (legendary) installs, and
/// sideloaded games.
pub fn detect_heroic_at(root: &Path, out: &mut Vec<InstalledGame>) {
    // GOG: install list + a separate title library.
    let titles = heroic_gog_titles(root);
    if let Some(installed) = read_json(&root.join("gog_store/installed.json")) {
        for entry in installed["installed"].as_array().into_iter().flatten() {
            let Some(path) = entry["install_path"].as_str() else {
                continue;
            };
            let app = entry["appName"].as_str().unwrap_or_default();
            let title = titles
                .iter()
                .find(|(a, _)| a == app)
                .map(|(_, t)| t.clone())
                .or_else(|| dir_name(path))
                .unwrap_or_default();
            push_match(out, &title, path, heroic_prefix(root, app));
        }
    }

    // Epic via bundled legendary: `{ "<app>": { title, install_path } }`.
    if let Some(installed) = read_json(&root.join("legendaryConfig/legendary/installed.json")) {
        for (app, entry) in installed.as_object().into_iter().flatten() {
            let (Some(title), Some(path)) =
                (entry["title"].as_str(), entry["install_path"].as_str())
            else {
                continue;
            };
            push_match(out, title, path, heroic_prefix(root, app));
        }
    }

    // Sideloaded: `{ "games": [ { app_name, title, install: { executable } } ] }`.
    if let Some(lib) = read_json(&root.join("sideload_apps/library.json")) {
        for entry in lib["games"].as_array().into_iter().flatten() {
            let Some(title) = entry["title"].as_str() else {
                continue;
            };
            let Some(dir) = entry["install"]["executable"]
                .as_str()
                .and_then(|e| Path::new(e).parent())
            else {
                continue;
            };
            let app = entry["app_name"].as_str().unwrap_or_default();
            push_match(out, title, dir, heroic_prefix(root, app));
        }
    }
}

/// GOG `appName → title` pairs from whichever library file this Heroic
/// version writes.
fn heroic_gog_titles(root: &Path) -> Vec<(String, String)> {
    let mut titles = Vec::new();
    for rel in ["gog_store/library.json", "store_cache/gog_library.json"] {
        let Some(lib) = read_json(&root.join(rel)) else {
            continue;
        };
        for g in lib["games"].as_array().into_iter().flatten() {
            if let (Some(app), Some(title)) = (g["app_name"].as_str(), g["title"].as_str()) {
                titles.push((app.to_string(), title.to_string()));
            }
        }
    }
    titles
}

/// The game's Wine prefix from Heroic's per-game config, if recorded.
fn heroic_prefix(root: &Path, app: &str) -> Option<PathBuf> {
    if app.is_empty() {
        return None;
    }
    let cfg = read_json(&root.join(format!("GamesConfig/{app}.json")))?;
    let prefix = cfg[app]["winePrefix"].as_str()?;
    Some(PathBuf::from(prefix))
}

// ---- Lutris ----------------------------------------------------------------

/// Scan one Lutris install: `pga.db` for the game list, per-game YAML for the
/// Wine prefix (and install dir fallback).
pub fn detect_lutris_at(data_dir: &Path, config_dir: &Path, out: &mut Vec<InstalledGame>) {
    let db_path = data_dir.join("pga.db");
    let Ok(conn) =
        rusqlite::Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
    else {
        return;
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT name, directory, configpath FROM games WHERE installed = 1")
    else {
        return;
    };
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    });
    for row in rows.into_iter().flatten().flatten() {
        let (name, directory, configpath) = row;
        let yml = configpath
            .filter(|c| !c.is_empty())
            .map(|c| config_dir.join("games").join(format!("{c}.yml")))
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let prefix = yaml_value(&yml, "prefix").map(PathBuf::from);
        // `directory` is often empty for Wine games; fall back to the exe dir.
        let dir = directory
            .filter(|d| !d.is_empty())
            .or_else(|| {
                yaml_value(&yml, "exe")
                    .and_then(|e| Path::new(&e).parent().map(|p| p.display().to_string()))
            })
            .unwrap_or_default();
        if !dir.is_empty() {
            push_match(out, &name, &dir, prefix);
        }
    }
}

/// First `key: value` occurrence in a simple Lutris YAML (line scan — the
/// files are flat enough that a YAML dependency isn't warranted).
fn yaml_value(yml: &str, key: &str) -> Option<String> {
    let needle = format!("{key}:");
    yml.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(&needle))
        .map(|v| v.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

// ---- shared ----------------------------------------------------------------

/// Match a store title against the catalog and record the install if valid.
fn push_match(
    out: &mut Vec<InstalledGame>,
    title: &str,
    dir: impl AsRef<Path>,
    prefix: Option<PathBuf>,
) {
    let Some(spec) = spec_for_title(title) else {
        return;
    };
    let path = dir.as_ref().to_path_buf();
    if !path.is_dir() || out.iter().any(|g| g.path == path) {
        return;
    }
    out.push(InstalledGame { spec, path, prefix });
}

/// Resolve a store/launcher display title to a catalog game.
pub fn spec_for_title(title: &str) -> Option<&'static GameSpec> {
    let norm = normalize(title);
    if let Some(spec) = CATALOG.iter().find(|s| normalize(s.name) == norm) {
        return Some(spec);
    }
    // Store titles vary; map the common variants (normalized, exact).
    let id = match norm.as_str() {
        "theelderscrollsvskyrimspecialedition"
        | "theelderscrollsvskyrimanniversaryedition"
        | "skyrimanniversaryedition" => "skyrimse",
        "theelderscrollsvskyrim" | "theelderscrollsvskyrimlegendaryedition" => "skyrim",
        "theelderscrollsivoblivion" | "theelderscrollsivoblivriongoty" => "oblivion",
        "theelderscrollsiiimorrowind" | "theelderscrollsiiimorrowindgotyedition" => "morrowind",
        "fallout4gameoftheyearedition" => "fallout4",
        "falloutnewvegas" | "falloutnewvegasultimateedition" => "falloutnv",
        "fallout3gameoftheyearedition" => "fallout3",
        "crusaderkings3" => "ck3",
        "crusaderkings2" => "ck2",
        _ => return None,
    };
    spec_by_id(id)
}

/// Lowercase, alphanumeric-only form for title comparison.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn dir_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("modeman-ln-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn titles_match_catalog() {
        assert_eq!(spec_for_title("Cyberpunk 2077").unwrap().id, "cyberpunk");
        assert_eq!(
            spec_for_title("The Elder Scrolls V: Skyrim Special Edition")
                .unwrap()
                .id,
            "skyrimse"
        );
        assert_eq!(
            spec_for_title("Fallout: New Vegas").unwrap().id,
            "falloutnv"
        );
        assert_eq!(spec_for_title("Stardew Valley").unwrap().id, "stardew");
        assert!(spec_for_title("Some Unknown Game").is_none());
    }

    #[test]
    fn detects_heroic_gog_install_with_prefix() {
        let root = tmp("heroic");
        let game_dir = tmp("heroic-game");
        let prefix = tmp("heroic-prefix");

        fs::create_dir_all(root.join("gog_store")).unwrap();
        fs::write(
            root.join("gog_store/installed.json"),
            format!(
                r#"{{"installed":[{{"appName":"1423049311","platform":"windows","install_path":"{}"}}]}}"#,
                game_dir.display()
            ),
        )
        .unwrap();
        fs::write(
            root.join("gog_store/library.json"),
            r#"{"games":[{"app_name":"1423049311","title":"Cyberpunk 2077"}]}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("GamesConfig")).unwrap();
        fs::write(
            root.join("GamesConfig/1423049311.json"),
            format!(
                r#"{{"1423049311":{{"winePrefix":"{}"}},"version":"v0"}}"#,
                prefix.display()
            ),
        )
        .unwrap();

        let mut out = Vec::new();
        detect_heroic_at(&root, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].spec.id, "cyberpunk");
        assert_eq!(out[0].path, game_dir);
        assert_eq!(out[0].prefix.as_deref(), Some(prefix.as_path()));
    }

    #[test]
    fn detects_lutris_install_from_pga_db() {
        let data = tmp("lutris-data");
        let config = tmp("lutris-config");
        let game_dir = tmp("lutris-game");
        let prefix = tmp("lutris-prefix");

        let conn = rusqlite::Connection::open(data.join("pga.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, slug TEXT,
             directory TEXT, installed INTEGER, configpath TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO games (name, slug, directory, installed, configpath)
             VALUES (?1, ?2, ?3, 1, ?4)",
            rusqlite::params![
                "The Elder Scrolls V: Skyrim Special Edition",
                "skyrim-se",
                game_dir.display().to_string(),
                "skyrim-se-1"
            ],
        )
        .unwrap();
        drop(conn);

        fs::create_dir_all(config.join("games")).unwrap();
        fs::write(
            config.join("games/skyrim-se-1.yml"),
            format!(
                "game:\n  exe: {}/SkyrimSELauncher.exe\n  prefix: {}\n",
                game_dir.display(),
                prefix.display()
            ),
        )
        .unwrap();

        let mut out = Vec::new();
        detect_lutris_at(&data, &config, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].spec.id, "skyrimse");
        assert_eq!(out[0].path, game_dir);
        assert_eq!(out[0].prefix.as_deref(), Some(prefix.as_path()));
    }

    #[test]
    fn prefix_override_resolves_appdata_with_gog_variant() {
        let game_dir = tmp("pfx-game");
        let prefix = tmp("pfx-root");
        // GOG-suffixed appdata folder under a plain-Wine user dir.
        let local = prefix.join("drive_c/users/steamuser/AppData/Local");
        fs::create_dir_all(local.join("Skyrim Special Edition GOG")).unwrap();

        let g = crate::game::InstalledGame {
            spec: spec_by_id("skyrimse").unwrap(),
            path: game_dir,
            prefix: Some(prefix),
        };
        let appdata = g.prefix_appdata().unwrap();
        assert!(appdata.ends_with("Skyrim Special Edition GOG"));
    }
}
