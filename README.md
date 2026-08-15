# SB Mod Manager

A mod manager for **Stellar Blade** (PC). It works out what a downloaded
archive is, puts each piece where the game expects it, and can put the game
folder back exactly as it found it.

<img src="docs/screenshot.png" alt="The mods list, with groups and pending changes" width="900">

## Why this exists

Stellar Blade mods do not all go in one place. A pak goes in `~mods`, a
Blueprint mod goes in `LogicMods` and will not work in `~mods`, a Lua mod needs
its own folder *and* a line in UE4SS's `mods.txt`, ReShade goes next to the
executable, and CNS packages ship a whole `SB/` tree to overlay. Getting this
wrong is the usual reason a mod "doesn't work".

## What it does

- **Detects the mod type and destination.** Eight layouts are recognised from
  structural markers rather than guesswork, and a single archive can contain
  several — a logic mod bundled with a cosmetic pak is routed as two
  independent components. When nothing is conclusive it asks once and
  remembers the answer.
- **Handles IoStore properly.** Stellar Blade is UE 4.26 with IoStore, so pak
  mods are `.pak`/`.utoc`/`.ucas` triplets that must share a base name and end
  in `_P`. Missing suffixes are fixed, incomplete sets are flagged.
- **Load order that actually applies.** `~mods` is mounted alphanumerically, so
  priority is written into the deployed file names (`0010_Outfit_P.pak`) with
  every file of a set renamed together. Logic mods are left alone, because
  UE4SS resolves those itself.
- **Enable and disable one mod, a group, or everything**, with the game folder
  reconciled in a single pass.
- **Leaves nothing behind.** Every file written and every original displaced is
  recorded, so disabling everything restores the folder byte for byte —
  including removing directories and the `mods.txt` it created, while never
  touching a file it did not put there.

## Status

The MVP core is complete and covered by tests. Not yet built:

- Nexus Mods integration — API key, downloads, `nxm://` handling, collections
- ProjFS virtual filesystem as an alternative to hard-linking
- Profiles (the schema already stores state per profile)
- Asset-level conflict detection by reading `.pak` and `.utoc` indexes

## Building

Requires Rust 1.82+, Node 22+ and pnpm. On Windows you also need the WebView2
runtime, which ships with Windows 11 and recent Windows 10.

```sh
pnpm install
pnpm tauri dev      # run it
pnpm tauri build    # produce an installer
```

## Architecture

The manager is a Rust workspace of platform-free library crates plus a thin
Tauri shell. Everything worth testing lives in the libraries, so the test suite
runs on any machine — `src-tauri` is only command wrappers.

| Crate | Responsibility |
| --- | --- |
| `sbmm-core` | Mod type detection over a normalised file tree, pak set grouping, load-order naming, deployment planning |
| `sbmm-deploy` | `DeployBackend` trait, hard-link backend, the manifest that makes removal exact, UE4SS `mods.txt` syncing |
| `sbmm-game` | Steam and Epic install discovery, including a small KeyValues parser |
| `sbmm-archive` | zip/7z/rar extraction with path-traversal protection |
| `sbmm-store` | SQLite persistence: mods, groups, per-profile state, deployment record |
| `sbmm-app` | The application service the UI drives |
| `src-tauri` | Tauri commands, window, bundling |

Mods are kept unpacked in the app's data directory and hard-linked into the
game when enabled. Links cost no extra disk space and are indistinguishable
from real files to the game, and the game folder holds nothing while a mod is
disabled.

`src-tauri` is deliberately its own Cargo workspace: it needs WebView2/WebKit
system libraries, and keeping it separate lets `cargo test --workspace` run the
domain crates on any platform.

### Where mods are installed

All paths are relative to the folder containing `SB/`.

| Type | Marker | Destination |
| --- | --- | --- |
| Pak | `.pak` (+ `.utoc`/`.ucas`) | `SB/Content/Paks/~mods/` |
| Logic mod | a `LogicMods` directory | `SB/Content/Paks/LogicMods/` |
| UE4SS Lua | `scripts/main.lua` | `SB/Binaries/Win64/ue4ss/Mods/<Name>/` + `mods.txt` |
| UE4SS C++ | `dlls/main.dll` | as above |
| UE4SS runtime | `UE4SS.dll` | `SB/Binaries/Win64/` |
| Root / ReShade | `ReShade.ini`, a proxy DLL | `SB/Binaries/Win64/` |
| Movie | `.mp4`/`.bk2`, a `Movies` directory | `SB/Content/Movies/` |
| Game root overlay | an explicit `SB/` tree | laid onto the game folder |

## Development

```sh
cargo test --workspace          # domain crates
cargo clippy --workspace --all-targets
pnpm typecheck

# Populate a fake install to exercise the UI without owning the game
cargo run -p sbmm-app --example seed -- <data-dir> <game-root>

# Regenerate the icon set (no image dependencies needed)
python3 scripts/make-icons.py
```

The app's data directory is `%APPDATA%/dev.sbmm.manager` on Windows and
`~/.local/share/dev.sbmm.manager` on Linux.

## Licence

MIT
