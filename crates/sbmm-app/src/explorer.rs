//! The explorer's view of the mod library: the real folders on disk, which
//! the user can move things around in.
//!
//! Everything here is confined to the mods folder. A path arrives as text from
//! the window, so it is checked name by name before it is joined to anything,
//! and a mod's own folder can be looked into but never moved or renamed.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use crate::dto::{LibraryChange, LibraryEntry, LibraryListing};
use crate::{path_string, App, AppError, Result};

/// A name that is one plain component: no separators, no way up, nothing a
/// file system treats specially.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
}

/// The names in a `/`-separated relative path, empty ones dropped.
fn parts(relative: &str) -> Vec<&str> {
    relative.split('/').filter(|p| !p.is_empty()).collect()
}

impl App {
    /// A library-relative path made real, refusing anything that is not
    /// strictly inside the mods folder.
    fn library_path(&self, relative: &str) -> Result<PathBuf> {
        let mut path = self.mods_dir();
        for part in parts(relative) {
            if !valid_name(part) {
                return Err(AppError::Invalid(format!(
                    "{relative} is not a path inside the mod library"
                )));
            }
            path.push(part);
        }
        Ok(path)
    }

    /// One folder of the library. The root lists the mods, by name.
    pub fn library_list(&self, dir: &str) -> Result<LibraryListing> {
        let names = parts(dir);
        let dir = names.join("/");
        let path = self.library_path(&dir)?;
        let records = self.store.list_mods()?;
        let owner = names
            .first()
            .and_then(|top| records.iter().find(|r| r.staging_folder == *top));

        let read = std::fs::read_dir(&path).map_err(|source| AppError::Io {
            path: path.clone(),
            source,
        })?;
        let mut entries = Vec::new();
        for item in read.flatten() {
            let Ok(meta) = item.metadata() else { continue };
            let name = item.file_name().to_string_lossy().into_owned();
            // In the root each folder is a mod of its own.
            let own = if names.is_empty() {
                records.iter().find(|r| r.staging_folder == name)
            } else {
                owner
            };
            entries.push(LibraryEntry {
                label: if names.is_empty() {
                    own.map(|r| r.name.clone())
                } else {
                    None
                },
                is_dir: meta.is_dir(),
                path: if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/{name}")
                },
                absolute: path_string(item.path()),
                size: if meta.is_dir() { 0 } else { meta.len() as i64 },
                modified: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64),
                mod_id: own.map(|r| r.id),
                name,
            });
        }
        let key = |e: &LibraryEntry| e.label.as_deref().unwrap_or(&e.name).to_lowercase();
        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| key(a).cmp(&key(b))));

        Ok(LibraryListing {
            dir,
            mod_id: owner.map(|r| r.id),
            mod_name: owner.map(|r| r.name.clone()),
            entries,
        })
    }

    /// The mods behind a set of top-level folder names.
    fn mods_in(&self, folders: &BTreeSet<String>) -> Result<Vec<i64>> {
        Ok(self
            .store
            .list_mods()?
            .into_iter()
            .filter(|r| folders.contains(&r.staging_folder))
            .map(|r| r.id)
            .collect())
    }

    /// Move files and folders into `to_dir`, one by one: what cannot go (the
    /// name is taken, it would land inside itself) is reported and left, and
    /// the rest still moves.
    pub fn library_move(&self, paths: &[String], to_dir: &str) -> Result<LibraryChange> {
        let to = parts(to_dir).join("/");
        if to.is_empty() {
            return Err(AppError::Invalid(
                "Choose a folder inside a mod: the top level holds only the mods' own folders."
                    .into(),
            ));
        }
        let dest = self.library_path(&to)?;
        if !dest.is_dir() {
            return Err(AppError::Invalid(format!("{to} is not a folder")));
        }

        let mut change = LibraryChange::default();
        let mut touched = BTreeSet::new();
        for path in paths {
            let names = parts(path);
            let Some(name) = names.last() else { continue };
            // Checked first: a malformed path is an error, not something to skip.
            let src = self.library_path(path)?;
            if names.len() < 2 {
                change
                    .skipped
                    .push(format!("{name}: a mod's own folder cannot be moved"));
                continue;
            }
            let target = dest.join(name);
            if !src.exists() {
                change
                    .skipped
                    .push(format!("{name}: it is no longer there"));
            } else if src.is_dir() && dest.starts_with(&src) {
                change
                    .skipped
                    .push(format!("{name}: a folder cannot go inside itself"));
            } else if target == src {
                // Already where it was asked to go.
            } else if target.exists() {
                change.skipped.push(format!(
                    "{name}: the folder already has something by that name"
                ));
            } else {
                match std::fs::rename(&src, &target) {
                    Ok(()) => {
                        change.moved += 1;
                        touched.insert(names[0].to_string());
                    }
                    Err(error) => change.skipped.push(format!("{name}: {error}")),
                }
            }
        }
        if change.moved > 0 {
            if let Some(top) = parts(&to).first() {
                touched.insert(top.to_string());
            }
        }
        change.mod_ids = self.mods_in(&touched)?;
        Ok(change)
    }

    /// Give a file or folder inside a mod another name.
    pub fn library_rename(&self, path: &str, new_name: &str) -> Result<LibraryChange> {
        let names = parts(path);
        if names.len() < 2 {
            return Err(AppError::Invalid(
                "A mod's own folder cannot be renamed here; rename the mod in the list.".into(),
            ));
        }
        let new_name = new_name.trim();
        if !valid_name(new_name) {
            return Err(AppError::Invalid(format!(
                "“{new_name}” is not a usable name"
            )));
        }
        let src = self.library_path(path)?;
        let target = src.with_file_name(new_name);
        if target == src {
            return Ok(LibraryChange::default());
        }
        if target.exists() {
            return Err(AppError::Invalid(format!(
                "There is already something called “{new_name}” here"
            )));
        }
        std::fs::rename(&src, &target).map_err(|source| AppError::Io { path: src, source })?;
        Ok(LibraryChange {
            moved: 1,
            skipped: Vec::new(),
            mod_ids: self.mods_in(&BTreeSet::from([names[0].to_string()]))?,
        })
    }

    /// Make a new folder inside a mod.
    pub fn library_mkdir(&self, dir: &str, name: &str) -> Result<()> {
        if parts(dir).is_empty() {
            return Err(AppError::Invalid(
                "Open a mod first; new folders go inside one.".into(),
            ));
        }
        let name = name.trim();
        if !valid_name(name) {
            return Err(AppError::Invalid(format!("“{name}” is not a usable name")));
        }
        let path = self.library_path(dir)?.join(name);
        std::fs::create_dir(&path).map_err(|source| AppError::Io { path, source })
    }
}
