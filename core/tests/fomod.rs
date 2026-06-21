//! FOMOD scripted-installer end-to-end: a staged install that requires a
//! wizard, finished with explicit selections, then verified on disk.

use modeman_core::manager::InstallOutcome;
use modeman_core::{game, Manager};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("modeman-fomod-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn make_tar(dir: &Path, archive: &Path) {
    let ok = Command::new("tar")
        .arg("-cf").arg(archive).arg("-C").arg(dir).arg(".")
        .status().expect("tar").success();
    assert!(ok);
}

const MODULE_CONFIG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<config>
  <moduleName>Test Mod</moduleName>
  <requiredInstallFiles>
    <folder source="Core" destination="" priority="0"/>
  </requiredInstallFiles>
  <installSteps order="Explicit">
    <installStep name="Main">
      <optionalFileGroups order="Explicit">
        <group name="Texture Resolution" type="SelectExactlyOne">
          <plugins order="Explicit">
            <plugin name="1K">
              <description>low res</description>
              <files><folder source="Optional\1K" destination="textures"/></files>
              <typeDescriptor><type name="Recommended"/></typeDescriptor>
            </plugin>
            <plugin name="2K">
              <description>high res</description>
              <files><folder source="Optional\2K" destination="textures"/></files>
              <typeDescriptor><type name="Optional"/></typeDescriptor>
            </plugin>
          </plugins>
        </group>
      </optionalFileGroups>
    </installStep>
  </installSteps>
</config>
"#;

#[test]
fn fomod_install_selects_option() {
    let data_root = tmp("data");
    let lib = tmp("lib");
    let src = tmp("src");

    let game_dir = lib.join("steamapps/common/SkyrimSE");
    fs::create_dir_all(&game_dir).unwrap();

    // Build a FOMOD source tree (note mixed case + backslashes in the XML).
    fs::create_dir_all(src.join("fomod")).unwrap();
    fs::write(src.join("fomod/ModuleConfig.xml"), MODULE_CONFIG).unwrap();
    fs::create_dir_all(src.join("Core")).unwrap();
    fs::write(src.join("Core/Main.esp"), b"TES4").unwrap();
    fs::create_dir_all(src.join("Optional/1K")).unwrap();
    fs::create_dir_all(src.join("Optional/2K")).unwrap();
    fs::write(src.join("Optional/1K/wall.dds"), b"1k").unwrap();
    fs::write(src.join("Optional/2K/wall.dds"), b"2k").unwrap();

    let archive = tmp("arc").join("testmod.tar");
    make_tar(&src, &archive);

    let installed = game::from_manual_path("skyrimse", game_dir.clone()).unwrap();
    let mut mgr = Manager::open(data_root, installed).unwrap();

    // Install → should require the wizard.
    let (slug, config) = match mgr.install_archive(&archive).unwrap() {
        InstallOutcome::NeedsFomod { slug, config, .. } => (slug, config),
        InstallOutcome::Installed(_) => panic!("expected FOMOD wizard"),
    };
    assert_eq!(config.module_name, "Test Mod");
    assert_eq!(config.steps.len(), 1);
    assert_eq!(config.steps[0].groups[0].plugins.len(), 2);

    // Default would pick 1K (Recommended); override to choose 2K.
    let mut sel = config.default_selections();
    sel[0][0] = vec![false, true];

    let rec = mgr.finish_fomod(&slug, &sel).unwrap();
    let mod_dir = data_root_mod_dir(&mgr, &rec.slug);
    assert!(mod_dir.join("Main.esp").is_file(), "required file installed");
    let dds = mod_dir.join("textures/wall.dds");
    assert!(dds.is_file(), "selected option installed");
    assert_eq!(fs::read(&dds).unwrap(), b"2k", "2K variant chosen");

    // Deploy works on the FOMOD-built tree.
    mgr.deploy().unwrap();
    assert!(game_dir.join("Data/Main.esp").is_symlink());
    assert!(game_dir.join("Data/textures/wall.dds").is_symlink());
}

fn data_root_mod_dir(mgr: &Manager, slug: &str) -> PathBuf {
    mgr.store_dir().join("mods").join(slug)
}

#[test]
fn plugin_type_pattern_evaluates_flags() {
    use modeman_core::fomod::{self, PluginType};
    use std::path::Path;

    let xml = r#"<?xml version="1.0"?>
    <config>
      <moduleName>T</moduleName>
      <installSteps order="Explicit">
        <installStep name="S">
          <optionalFileGroups order="Explicit">
            <group name="G" type="SelectAny">
              <plugins order="Explicit">
                <plugin name="Patch">
                  <description>compat patch</description>
                  <files/>
                  <typeDescriptor>
                    <dependencyType>
                      <defaultType name="NotUsable"/>
                      <patterns>
                        <pattern>
                          <dependencies operator="And">
                            <flagDependency flag="hasBase" value="On"/>
                          </dependencies>
                          <type name="Recommended"/>
                        </pattern>
                      </patterns>
                    </dependencyType>
                  </typeDescriptor>
                </plugin>
              </plugins>
            </group>
          </optionalFileGroups>
        </installStep>
      </installSteps>
    </config>"#;
    let dir = tmp("cfgonly");
    fs::create_dir_all(dir.join("fomod")).unwrap();
    fs::write(dir.join("fomod/ModuleConfig.xml"), xml).unwrap();
    let (cfg, _) = fomod::find_config(&dir).unwrap();
    let config = fomod::parse(&cfg).unwrap();
    let plugin = &config.steps[0].groups[0].plugins[0];

    let none = Path::new("");
    // No flags → default NotUsable.
    assert_eq!(plugin.effective_type(&[], none), PluginType::NotUsable);
    // Flag set → pattern matches → Recommended.
    let flags = vec![("hasBase".to_string(), "On".to_string())];
    assert_eq!(plugin.effective_type(&flags, none), PluginType::Recommended);
}
