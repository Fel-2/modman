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
- [x] Free mod platforms — Thunderstore, mod.io, GameBanana: in-UI browse +
      download with no premium wall (unlike Nexus's premium-only download API)
- [x] FOMOD scripted installer — wizard with stepped option groups, condition
      flags, case-insensitive source resolution
- [x] Conflict viewer — which enabled mods overwrite each file, and who wins
- [x] Multi-engine deploy model — game-dir or prefix-`Documents/` targets,
      per-game flatten policy (folder-per-mod vs loose-file)
- [x] Load-order writers — RimWorld `ModsConfig.xml`, Paradox `dlc_load.json`
- [x] VFS launch wrapper + Cyberpunk REDmod — experimental, real-machine only
- [x] Keyword search across all sources — Nexus (v2 GraphQL, works without a
      key), mod.io + GameBanana (server-side), Thunderstore (full-list filter)
- [x] In-place mod updates — re-downloading a Nexus mod you already have
      replaces it, keeping its load-order slot, enabled state, and profiles
- [x] Master-aware plugin sort — plugin headers are parsed for `MAST` entries
      and dependents are ordered after their masters at deploy time
- [x] Deployment hygiene — removing or updating a mod while deployed refreshes
      the live links (no dangling symlinks, vanilla files restored)
- [x] Manual game registration — "Add game…" for GOG/Heroic/custom installs

### Supported games

| Game | Engine | Mods go to | Auto-detect |
|------|--------|-----------|-------------|
| Skyrim (LE/SE), Fallout 3/NV/4, Oblivion, Morrowind, Starfield | Creation | `Data/` | ✅ |
| Cyberpunk 2077 | REDengine | game root | ✅ |
| RimWorld | folder-per-mod | `Mods/` | ✅ |
| Stardew Valley (SMAPI) | folder-per-mod | `Mods/` | ✅ |
| Crusader Kings II / III | Paradox | prefix `Documents/.../mod/` | ✅ |
| Generic Unity (BepInEx) | Unity | `BepInEx/plugins/` | manual path |

Anything not auto-detected (GOG, Heroic, a custom Unity/BepInEx game) can be
registered via **Add game…** — pick the catalog entry and the install folder;
the registration persists in `manual-games.json` under the data dir.

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
├── nexus/       # modeman-nexus: Nexus network layer
│   ├── nxm.rs       nxm:// link parser
│   ├── client.rs    blocking Nexus REST client + downloads
│   └── protocol.rs  nxm:// .desktop handler registration
├── platform/    # modeman-platform: free platforms behind one trait
│   ├── thunderstore.rs
│   ├── modio.rs
│   └── gamebanana.rs
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

## Free mod platforms

Nexus gates in-app downloads behind premium. modeman also browses + downloads,
fully free, from:

- **Thunderstore** — `thunderstore.io`, no auth. Game = community slug
  (e.g. `lethal-company`). Best for Unity/BepInEx games.
- **mod.io** — official, free API key (no premium tier). Game = numeric
  `game_id`. Paste the key in the browse panel.
- **GameBanana** — `gamebanana.com`, no auth. Game = numeric game id. Large
  catalog incl. Cyberpunk.

In the **Browse** overlay pick a source, then Top/Newest/Updated or type a
keyword search → a mod → Download. The downloaded archive flows through the
normal install pipeline (FOMOD wizard fires if scripted). Game ids/slugs
auto-fill from the catalog where known (GameBanana ids for most games,
Thunderstore's `cyberpunk2077`); mod.io ids are entered manually — its catalog
doesn't overlap the built-in games.

The `platform` crate holds one `ModPlatform` trait with a provider each.

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
- [x] Keyword search — Nexus v2 GraphQL + per-platform search, one search box
- [x] In-place updates, master-aware plugin sorting, deploy-safe remove/update
- [x] Manual game registration ("Add game…", persisted)
- [ ] More games / engines as needed
- [ ] Overlayfs/VFS deploy backend (game dir untouched)
- [ ] GOG / Heroic / Lutris auto-detection
- [ ] Packaging: AppImage / Flatpak / AUR

## License

MIT
