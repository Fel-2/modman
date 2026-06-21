//! Paradox launcher-v2 playset database integration (experimental).
//!
//! Modern Paradox launchers keep mod activation in `launcher-v2.sqlite`
//! (playsets + mods + a join table). `dlc_load.json` ([`crate::loadorder::paradox`])
//! still works, but the launcher may regenerate it from this DB, so we also
//! reflect the active set here.
//!
//! Conservative on purpose: the schema differs across launcher versions, so we
//! only toggle `enabled` for mods the launcher already knows (matched by
//! `gameRegistryId`) in the *active* playset — never inserting playsets or mods
//! and never guessing NOT NULL columns. The DB is backed up before writing.

use crate::error::{Error, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::path::Path;

/// `(id, name)` of the active playset, if any.
pub fn active_playset(conn: &Connection) -> rusqlite::Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT id, name FROM playsets WHERE isActive = 1 LIMIT 1",
        [],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// In the active playset, set `enabled` for each managed mod the launcher
/// already knows (by `gameRegistryId`). Returns rows updated.
pub fn apply(
    conn: &Connection,
    enabled: &BTreeSet<String>,
    managed: &[String],
) -> rusqlite::Result<usize> {
    let Some((playset_id, _)) = active_playset(conn)? else {
        return Ok(0);
    };
    let mut updated = 0;
    for reg in managed {
        let mod_id: Option<String> = conn
            .query_row(
                "SELECT id FROM mods WHERE gameRegistryId = ?1 LIMIT 1",
                [reg],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        if let Some(mod_id) = mod_id {
            let on = enabled.contains(reg);
            updated += conn.execute(
                "UPDATE playsets_mods SET enabled = ?1 WHERE playsetId = ?2 AND modId = ?3",
                rusqlite::params![on as i64, playset_id, mod_id],
            )?;
        }
    }
    Ok(updated)
}

/// Open `db_path`, back it up once, and apply the active set. No-op (Ok(false))
/// if the DB does not exist. Returns whether any row changed.
pub fn sync_file(db_path: &Path, enabled: &[String], managed: &[String]) -> Result<bool> {
    if !db_path.is_file() {
        return Ok(false);
    }
    let backup = db_path.with_extension("sqlite.modeman-bak");
    if !backup.exists() {
        std::fs::copy(db_path, &backup).map_err(|e| Error::io(&backup, e))?;
    }
    let conn = Connection::open(db_path).map_err(|e| Error::other(e.to_string()))?;
    let set: BTreeSet<String> = enabled.iter().cloned().collect();
    let n = apply(&conn, &set, managed).map_err(|e| Error::other(e.to_string()))?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE playsets(id TEXT PRIMARY KEY, name TEXT, isActive INTEGER);
             CREATE TABLE mods(id TEXT PRIMARY KEY, gameRegistryId TEXT);
             CREATE TABLE playsets_mods(playsetId TEXT, modId TEXT, enabled INTEGER);
             INSERT INTO playsets VALUES('ps1','modeman',1);
             INSERT INTO mods VALUES('m1','mod/alpha.mod');
             INSERT INTO mods VALUES('m2','mod/beta.mod');
             INSERT INTO playsets_mods VALUES('ps1','m1',0);
             INSERT INTO playsets_mods VALUES('ps1','m2',1);",
        )
        .unwrap();
        c
    }

    #[test]
    fn toggles_enabled_in_active_playset() {
        let conn = fixture();
        assert_eq!(active_playset(&conn).unwrap().unwrap().1, "modeman");

        // Enable alpha, disable beta.
        let enabled: BTreeSet<String> = ["mod/alpha.mod".to_string()].into_iter().collect();
        let managed = vec!["mod/alpha.mod".to_string(), "mod/beta.mod".to_string()];
        let n = apply(&conn, &enabled, &managed).unwrap();
        assert_eq!(n, 2);

        let alpha: i64 = conn
            .query_row("SELECT enabled FROM playsets_mods WHERE modId='m1'", [], |r| r.get(0))
            .unwrap();
        let beta: i64 = conn
            .query_row("SELECT enabled FROM playsets_mods WHERE modId='m2'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(alpha, 1);
        assert_eq!(beta, 0);
    }
}
