//! End-to-end engine test: install an archive, deploy it as symlinks into a
//! fake Steam-layout game dir, activate its plugin, then revert. Uses the
//! system `tar` to build a test archive (libarchive reads tar; no writer dep).

use modeman_core::{game, Manager};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("modeman-test-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn make_tar(dir: &Path, archive: &Path) {
    let status = Command::new("tar")
        .arg("-cf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .arg(".")
        .status()
        .expect("run tar");
    assert!(status.success(), "tar failed");
}

#[test]
fn install_deploy_clear_roundtrip() {
    let data_root = tmp("data");
    let lib = tmp("lib");
    let mod_src = tmp("modsrc");

    // Fake Steam library layout so the Proton prefix path resolves.
    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(&game_dir).unwrap();
    let appdata = lib.join(
        "steamapps/compatdata/489830/pfx/drive_c/users/steamuser/AppData/Local/Skyrim Special Edition",
    );
    fs::create_dir_all(&appdata).unwrap();
    // Pre-existing user plugins.txt with a base master that must be preserved.
    let plugins_txt = appdata.join("plugins.txt");
    fs::write(&plugins_txt, "*Skyrim.esm\n").unwrap();

    // Mod tree: Textures/wall.dds, scripts/a.pex, CoolMod.esp
    fs::create_dir_all(mod_src.join("Textures")).unwrap();
    fs::create_dir_all(mod_src.join("scripts")).unwrap();
    fs::write(mod_src.join("Textures/wall.dds"), b"pixels").unwrap();
    fs::write(mod_src.join("scripts/a.pex"), b"bytecode").unwrap();
    fs::write(mod_src.join("CoolMod.esp"), b"TES4").unwrap();

    let archive = tmp("arc").join("coolmod.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();

    let rec = match mgr.install_archive(&archive).unwrap() {
        modeman_core::manager::InstallOutcome::Installed(r) => r,
        _ => panic!("plain archive should not need FOMOD"),
    };
    assert_eq!(rec.name, "coolmod");
    assert_eq!(mgr.mods().len(), 1);

    // Deploy → files symlinked under game/Data, plugin activated.
    mgr.deploy().unwrap();
    let deployed = game_dir.join("Data/Textures/wall.dds");
    assert!(deployed.is_symlink(), "deployed file should be a symlink");
    assert_eq!(fs::read(&deployed).unwrap(), b"pixels");
    assert!(game_dir.join("Data/CoolMod.esp").is_symlink());

    let txt = fs::read_to_string(&plugins_txt).unwrap();
    assert!(txt.contains("*Skyrim.esm"), "base master preserved");
    assert!(txt.contains("*CoolMod.esp"), "mod plugin activated");

    // Clear → game dir clean, plugin removed, base master kept.
    mgr.clear().unwrap();
    assert!(!deployed.exists(), "symlink should be removed");
    let txt = fs::read_to_string(&plugins_txt).unwrap();
    assert!(txt.contains("*Skyrim.esm"), "base master still preserved");
    assert!(!txt.contains("CoolMod.esp"), "mod plugin deactivated");

    // Disable then redeploy → nothing linked, plugin stays inactive.
    mgr.set_enabled(&rec.slug, false).unwrap();
    mgr.deploy().unwrap();
    assert!(!game_dir.join("Data/CoolMod.esp").exists());
    let txt = fs::read_to_string(&plugins_txt).unwrap();
    assert!(!txt.contains("CoolMod.esp"));
}

#[test]
fn deploy_preserves_and_restores_vanilla_files() {
    let data_root = tmp("data2");
    let lib = tmp("lib2");
    let mod_src = tmp("modsrc2");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    let vanilla = game_dir.join("Data/Textures/wall.dds");
    fs::create_dir_all(vanilla.parent().unwrap()).unwrap();
    fs::write(&vanilla, b"VANILLA").unwrap();

    // Mod ships a file at the same path as the vanilla loose file.
    fs::create_dir_all(mod_src.join("Textures")).unwrap();
    fs::write(mod_src.join("Textures/wall.dds"), b"MODDED").unwrap();
    let archive = tmp("arc2").join("retex.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    match mgr.install_archive(&archive).unwrap() {
        modeman_core::manager::InstallOutcome::Installed(_) => {}
        _ => panic!("plain archive"),
    }

    mgr.deploy().unwrap();
    // Mod content is live; vanilla preserved as a backup.
    assert_eq!(fs::read(&vanilla).unwrap(), b"MODDED");
    assert!(vanilla.is_symlink());
    assert!(game_dir
        .join("Data/Textures/wall.dds.modeman-orig")
        .exists());

    mgr.clear().unwrap();
    // Vanilla file is back, byte-for-byte; backup removed.
    assert!(!vanilla.is_symlink());
    assert_eq!(fs::read(&vanilla).unwrap(), b"VANILLA");
    assert!(!game_dir
        .join("Data/Textures/wall.dds.modeman-orig")
        .exists());
}

#[test]
fn folder_per_mod_preserves_wrapper() {
    // RimWorld: the archive's top folder IS the mod and must NOT be flattened.
    let data_root = tmp("data-rw");
    let lib = tmp("lib-rw");
    let mod_src = tmp("modsrc-rw");

    let game_dir = lib.join("steamapps/common/RimWorld");
    fs::create_dir_all(&game_dir).unwrap();
    let cfg_dir = lib.join("steamapps/compatdata/294100/pfx/drive_c/users/steamuser/AppData/LocalLow/Ludeon Studios/RimWorld by Ludeon Studios/Config");
    fs::create_dir_all(&cfg_dir).unwrap();

    // Archive root is a single wrapper folder with an About/About.xml packageId.
    fs::create_dir_all(mod_src.join("CoolMod/About")).unwrap();
    fs::write(
        mod_src.join("CoolMod/About/About.xml"),
        b"<?xml version=\"1.0\"?><ModMetaData><name>Cool Mod</name><packageId>Author.CoolMod</packageId></ModMetaData>",
    )
    .unwrap();
    let archive = tmp("arc-rw").join("coolmod.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("rimworld", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let _ = mgr.install_archive(&archive).unwrap();
    // Display name comes from About.xml <name>, not the archive filename.
    assert_eq!(mgr.mods()[0].name, "Cool Mod");
    mgr.deploy().unwrap();

    // Deployed as Mods/CoolMod/About/About.xml (wrapper preserved).
    let deployed = game_dir.join("Mods/CoolMod/About/About.xml");
    assert!(deployed.is_symlink(), "mod folder preserved under Mods/");

    // ModsConfig.xml lists Core + our packageId (lowercased) in order.
    let mods_config = cfg_dir.join("ModsConfig.xml");
    let xml = fs::read_to_string(&mods_config).unwrap();
    assert!(xml.contains("<li>ludeon.rimworld</li>"), "Core present");
    assert!(xml.contains("<li>author.coolmod</li>"), "mod activated");

    // Clear removes the managed packageId, keeps Core.
    mgr.clear().unwrap();
    let xml = fs::read_to_string(&mods_config).unwrap();
    assert!(xml.contains("<li>ludeon.rimworld</li>"));
    assert!(!xml.contains("author.coolmod"));
}

#[test]
fn stardew_uses_manifest_name() {
    let data_root = tmp("data-sdv");
    let lib = tmp("lib-sdv");
    let mod_src = tmp("modsrc-sdv");

    let game_dir = lib.join("steamapps/common/Stardew Valley");
    fs::create_dir_all(&game_dir).unwrap();

    fs::create_dir_all(mod_src.join("LookupAnything")).unwrap();
    fs::write(
        mod_src.join("LookupAnything/manifest.json"),
        br#"{ "Name": "Lookup Anything", "UniqueID": "Pathoschild.LookupAnything" }"#,
    )
    .unwrap();
    let archive = tmp("arc-sdv").join("lookup.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("stardew", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let _ = mgr.install_archive(&archive).unwrap();
    assert_eq!(mgr.mods()[0].name, "Lookup Anything");
    mgr.deploy().unwrap();
    assert!(game_dir
        .join("Mods/LookupAnything/manifest.json")
        .is_symlink());
}

#[test]
fn paradox_writes_dlc_load() {
    // Crusader Kings III: mods deploy to prefix Documents/.../mod and load order
    // is recorded in dlc_load.json next to it.
    let data_root = tmp("data-ck");
    let lib = tmp("lib-ck");
    let mod_src = tmp("modsrc-ck");

    let game_dir = lib.join("steamapps/common/CK3");
    fs::create_dir_all(&game_dir).unwrap();
    let docs = lib.join("steamapps/compatdata/1158310/pfx/drive_c/users/steamuser/Documents/Paradox Interactive/Crusader Kings III");
    fs::create_dir_all(&docs).unwrap();

    // Mod ships a descriptor + folder.
    fs::write(
        mod_src.join("mymod.mod"),
        b"name=\"My Mod\"\npath=\"mod/mymod\"",
    )
    .unwrap();
    fs::create_dir_all(mod_src.join("mymod")).unwrap();
    fs::write(mod_src.join("mymod/descriptor.mod"), b"name=\"My Mod\"").unwrap();
    let archive = tmp("arc-ck").join("mymod.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("ck3", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let _ = mgr.install_archive(&archive).unwrap();
    mgr.deploy().unwrap();

    // Descriptor deployed into mod/, dlc_load.json lists it.
    assert!(docs.join("mod/mymod.mod").is_symlink());
    let dlc = docs.join("dlc_load.json");
    let json = fs::read_to_string(&dlc).unwrap();
    assert!(json.contains("mod/mymod.mod"), "descriptor enabled");
    assert!(json.contains("disabled_dlcs"), "structure preserved");

    mgr.clear().unwrap();
    let json = fs::read_to_string(&dlc).unwrap();
    assert!(!json.contains("mymod.mod"), "descriptor removed on clear");
}

#[test]
fn overlay_method_deploys_without_touching_game_dir() {
    use modeman_core::deploy::LinkMethod;

    let data_root = tmp("data-ov");
    let lib = tmp("lib-ov");
    let mod_src = tmp("modsrc-ov");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(game_dir.join("Data")).unwrap();
    let appdata = lib.join(
        "steamapps/compatdata/489830/pfx/drive_c/users/steamuser/AppData/Local/Skyrim Special Edition",
    );
    fs::create_dir_all(&appdata).unwrap();
    fs::write(appdata.join("plugins.txt"), "*Skyrim.esm\n").unwrap();

    fs::create_dir_all(mod_src.join("Textures")).unwrap();
    fs::write(mod_src.join("Textures/wall.dds"), b"pixels").unwrap();
    fs::write(mod_src.join("CoolMod.esp"), b"TES4").unwrap();
    let archive = tmp("arc-ov").join("coolmod.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let _ = mgr.install_archive(&archive).unwrap();
    mgr.set_deploy_method(LinkMethod::Overlay).unwrap();
    mgr.deploy().unwrap();

    // Game dir stays pristine — the overlay mounts at launch instead.
    assert!(mgr.is_deployed());
    assert!(!game_dir.join("Data/Textures").exists(), "no files linked");
    assert!(!game_dir.join("Data/CoolMod.esp").exists());

    // Load order is still activated, and the wrapper is available.
    let txt = fs::read_to_string(appdata.join("plugins.txt")).unwrap();
    assert!(txt.contains("*CoolMod.esp"), "plugin still activated");
    let opt = mgr
        .vfs_launch_option()
        .expect("launch option for game-dir target");
    assert!(opt.starts_with("bwrap ") && opt.ends_with("%command%"));

    mgr.clear().unwrap();
    let txt = fs::read_to_string(appdata.join("plugins.txt")).unwrap();
    assert!(!txt.contains("CoolMod.esp"), "plugin deactivated on clear");
}

#[test]
fn overlay_method_rejected_for_prefix_docs_games() {
    use modeman_core::deploy::LinkMethod;

    let data_root = tmp("data-ovck");
    let lib = tmp("lib-ovck");
    let game_dir = lib.join("steamapps/common/CK3");
    fs::create_dir_all(&game_dir).unwrap();

    let installed = game::from_manual_path("ck3", game_dir).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    assert!(mgr.set_deploy_method(LinkMethod::Overlay).is_err());
}

#[test]
fn remove_while_deployed_leaves_no_dangling_links() {
    let data_root = tmp("data-rm");
    let lib = tmp("lib-rm");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    // Vanilla file that mod A will shadow.
    let vanilla = game_dir.join("Data/Textures/wall.dds");
    fs::create_dir_all(vanilla.parent().unwrap()).unwrap();
    fs::write(&vanilla, b"VANILLA").unwrap();

    // Mod A shadows the vanilla texture; mod B ships its own file.
    let src_a = tmp("modsrc-rm-a");
    fs::create_dir_all(src_a.join("Textures")).unwrap();
    fs::write(src_a.join("Textures/wall.dds"), b"MOD-A").unwrap();
    let arc_a = tmp("arc-rm-a").join("shadow.tar");
    make_tar(&src_a, &arc_a);

    let src_b = tmp("modsrc-rm-b");
    fs::create_dir_all(src_b.join("meshes")).unwrap();
    fs::write(src_b.join("meshes/chair.nif"), b"MOD-B").unwrap();
    let arc_b = tmp("arc-rm-b").join("chair.tar");
    make_tar(&src_b, &arc_b);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let rec_a = match mgr.install_archive(&arc_a).unwrap() {
        modeman_core::manager::InstallOutcome::Installed(r) => r,
        _ => panic!("plain archive"),
    };
    let _ = mgr.install_archive(&arc_b).unwrap();
    mgr.deploy().unwrap();
    assert_eq!(fs::read(&vanilla).unwrap(), b"MOD-A");

    // Remove mod A while deployed: its link must go, the vanilla file must
    // come back, and mod B must stay live.
    mgr.remove_mod(&rec_a.slug).unwrap();
    assert!(mgr.is_deployed(), "deployment stays live after remove");
    assert!(!vanilla.is_symlink(), "removed mod's link is gone");
    assert_eq!(fs::read(&vanilla).unwrap(), b"VANILLA", "vanilla restored");
    let b_file = game_dir.join("Data/meshes/chair.nif");
    assert!(b_file.is_symlink(), "other mod still deployed");
    assert_eq!(fs::read(&b_file).unwrap(), b"MOD-B");

    // Nothing under the deploy root may dangle.
    for entry in walk_files(&game_dir.join("Data")) {
        assert!(
            entry.exists(),
            "dangling link left behind: {}",
            entry.display()
        );
    }
}

/// All paths (files + symlinks) under a dir.
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && !p.is_symlink() {
            out.extend(walk_files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn update_archive_keeps_slot_and_refreshes_content() {
    use modeman_core::NexusRef;

    let data_root = tmp("data-up");
    let lib = tmp("lib-up");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(&game_dir).unwrap();

    // v1 of the mod, plus a second mod so order matters.
    let src_v1 = tmp("modsrc-up-v1");
    fs::create_dir_all(src_v1.join("Textures")).unwrap();
    fs::write(src_v1.join("Textures/old.dds"), b"v1").unwrap();
    let arc_v1 = tmp("arc-up-v1").join("coolmod.tar");
    make_tar(&src_v1, &arc_v1);

    let src_other = tmp("modsrc-up-o");
    fs::create_dir_all(src_other.join("meshes")).unwrap();
    fs::write(src_other.join("meshes/x.nif"), b"other").unwrap();
    let arc_other = tmp("arc-up-o").join("other.tar");
    make_tar(&src_other, &arc_other);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let rec = match mgr.install_archive(&arc_v1).unwrap() {
        modeman_core::manager::InstallOutcome::Installed(r) => r,
        _ => panic!("plain archive"),
    };
    let _ = mgr.install_archive(&arc_other).unwrap();
    mgr.set_nexus_ref(
        &rec.slug,
        NexusRef {
            domain: "skyrimspecialedition".into(),
            mod_id: 42,
            file_id: 1,
            version: "1.0".into(),
        },
    )
    .unwrap();
    mgr.set_enabled(&rec.slug, false).unwrap();
    mgr.deploy().unwrap();

    // v2 replaces the old texture with a new file.
    let src_v2 = tmp("modsrc-up-v2");
    fs::create_dir_all(src_v2.join("Textures")).unwrap();
    fs::write(src_v2.join("Textures/new.dds"), b"v2").unwrap();
    let arc_v2 = tmp("arc-up-v2").join("coolmod-v2.tar");
    make_tar(&src_v2, &arc_v2);

    let updated = match mgr.update_archive(&rec.slug, &arc_v2).unwrap() {
        modeman_core::manager::InstallOutcome::Installed(r) => r,
        _ => panic!("plain archive"),
    };
    assert_eq!(updated.slug, rec.slug, "slug is stable across updates");
    assert_eq!(mgr.mods().len(), 2, "no duplicate record");

    // Load-order slot and enabled state survive; Nexus ref carried over.
    let prof = mgr.active_profile();
    assert_eq!(prof.order[0].slug, rec.slug, "position preserved");
    assert!(!prof.order[0].enabled, "enabled state preserved");
    let stored = mgr.mods().iter().find(|m| m.slug == rec.slug).unwrap();
    assert_eq!(stored.nexus.as_ref().unwrap().mod_id, 42);

    // Store content swapped to v2.
    let mod_dir = stored.dir(mgr.store_dir());
    assert!(!mod_dir.join("Textures/old.dds").exists(), "v1 file gone");
    assert_eq!(fs::read(mod_dir.join("Textures/new.dds")).unwrap(), b"v2");

    // Live deployment was refreshed (mod disabled → nothing dangles).
    for entry in walk_files(&game_dir.join("Data")) {
        assert!(entry.exists(), "dangling link: {}", entry.display());
    }
}

#[test]
fn hardlink_method_deploys_real_files() {
    use modeman_core::deploy::LinkMethod;

    let data_root = tmp("data3");
    let lib = tmp("lib3");
    let mod_src = tmp("modsrc3");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(&game_dir).unwrap();
    fs::create_dir_all(mod_src.join("Textures")).unwrap();
    fs::write(mod_src.join("Textures/wall.dds"), b"pixels").unwrap();
    let archive = tmp("arc3").join("tex.tar");
    make_tar(&mod_src, &archive);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();
    let _ = mgr.install_archive(&archive).unwrap();
    mgr.set_deploy_method(LinkMethod::Hardlink).unwrap();
    mgr.deploy().unwrap();

    let f = game_dir.join("Data/Textures/wall.dds");
    assert!(!f.is_symlink(), "hardlink is not a symlink");
    assert_eq!(fs::read(&f).unwrap(), b"pixels");
}
