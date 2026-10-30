# SB Mod Manager

A mod manager for **Stellar Blade** (PC). It works out what a downloaded
archive is, puts each piece where the game expects it, and can put the game
folder back exactly as it found it.

<img src="docs/screenshot.png" alt="The mods list, with groups and pending changes" width="900">

Stellar Blade mods do not all go in one place: paks go in `~mods`, Blueprint
mods in `LogicMods`, Lua mods in their own UE4SS folder plus a line in
`mods.txt`, ReShade next to the executable. Getting this wrong is the usual
reason a mod "doesn't work". This manager gets it right for you.

## Features

**Installing**

- **Automatic type detection.** Every layout in [the table
  below](#where-mods-are-installed) is recognised by structure; one archive
  can hold several, each routed on its own. Readmes and source art are set aside, and
  when nothing is conclusive it asks once and remembers.
- **IoStore done right.** `.pak`/`.utoc`/`.ucas` sets keep one base name and
  the `_P` suffix; incomplete sets are flagged.
- **Variant picker and FOMOD.** Archives with several versions of an outfit
  ask which one, with pictures. FOMOD installers run as a wizard, as in Mod
  Organizer 2. Answers are remembered for the next version.
- **Special cases handled.** Custom Nanosuit System outfits go where CNS
  reads them, `Engine.ini`-style tweaks are merged key by key into the
  game's own settings and taken back out exactly, and ReShade presets are
  recognised by their contents.
- **Batches, Nexus and collections.** Drop any number of archives at once.
  *Mod Manager Download* on Nexus installs with no further clicks (free
  accounts get one click per mod, as Nexus requires). Collections, mod lists
  (`.sbmm.json`) and a watched downloads folder feed the same queue.

**Running the game**

- **Load order that applies.** Priority is written into pak file names
  (`0010_Outfit_P.pak`), and "load after" rules are kept as you reorder.
- **Two deployment modes.** Hard links into the game folder, or an
  experimental **virtual file system** like Mod Organizer 2's: the folder
  stays untouched, and whatever the game creates goes to an *overwrite*
  folder you can turn into a mod.
- **Profiles.** Separate enabled sets and load orders, switchable in one
  click, plus desktop shortcuts and a command line:
  `sbmm.exe --profile "Cosmetics only" --launch`.
- **Dependencies checked.** Mods that need UE4SS, ReShade or another Nexus
  mod are flagged. UE4SS (Chrisr0's Stellar Blade build) installs and
  updates in one click.
- **ReShade presets.** List every preset with the effect files it is
  missing, pick the one ReShade starts with, and set one per profile.
- **DLSS, FSR and frame generation.** Swap in newer DLSS/FSR DLLs, and
  *(experimental)* run DLSS Frame Generation on RTX 20/30 cards via
  [dlssg_for_sm86](https://github.com/sdli1995/dlssg_for_sm86). Both are
  compatibility-checked and fully undoable.

**Understanding your setup**

- **Conflicts.** Reads pak and utoc indexes to name the asset two mods both
  replace and which one wins. MO2-style `+N`/`−N`/*Redundant* flags cover
  loose files, and the Files view shows the game folder as mods make it.
- **Mod details.** What a mod changes, its Nexus page info and requirements,
  notes, tags, a picture, and an editor for its INI/JSON/Lua settings.
  Your settings are kept across reinstalls and updates.
- **A fast list.** Filters, sorting, a context menu and keyboard control
  throughout (F1 lists the shortcuts).

**Safety**

- **Leaves nothing behind.** Every file written and every original displaced
  is recorded, so disabling everything restores the folder byte for byte.
- **Recovers.** Restore points of the enabled set, save backups before each
  launch, game update detection and one-click repair of files changed
  outside the manager.
- **Housekeeping.** Disk usage cleanup, a movable library, a diagnostics
  zip, signed self-updates (stable or nightly) and optional Discord presence.

## Known limitations

- **The virtual file system has not been tested against the game itself.**
  CI tests it end to end on Windows (and under Wine) with a stand-in
  process. It hooks Win32 file functions, so code calling `ntdll` directly
  sees the real folder. An Epic install may need the Epic launcher running.
- **Frame generation on RTX 20/30 is third-party.** The check reads the card
  and the game folder, but cannot prove the game will load the proxy.
- **The Nexus collection query was written without the live API.** The
  `collection.json` inside the revision archive is preferred when available.
- **Release checks use the anonymous GitHub API.** When it is rate limited,
  the app says it could not check.

## Where mods are installed

Paths are relative to the folder containing `SB/`.

| Type | Marker | Destination |
| --- | --- | --- |
| Pak | `.pak` (+ `.utoc`/`.ucas`) | `SB/Content/Paks/~mods/` |
| Logic mod | a `LogicMods` directory | `SB/Content/Paks/LogicMods/` |
| UE4SS Lua / C++ | `scripts/main.lua` / `dlls/main.dll` | `SB/Binaries/Win64/ue4ss/Mods/<Name>/` + `mods.txt` |
| UE4SS runtime | `UE4SS.dll` | `SB/Binaries/Win64/` |
| Root / ReShade | `ReShade.ini`, a proxy DLL | `SB/Binaries/Win64/` |
| ReShade preset | an INI with `Techniques=` or `[*.fx]` sections | `SB/Binaries/Win64/reshade-presets/` |
| CNS outfit | `*.dekcns.json` | `SB/Content/Paks/~mods/CustomNanosuitSystem/` |
| Engine tweak | `Engine.ini`, `GameUserSettings.ini`… | merged into `%LOCALAPPDATA%\SB\Saved\Config\WindowsNoEditor` |
| Movie | `.mp4`/`.bk2`, a `Movies` directory | `SB/Content/Movies/` |
| Splash art | `Splash.bmp`, a `Splash` directory | `SB/Content/Splash/` |
| Game root overlay | an explicit `SB/` tree | laid onto the game folder |

## Building

Requires Rust 1.82+, Node 22+ and pnpm; on Windows, the WebView2 runtime.

```sh
pnpm install
pnpm tauri dev      # run it
pnpm tauri build    # produce an installer
```

**Releases.** The version lives only in `package.json`. Bump it on `master`,
then push a matching tag (`v0.3.0` for `0.3.0`). The workflow refuses a
mismatch, because such a release would never be offered as an update.
Nightlies build daily when `master` has moved since the last one, or on
demand from the *Release* workflow. Each is versioned as the next patch
prerelease (`0.7.2-nightly.202610310300`), so the nightly channel offers it.

**Update signing.** Updates must be signed. A fork needs its own keypair:

1. Run `pnpm tauri signer generate -w .tauri/updater.key`.
2. Put the public key in `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`.
3. Store the private key and its password as the secrets
   `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

Losing the private key means existing installs can no longer update.

**Discord.** A fork can use its own application id: set `APP_ID` in
`src-tauri/src/discord.rs` and upload an `icon` art asset.

## Architecture

A Rust workspace of platform-free library crates, so the tests run on any
machine, plus a thin Tauri shell.

| Crate | Responsibility |
| --- | --- |
| `sbmm-core` | Type detection, pak sets, load-order naming, deployment plans, INI and ReShade parsing |
| `sbmm-deploy` | Hard-link backend and the manifest that makes removal exact; `mods.txt` syncing |
| `sbmm-game` | Steam and Epic install discovery |
| `sbmm-archive` | zip/7z/rar extraction with path-traversal protection |
| `sbmm-assets` | Reading `.pak`/`.utoc` indexes |
| `sbmm-store` | SQLite: mods, groups, profiles, deployment record |
| `sbmm-nexus` | Nexus REST/GraphQL client, `nxm://` links, download queue |
| `sbmm-upscaler` | DLSS/FSR DLLs, GPU rules, the RTX 20/30 frame generation package |
| `sbmm-vfs` / `sbmm-vfs-hook` | The virtual file system: layered view, mapping, launcher, and the hook DLL |
| `sbmm-shell` | Command line and profile shortcuts |
| `sbmm-app` | The application service the UI drives |
| `src-tauri` | Tauri commands, window, bundling (a separate workspace; embeds the hook — `SBMM_SKIP_VFS_HOOK=1` skips it) |

Mods stay unpacked in a library folder, which can live on any drive (only
`sbmm.db` stays in the app data folder). They are hard-linked into the game
when enabled, or shown to it by the hook. App data is in
`%APPDATA%/dev.sbmm.manager` (`~/.local/share/dev.sbmm.manager` on Linux).

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
pnpm typecheck

scripts/test-vfs-wine.sh       # VFS end to end under Wine (wine64, mingw-w64)
cargo run -p sbmm-app --example seed -- <data-dir> <game-root> [library-root]   # fake install for the UI
python3 scripts/make-icons.py  # regenerate icons

# Against real releases (download or unpack them first)
SBMM_UE4SS_ARCHIVE=/path/UE4SS_v3.1.0-6.zip cargo test -p sbmm-app --test lifecycle real_ue4ss
SBMM_SM86_PACKAGE=/path/unpacked cargo test -p sbmm-upscaler --test framegen_release
```

## Licence

MIT
