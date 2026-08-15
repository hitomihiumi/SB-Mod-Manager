//! Where the mod library lives on disk.
//!
//! Mods and the backups of displaced game files are the only things that grow
//! large, so those are what the user can relocate — typically off the system
//! drive. The database stays in the app data directory because it has to be
//! found before any setting can be read.

use std::fs;
use std::path::{Path, PathBuf};

use crate::AppError;

type Result<T> = std::result::Result<T, AppError>;

/// Reject a destination that would cause trouble later.
pub fn validate_root(new_root: &Path, game_root: Option<&Path>) -> Result<()> {
    if let Some(game) = game_root {
        // Staging inside the game folder would mean deploying a mod on top of
        // its own source, and would break the promise that disabling every mod
        // leaves the game folder as it was.
        if new_root == game || new_root.starts_with(game) {
            return Err(AppError::BadLibraryRoot(
                "the library cannot live inside the game folder".into(),
            ));
        }
    }

    fs::create_dir_all(new_root).map_err(|e| AppError::io(new_root, e))?;

    // An existing library here would disagree with what the database knows.
    let mods = new_root.join("mods");
    if mods.is_dir() && fs::read_dir(&mods).map(|d| d.count()).unwrap_or(0) > 0 {
        return Err(AppError::BadLibraryRoot(format!(
            "{} already contains a mod library; choose an empty folder",
            mods.display()
        )));
    }

    // Prove it is writable now rather than failing on the first install.
    let probe = new_root.join(".sbmm-write-test");
    fs::write(&probe, b"").map_err(|e| AppError::io(&probe, e))?;
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Move a directory, falling back to copying when the destination is on
/// another volume.
pub fn move_tree(from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }

    // Cross-volume: copy every file, then drop the original.
    for relative in sbmm_archive::walk_relative(from) {
        let src = from.join(&relative);
        let dst = to.join(&relative);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }
        fs::copy(&src, &dst).map_err(|e| AppError::io(&dst, e))?;
    }
    fs::remove_dir_all(from).map_err(|e| AppError::io(from, e))?;
    Ok(())
}

/// Whether two paths sit on the same volume.
///
/// This decides whether deployment can use hard links. When it cannot, every
/// enabled mod is stored twice — once in the library and once in the game
/// folder — which the UI needs to say out loud rather than let the user
/// discover from their free space.
pub fn same_volume(a: &Path, b: &Path) -> bool {
    let (Some(a), Some(b)) = (nearest_existing(a), nearest_existing(b)) else {
        return true; // Nothing to compare yet; do not raise a false alarm.
    };
    volume_id(&a)
        .zip(volume_id(&b))
        .map(|(x, y)| x == y)
        .unwrap_or(true)
}

/// Walk up until a path that actually exists, so a not-yet-created folder can
/// still be judged by its parent.
fn nearest_existing(path: &Path) -> Option<PathBuf> {
    let mut cursor = path;
    loop {
        if cursor.exists() {
            return Some(cursor.to_path_buf());
        }
        cursor = cursor.parent()?;
    }
}

#[cfg(windows)]
fn volume_id(path: &Path) -> Option<String> {
    use std::path::Component;
    // The drive letter or UNC share is what determines link compatibility.
    match path.components().next()? {
        Component::Prefix(prefix) => {
            Some(prefix.as_os_str().to_string_lossy().to_ascii_lowercase())
        }
        _ => None,
    }
}

#[cfg(not(windows))]
fn volume_id(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).ok().map(|m| m.dev().to_string())
}

/// Total size of the library, for showing what a move would involve.
pub fn size_of(root: &Path) -> u64 {
    sbmm_archive::directory_size(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_a_root_inside_the_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        fs::create_dir_all(&game).unwrap();

        let err = validate_root(&game.join("mods"), Some(&game)).unwrap_err();
        assert!(matches!(err, AppError::BadLibraryRoot(_)), "got {err:?}");
    }

    #[test]
    fn refuses_a_folder_that_already_holds_a_library() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");
        fs::create_dir_all(root.join("mods/Existing")).unwrap();
        fs::write(root.join("mods/Existing/a.pak"), b"x").unwrap();

        assert!(validate_root(&root, None).is_err());
    }

    #[test]
    fn accepts_a_fresh_folder_and_leaves_no_probe_behind() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("library");

        validate_root(&root, None).unwrap();
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[test]
    fn moves_a_tree_and_removes_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("old/mods");
        let to = dir.path().join("new/mods");
        fs::create_dir_all(from.join("Cool Mod")).unwrap();
        fs::write(from.join("Cool Mod/a.pak"), b"payload").unwrap();

        move_tree(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(
            fs::read_to_string(to.join("Cool Mod/a.pak")).unwrap(),
            "payload"
        );
    }

    #[test]
    fn moving_a_missing_tree_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        move_tree(&dir.path().join("absent"), &dir.path().join("dest")).unwrap();
    }

    #[test]
    fn a_folder_and_its_sibling_share_a_volume() {
        let dir = tempfile::tempdir().unwrap();
        assert!(same_volume(
            &dir.path().join("a"),
            &dir.path().join("b/c/d")
        ));
    }
}
