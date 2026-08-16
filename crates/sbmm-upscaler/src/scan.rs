//! Finding the upscaler DLLs inside a game folder.
//!
//! Their location is not fixed. Unreal puts plugin binaries under
//! `Engine/Plugins/.../Binaries/ThirdParty/`, some builds put them next to the
//! executable, and a user who has already swapped one by hand may have put it
//! somewhere else again. So the folder is walked and files are matched by
//! name, which is the one thing the game itself relies on.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{pe, Component, Version};

/// An upscaler DLL that is present in the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFile {
    pub component: Component,
    /// Relative to the game root, so it survives the folder being moved.
    pub path: PathBuf,
    /// `None` when the file carries no readable version resource.
    pub version: Option<Version>,
}

/// Directories that never hold a DLL the game loads, and can be large.
const SKIP: &[&str] = &["~mods", "logicmods", "saved", "movies", "content"];

/// Every upscaler DLL under `game_root`.
///
/// Results are ordered by component so the UI does not reshuffle between
/// scans, and duplicates of one component are all reported: a game with two
/// copies of `nvngx_dlss.dll` needs both replaced or the older one wins
/// depending on which path the loader reaches first.
pub fn scan(game_root: &Path) -> Vec<InstalledFile> {
    let mut found = Vec::new();
    walk(game_root, game_root, 0, &mut found);
    found.sort_by(|a, b| (a.component, &a.path).cmp(&(b.component, &b.path)));
    found
}

/// Depth is bounded because the walk is over someone's disk: a symlink loop in
/// a game folder should not hang the app.
const MAX_DEPTH: usize = 12;

fn walk(root: &Path, dir: &Path, depth: usize, found: &mut Vec<InstalledFile>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            // `Content` holds the paks and is by far the biggest part of the
            // install; nothing under it is a loadable binary.
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            walk(root, &path, depth + 1, found);
            continue;
        }
        if !kind.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(component) = Component::from_file_name(&name) else {
            continue;
        };
        found.push(InstalledFile {
            component,
            path: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
            version: pe::file_version_opt(&path),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not a real dll").unwrap();
    }

    #[test]
    fn upscaler_dlls_are_found_wherever_unreal_put_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        touch(&root.join("SB/Binaries/Win64/nvngx_dlss.dll"));
        touch(
            &root.join(
                "Engine/Plugins/Runtime/Nvidia/DLSS/Binaries/ThirdParty/Win64/nvngx_dlssg.dll",
            ),
        );
        touch(&root.join("SB/Binaries/Win64/amd_fidelityfx_dx12.dll"));
        // Ordinary game files must be left alone.
        touch(&root.join("SB/Binaries/Win64/SB-Win64-Shipping.exe"));
        touch(&root.join("SB/Binaries/Win64/amd_ags_x64.dll"));

        let found = scan(root);
        let components: Vec<Component> = found.iter().map(|f| f.component).collect();
        assert_eq!(
            components,
            vec![
                Component::DlssSuperResolution,
                Component::DlssFrameGeneration,
                Component::FsrDx12,
            ]
        );
        assert_eq!(
            found[0].path,
            PathBuf::from("SB/Binaries/Win64/nvngx_dlss.dll")
        );
    }

    #[test]
    fn a_file_with_no_version_resource_is_still_reported() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("SB/Binaries/Win64/nvngx_dlss.dll"));

        let found = scan(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].version, None,
            "an unreadable version must not hide the file, only leave it unknown"
        );
    }

    #[test]
    fn both_copies_are_reported_when_a_dll_appears_twice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        touch(&root.join("SB/Binaries/Win64/nvngx_dlss.dll"));
        touch(&root.join("Engine/Binaries/ThirdParty/nvngx_dlss.dll"));

        let found = scan(root);
        assert_eq!(
            found.len(),
            2,
            "whichever the loader reaches first wins, so both matter"
        );
    }

    #[test]
    fn the_content_folder_is_not_walked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // A mod could put anything in ~mods; it is not what the game loads.
        touch(&root.join("SB/Content/Paks/~mods/nvngx_dlss.dll"));
        touch(&root.join("SB/Content/nvngx_dlss.dll"));

        assert!(scan(root).is_empty());
    }

    #[test]
    fn a_folder_that_does_not_exist_scans_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan(&dir.path().join("no-such-game")).is_empty());
    }
}
