//! Keeping UE4SS's `mods.txt` in sync.
//!
//! UE4SS only loads a Lua or C++ mod if its folder name is listed in
//! `ue4ss/Mods/mods.txt` as `<FolderName> : <0|1>`. Deploying the files alone
//! is not enough, so enabling and disabling a UE4SS mod has to rewrite this
//! file — carefully, because the user may have their own entries in it.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sbmm_core::paths;

use crate::backend::DeployError;

/// One parsed line of `mods.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// A mod registration we may rewrite.
    Entry { name: String, enabled: bool },
    /// A comment or blank line, preserved verbatim.
    Raw(String),
}

/// Apply the given enabled/disabled states, leaving every other line intact.
///
/// Names absent from `states` are left exactly as the user had them; names in
/// `states` that are missing from the file are appended.
pub fn sync_mods_txt(
    game_root: &Path,
    backup_root: &Path,
    states: &BTreeMap<String, bool>,
) -> Result<(), DeployError> {
    let path = game_root.join(paths::UE4SS_MODS_TXT);
    remember_ownership(&path, backup_root)?;
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(DeployError::io(&path, e)),
    };

    let mut lines = parse(&existing);
    let mut seen: Vec<String> = Vec::new();

    for line in lines.iter_mut() {
        if let Line::Entry { name, enabled } = line {
            if let Some(desired) = states.get(name.as_str()) {
                *enabled = *desired;
                seen.push(name.clone());
            }
        }
    }

    for (name, enabled) in states {
        if !seen.contains(name) {
            lines.push(Line::Entry {
                name: name.clone(),
                enabled: *enabled,
            });
        }
    }

    write(&path, &lines)
}

/// Drop a mod's registration entirely, for when the mod is uninstalled.
pub fn remove_from_mods_txt(
    game_root: &Path,
    backup_root: &Path,
    names: &[String],
) -> Result<(), DeployError> {
    let path = game_root.join(paths::UE4SS_MODS_TXT);
    remember_ownership(&path, backup_root)?;
    let existing = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(DeployError::io(&path, e)),
    };

    let lines: Vec<Line> = parse(&existing)
        .into_iter()
        .filter(|line| !matches!(line, Line::Entry { name, .. } if names.contains(name)))
        .collect();

    write(&path, &lines)
}

/// Read the current registrations, for showing state in the UI.
pub fn read_mods_txt(game_root: &Path) -> BTreeMap<String, bool> {
    let path = game_root.join(paths::UE4SS_MODS_TXT);
    let Ok(text) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    parse(&text)
        .into_iter()
        .filter_map(|line| match line {
            Line::Entry { name, enabled } => Some((name, enabled)),
            Line::Raw(_) => None,
        })
        .collect()
}

fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
                return Line::Raw(raw.to_string());
            }
            match trimmed.split_once(':') {
                Some((name, value)) => {
                    let name = name.trim();
                    let enabled = value.trim().starts_with('1');
                    if name.is_empty() {
                        Line::Raw(raw.to_string())
                    } else {
                        Line::Entry {
                            name: name.to_string(),
                            enabled,
                        }
                    }
                }
                None => Line::Raw(raw.to_string()),
            }
        })
        .collect()
}

fn write(path: &Path, lines: &[Line]) -> Result<(), DeployError> {
    let mut out = String::new();
    for line in lines {
        match line {
            Line::Entry { name, enabled } => {
                out.push_str(&format!("{name} : {}\n", if *enabled { 1 } else { 0 }));
            }
            Line::Raw(raw) => {
                out.push_str(raw);
                out.push('\n');
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| DeployError::io(parent, e))?;
    }
    fs::write(path, out).map_err(|e| DeployError::io(path, e))
}

/// Called when no UE4SS mods are enabled any more.
///
/// If the user had their own `mods.txt` before we touched it, theirs comes
/// back untouched. If the file only exists because we wrote it, it goes away
/// entirely — otherwise disabling every mod would still leave a trace in the
/// game folder.
pub fn clear_registrations(game_root: &Path, backup_root: &Path) -> Result<(), DeployError> {
    if ownership(backup_root) == Ownership::UsersOwn {
        return restore_original(game_root, backup_root);
    }
    let path = game_root.join(paths::UE4SS_MODS_TXT);
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(DeployError::io(&path, e)),
    }
    let _ = fs::remove_file(ownership_path(backup_root));
    Ok(())
}

