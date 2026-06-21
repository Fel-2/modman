//! Conflict detection: two enabled mods providing the same file; the later
//! one in load order wins.

use modeman_core::manager::InstallOutcome;
use modeman_core::{game, Manager};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("modeman-conflict-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn tar_of(files: &[(&str, &[u8])], into: &Path) -> PathBuf {
    let root = into.with_extension("src");
    let _ = fs::remove_dir_all(&root);
    for (rel, data) in files {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, data).unwrap();
    }
    let ok = Command::new("tar")
        .arg("-cf")
        .arg(into)
        .arg("-C")
        .arg(&root)
        .arg(".")
        .status()
        .unwrap()
        .success();
    assert!(ok);
    into.to_path_buf()
}

fn install(mgr: &mut Manager, archive: &Path) -> String {
    match mgr.install_archive(archive).unwrap() {
        InstallOutcome::Installed(r) => r.slug,
        _ => panic!("unexpected FOMOD"),
    }
}

#[test]
fn detects_overwrite_winner() {
    let data_root = tmp("data");
    let lib = tmp("lib");
    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(&game_dir).unwrap();

    let a = tar_of(
        &[("Textures/shared.dds", b"from-a"), ("a-only.esp", b"a")],
        &tmp("a").join("aaa.tar"),
    );
    let b = tar_of(
        &[("Textures/shared.dds", b"from-b"), ("b-only.esp", b"b")],
        &tmp("b").join("bbb.tar"),
    );

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();

    let slug_a = install(&mut mgr, &a);
    let slug_b = install(&mut mgr, &b);

    // Both enabled; load order is install order (a before b) → b wins.
    let conflicts = mgr.conflicts();
    assert_eq!(conflicts.len(), 1, "exactly one shared file");
    let c = &conflicts[0];
    assert_eq!(c.rel_path, "Textures/shared.dds");
    assert_eq!(c.providers, vec![slug_a.clone(), slug_b.clone()]);
    assert_eq!(c.winner, slug_b, "later in load order wins");

    // Disabling b removes the conflict.
    mgr.set_enabled(&slug_b, false).unwrap();
    assert!(mgr.conflicts().is_empty());
}
