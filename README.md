# modeman

Linux-first mod manager for games. Rust + [Slint](https://slint.dev) GUI.

Starting with Bethesda titles (Skyrim SE, Fallout 4, Starfield, …) and
Cyberpunk 2077. Designed around Proton/Steam realities on Linux.

## Status

- [x] Steam game detection (parses `libraryfolders.vdf` + `appmanifest_*.acf`)
- [x] Mod install from archive (zip / 7z / rar / tar via libarchive)
- [x] Per-game mod store outside the game install
- [x] Profiles with ordered, toggleable load order
- [x] Deployment into the game dir — symlink **or** hardlink, revertable via
      manifest; overwritten vanilla files are backed up and restored on clear
- [x] Creation Engine plugin activation — writes `plugins.txt` in the Proton
      prefix, preserving base/DLC masters
- [x] Nexus Mods integration — API-key login, `nxm://` link download + install,
      protocol-handler registration, in-UI browse (trending/latest/updated)
- [x] FOMOD scripted installer — wizard with stepped option groups, condition
      flags, case-insensitive source resolution
- [x] Conflict viewer — which enabled mods overwrite each file, and who wins
- [x] Multi-engine deploy model — game-dir or prefix-`Documents/` targets,
      per-game flatten policy (folder-per-mod vs loose-file)
- [x] Load-order writers — RimWorld `ModsConfig.xml`, Paradox `dlc_load.json`
- [x] VFS launch wrapper + Cyberpunk REDmod — experimental, real-machine only

### Supported games

| Game | Engine | Mods go to | Auto-detect |
|------|--------|-----------|-------------|
| Skyrim (LE/SE), Fallout 3/NV/4, Oblivion, Morrowind, Starfield | Creation | `Data/` | ✅ |
| Cyberpunk 2077 | REDengine | game root | ✅ |
| RimWorld | folder-per-mod | `Mods/` | ✅ |
| Stardew Valley (SMAPI) | folder-per-mod | `Mods/` | ✅ |
| Crusader Kings II / III | Paradox | prefix `Documents/.../mod/` | ✅ |
| Generic Unity (BepInEx) | Unity | `BepInEx/plugins/` | manual path |

Load-order writers ship for: Creation Engine (`plugins.txt`), RimWorld
(`ModsConfig.xml`, by `packageId`), Paradox (`dlc_load.json`, by `.mod`
descriptor). SMAPI and BepInEx need none — SMAPI resolves order from each mod's
`manifest.json`, BepInEx loads all plugins — so those are deploy-complete.
Mod display names are read from `About.xml` / `manifest.json` where present.

## Architecture

```
modeman/
├── core/        # modeman-core: engine, no network deps
│   ├── vdf.rs       Steam KeyValues parser
│   ├── game.rs      catalog + Steam detection + Proton prefix paths
│   ├── archive.rs   libarchive extraction
│   ├── store.rs     per-game mod storage + slugging + staging
│   ├── profile.rs   profiles / load order
│   ├── plugins.rs   plugins.txt activation (Creation Engine)
│   ├── fomod.rs     FOMOD scripted-installer parse + apply
│   ├── loadorder.rs RimWorld / Paradox / SMAPI load-order + metadata
│   ├── paradoxdb.rs Paradox launcher-v2 playset SQLite integration
│   ├── redmod.rs    Cyberpunk REDmod deploy (Proton)
│   ├── vfs.rs       bubblewrap overlay launch-wrapper
│   ├── conflict.rs  file-overwrite conflict detection
│   ├── deploy.rs    Deployer trait + SymlinkDeployer
│   └── manager.rs   orchestration + JSON persistence
├── nexus/       # modeman-nexus: network layer
│   ├── nxm.rs       nxm:// link parser
│   ├── client.rs    blocking Nexus REST client + downloads
│   └── protocol.rs  nxm:// .desktop handler registration
└── gui/         # modeman-gui: Slint front end (bin: `modeman`)
    ├── ui/app.slint
    └── src/main.rs  worker threads → mpsc → Slint Timer
```

Deployment sits behind a `Deployer` trait. `LinkDeployer` ships first (symlink
or hardlink, user-selectable); an overlayfs/VFS backend can be added without
touching the rest. Deploy is non-destructive: any pre-existing real file it
shadows is moved to `*.modeman-orig` and restored on clear.

### Data layout

```
$XDG_DATA_HOME/modeman/games/<game-id>/
├── mods/<mod-slug>/   extracted mod trees
└── state.json         profiles, mod records, live deploy manifest
```

## Build & run

System dependency: **libarchive** (extraction backend).

```sh
# Arch/Artix
sudo pacman -S libarchive

cargo run -p modeman-gui      # launches `modeman`
```

## Nexus Mods

1. Get a personal API key: nexusmods.com → Settings → API → "Personal key".
2. Paste it into the **Nexus Mods** box, hit **Save** / **Validate**.
3. Register the protocol handler so the site's "Mod Manager Download" button
   routes here:
   ```sh
   modeman --register-nxm   # (or call modeman_nexus::install_protocol_handler)
   ```
4. Click **Mod Manager Download** on Nexus → modeman downloads and installs the
   archive into the matching game's store. Or paste an `nxm://` link into the
   GUI and hit **Download & install**.

Free accounts download via the `nxm://` link (its one-time key authorizes the
fetch). Premium accounts can also resolve direct links.

## Experimental (real-machine only)

These ship but can't be validated headless — try them on a real install:

- **VFS launch wrapper** — `Manager::vfs_launch_option()` returns a
  `bwrap --overlay … -- %command%` string. Paste it into the game's Steam
  launch options; mods are overlaid over the game dir only while it runs, so the
  install stays pristine. Needs `bubblewrap` with overlay support.
- **Cyberpunk REDmod** — `Manager::redmod_deploy()` runs the bundled
  `redMod.exe deploy` through a detected Proton runtime to compile `mods/` into
  `archive/pc/mod/`. Legacy `archive/pc/mod` mods already work via plain deploy.

## Roadmap

- [x] Local management (detect, install, profiles, deploy)
- [x] `plugins.txt` activation
- [x] Nexus API: key login, `nxm://` download + install, protocol handler
- [x] FOMOD scripted installer (wizard)
- [x] Conflict viewer
- [x] RimWorld `ModsConfig.xml` load order (by `packageId`)
- [x] Paradox `dlc_load.json` load order (by `.mod` descriptor)
- [x] SMAPI / BepInEx — deploy-complete (no order file); names from manifest
- [x] VFS launch wrapper (bubblewrap overlay) — pristine game dir, experimental
- [x] Cyberpunk REDmod deploy (redMod.exe via Proton) — experimental
- [x] In-UI Nexus browse — trending / latest / updated lists, mod files,
      one-click download+install (premium); free accounts use the `nxm://` button
- [x] FOMOD pattern-based plugin `typeDescriptor` evaluation (flag-conditional)
- [x] Paradox launcher-v2 playset DB — toggles `enabled` in the active playset
      (conservative; backs up the DB)
- [x] UI polish — mod sizes, FOMOD option images/descriptions, Nexus update
      check (newer-file detection), GitHub Actions CI (fmt + clippy + tests)
- [ ] More games / engines as needed
- [ ] Overlayfs/VFS deploy backend (game dir untouched)
- [ ] GOG / Heroic / Lutris detection

## License

MIT