/// Who `mods.txt` belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ownership {
    /// The user already had one before we ever wrote to it.
    UsersOwn,
    /// It exists only because we created it.
    Ours,
}

/// Record, once and for all, whether `mods.txt` predates us.
///
/// This has to be decided the very first time we touch the file and then never
/// revisited: on a later run our own file would otherwise look like the user's,
/// and disabling every UE4SS mod would leave it behind forever.
fn remember_ownership(path: &Path, backup_root: &Path) -> Result<(), DeployError> {
    let marker = ownership_path(backup_root);
    if marker.exists() {
        return Ok(());
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent).map_err(|e| DeployError::io(parent, e))?;
    }

    if path.exists() {
        let backup = original_backup_path(backup_root);
        fs::copy(path, &backup).map_err(|e| DeployError::io(&backup, e))?;
        fs::write(&marker, "users-own").map_err(|e| DeployError::io(&marker, e))?;
    } else {
        fs::write(&marker, "ours").map_err(|e| DeployError::io(&marker, e))?;
    }
    Ok(())
}

fn ownership(backup_root: &Path) -> Ownership {
    match fs::read_to_string(ownership_path(backup_root)) {
        Ok(text) if text.trim() == "users-own" => Ownership::UsersOwn,
        _ => Ownership::Ours,
    }
}

fn ownership_path(backup_root: &Path) -> PathBuf {
    backup_root.join("_originals").join("ue4ss-mods.owner")
}

/// Put `mods.txt` back the way it was found.
pub fn restore_original(game_root: &Path, backup_root: &Path) -> Result<(), DeployError> {
    let backup = original_backup_path(backup_root);
    if !backup.exists() {
        return Ok(());
    }
    let path = game_root.join(paths::UE4SS_MODS_TXT);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| DeployError::io(parent, e))?;
    }
    fs::copy(&backup, &path).map_err(|e| DeployError::io(&path, e))?;
    fs::remove_file(&backup).map_err(|e| DeployError::io(&backup, e))?;
    let _ = fs::remove_file(ownership_path(backup_root));
    Ok(())
}

fn original_backup_path(backup_root: &Path) -> PathBuf {
    backup_root.join("_originals").join("ue4ss-mods.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_comments_and_foreign_entries() {
        let lines = parse("; my notes\nHandMade : 1\n\nOther : 0\n");
        assert_eq!(lines.len(), 4);
        assert!(matches!(lines[0], Line::Raw(_)));
        assert!(matches!(&lines[1], Line::Entry { name, enabled: true } if name == "HandMade"));
    }

    #[test]
    fn flips_only_the_requested_entries() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let backup = dir.path().join("backup");
        let path = game.join(paths::UE4SS_MODS_TXT);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "; keep me\nUserMod : 1\nOurMod : 0\n").unwrap();

        let mut states = BTreeMap::new();
        states.insert("OurMod".to_string(), true);
        sync_mods_txt(&game, &backup, &states).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("; keep me"));
        assert!(text.contains("UserMod : 1"), "foreign entry must survive");
        assert!(text.contains("OurMod : 1"));
    }

    #[test]
    fn appends_entries_that_are_not_yet_listed() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let backup = dir.path().join("backup");

        let mut states = BTreeMap::new();
        states.insert("BrandNew".to_string(), true);
        sync_mods_txt(&game, &backup, &states).unwrap();

        let text = fs::read_to_string(game.join(paths::UE4SS_MODS_TXT)).unwrap();
        assert_eq!(text, "BrandNew : 1\n");
    }

    #[test]
    fn restores_the_users_original_file() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path().join("game");
        let backup = dir.path().join("backup");
        let path = game.join(paths::UE4SS_MODS_TXT);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "UserMod : 1\n";
        fs::write(&path, original).unwrap();

        let mut states = BTreeMap::new();
        states.insert("OurMod".to_string(), true);
        sync_mods_txt(&game, &backup, &states).unwrap();
        assert_ne!(fs::read_to_string(&path).unwrap(), original);

        restore_original(&game, &backup).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }
}
