# Architecture

A Rust workspace of platform-free library crates, so the tests run on any
machine, plus a thin Tauri shell with a React front end.

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

The front end lives in `src/` (React, Zustand, Tailwind) and talks to
`sbmm-app` only through the Tauri commands in `src-tauri/src/`.

## How a mod flows through

1. **Extract.** `sbmm-archive` unpacks the archive into the mod library,
   refusing paths that escape it.
2. **Detect.** `sbmm-core` classifies each piece by structure (see
   [Where mods are installed](mod-types.md)) and builds a deployment plan.
3. **Store.** `sbmm-store` records the mod, its group, and its place in each
   profile.
4. **Deploy.** `sbmm-deploy` hard-links the files into the game folder and
   writes a manifest of everything written and everything displaced, or
   `sbmm-vfs` presents the same layout to the game without touching the
   folder.
5. **Undo.** Removal replays the manifest, so the folder comes back byte for
   byte.

## Data on disk

Mods stay unpacked in a library folder, which can live on any drive (only
`sbmm.db` stays in the app data folder). App data is in
`%APPDATA%/dev.sbmm.manager` (`~/.local/share/dev.sbmm.manager` on Linux).
