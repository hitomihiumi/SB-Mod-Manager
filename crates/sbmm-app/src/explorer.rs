//! The explorer's view of the mod library: the real folders on disk, which
//! the user can move things around in.
//!
//! Everything here is confined to the mods folder. A path arrives as text from
//! the window, so it is checked name by name before it is joined to anything,
//! and a mod's own folder can be looked into but never moved or renamed.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use base64::Engine;

use crate::dto::{LibraryChange, LibraryEntry, LibraryListing, LibraryPreview};
use crate::{copy_tree, path_string, App, AppError, Result};

/// Deleted items are kept this long in the library's trash, to undo.
const TRASH_DAYS: u64 = 7;
/// The start of a text file the preview shows.
const PREVIEW_TEXT_BYTES: u64 = 128 * 1024;
/// An image larger than this is not previewed.
const PREVIEW_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
/// What a trash entry remembers: where each item came from.
const TRASH_MANIFEST: &str = "restore.json";

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
            undo: None,
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

    // -- copying, deleting, bringing files in ---------------------------------

    /// Copy files and folders into `to_dir`. A name already taken there gets
    /// " - copy" (then " - copy 2" and so on), so copying into the same
    /// folder duplicates.
    pub fn library_copy(&self, paths: &[String], to_dir: &str) -> Result<LibraryChange> {
        let (to, dest) = self.destination(to_dir)?;
        let mut change = LibraryChange::default();
        let mut touched = BTreeSet::new();
        for path in paths {
            let names = parts(path);
            let Some(name) = names.last() else { continue };
            let src = self.library_path(path)?;
            if names.len() < 2 {
                change
                    .skipped
                    .push(format!("{name}: copy a mod's files, not its own folder"));
                continue;
            }
            if !src.exists() {
                change
                    .skipped
                    .push(format!("{name}: it is no longer there"));
                continue;
            }
            if src.is_dir() && dest.starts_with(&src) {
                change
                    .skipped
                    .push(format!("{name}: a folder cannot be copied into itself"));
                continue;
            }
            let target = free_name(&dest, name);
            match copy_any(&src, &target) {
                Ok(()) => change.moved += 1,
                Err(error) => change.skipped.push(format!("{name}: {error}")),
            }
        }
        if change.moved > 0 {
            touched.insert(parts(&to)[0].to_string());
        }
        change.mod_ids = self.mods_in(&touched)?;
        Ok(change)
    }

    /// Bring files and folders from elsewhere on the computer into `to_dir`
    /// — dropped onto the explorer from the system's file manager. They are
    /// copied; the originals stay where they are.
    pub fn library_import(&self, sources: &[String], to_dir: &str) -> Result<LibraryChange> {
        let (to, dest) = self.destination(to_dir)?;
        let mut change = LibraryChange::default();
        for source in sources {
            let src = PathBuf::from(source);
            let name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| source.clone());
            if !src.exists() {
                change
                    .skipped
                    .push(format!("{name}: it is no longer there"));
                continue;
            }
            if dest.starts_with(&src) {
                change
                    .skipped
                    .push(format!("{name}: a folder cannot be copied into itself"));
                continue;
            }
            let target = free_name(&dest, &name);
            match copy_any(&src, &target) {
                Ok(()) => change.moved += 1,
                Err(error) => change.skipped.push(format!("{name}: {error}")),
            }
        }
        if change.moved > 0 {
            change.mod_ids = self.mods_in(&BTreeSet::from([parts(&to)[0].to_string()]))?;
        }
        Ok(change)
    }

    /// Delete files and folders inside mods — into the library's trash, so
    /// the change can be taken back with [`App::library_restore`] for a week.
    /// A mod's own folder goes with uninstalling the mod, not from here.
    pub fn library_delete(&self, paths: &[String]) -> Result<LibraryChange> {
        self.purge_trash();
        let id = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let bin = self.trash_dir().join(&id);
        let mut change = LibraryChange::default();
        let mut touched = BTreeSet::new();
        let mut restore: Vec<String> = Vec::new();
        for path in paths {
            let names = parts(path);
            let Some(name) = names.last() else { continue };
            let src = self.library_path(path)?;
            if names.len() < 2 {
                change.skipped.push(format!(
                    "{name}: a mod's own folder goes when the mod is uninstalled"
                ));
                continue;
            }
            if !src.exists() {
                continue;
            }
            let target = bin.join(restore.len().to_string());
            if let Err(error) = std::fs::create_dir_all(&bin).and_then(|()| {
                std::fs::rename(&src, &target).or_else(|_| {
                    copy_any(&src, &target)?;
                    remove_any(&src)
                })
            }) {
                change.skipped.push(format!("{name}: {error}"));
                continue;
            }
            restore.push(names.join("/"));
            change.moved += 1;
            touched.insert(names[0].to_string());
        }
        if !restore.is_empty() {
            let manifest = bin.join(TRASH_MANIFEST);
            std::fs::write(&manifest, serde_json::to_vec(&restore)?)
                .map_err(|e| AppError::io(&manifest, e))?;
            change.undo = Some(id);
        }
        change.mod_ids = self.mods_in(&touched)?;
        Ok(change)
    }

    /// Put back what a delete took out, where it was. What finds its place
    /// taken stays in the trash and is reported.
    pub fn library_restore(&self, undo: &str) -> Result<LibraryChange> {
        if !undo.chars().all(|c| c.is_ascii_digit()) || undo.is_empty() {
            return Err(AppError::Invalid(
                "there is nothing like that to undo".into(),
            ));
        }
        let bin = self.trash_dir().join(undo);
        let manifest = bin.join(TRASH_MANIFEST);
        let text = std::fs::read(&manifest).map_err(|e| AppError::io(&manifest, e))?;
        let restore: Vec<String> = serde_json::from_slice(&text)?;
        let mut change = LibraryChange::default();
        let mut touched = BTreeSet::new();
        let mut left = false;
        for (index, path) in restore.iter().enumerate() {
            let held = bin.join(index.to_string());
            let target = self.library_path(path)?;
            let name = parts(path).last().copied().unwrap_or_default().to_string();
            if target.exists() {
                change
                    .skipped
                    .push(format!("{name}: something else is in its place now"));
                left = true;
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            match std::fs::rename(&held, &target) {
                Ok(()) => {
                    change.moved += 1;
                    touched.insert(parts(path)[0].to_string());
                }
                Err(error) => {
                    change.skipped.push(format!("{name}: {error}"));
                    left = true;
                }
            }
        }
        if !left {
            let _ = std::fs::remove_dir_all(&bin);
        }
        change.mod_ids = self.mods_in(&touched)?;
        Ok(change)
    }

    /// What a file holds, for the explorer's preview: a picture, the start
    /// of a text file, or nothing for anything else.
    pub fn library_preview(&self, path: &str) -> Result<LibraryPreview> {
        let full = self.library_path(path)?;
        let meta = std::fs::metadata(&full).map_err(|e| AppError::io(&full, e))?;
        let size = meta.len();
        let ext = full
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let none = || LibraryPreview {
            kind: "none".into(),
            image: None,
            text: None,
            truncated: false,
            size: size as i64,
        };
        if meta.is_dir() {
            return Ok(none());
        }
        let mime = match ext.as_str() {
            "png" => Some("image/png"),
            "jpg" | "jpeg" => Some("image/jpeg"),
            "webp" => Some("image/webp"),
            "gif" => Some("image/gif"),
            "bmp" => Some("image/bmp"),
            _ => None,
        };
        if let Some(mime) = mime {
            if size > PREVIEW_IMAGE_BYTES {
                return Ok(none());
            }
            let bytes = std::fs::read(&full).map_err(|e| AppError::io(&full, e))?;
            return Ok(LibraryPreview {
                kind: "image".into(),
                image: Some(format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(bytes)
                )),
                text: None,
                truncated: false,
                size: size as i64,
            });
        }
        let texty = matches!(
            ext.as_str(),
            "txt"
                | "md"
                | "ini"
                | "cfg"
                | "conf"
                | "json"
                | "lua"
                | "toml"
                | "yaml"
                | "yml"
                | "xml"
                | "log"
                | "csv"
                | "nfo"
        );
        if !texty {
            return Ok(none());
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        std::fs::File::open(&full)
            .and_then(|file| file.take(PREVIEW_TEXT_BYTES).read_to_end(&mut bytes))
            .map_err(|e| AppError::io(&full, e))?;
        let text = match bytes.strip_prefix(&[0xFF, 0xFE]) {
            Some(rest) => String::from_utf16_lossy(
                &rest
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>(),
            ),
            None => {
                String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes))
                    .into_owned()
            }
        };
        Ok(LibraryPreview {
            kind: "text".into(),
            image: None,
            text: Some(text),
            truncated: size > PREVIEW_TEXT_BYTES,
            size: size as i64,
        })
    }

    /// A folder inside a mod to put things in.
    fn destination(&self, to_dir: &str) -> Result<(String, PathBuf)> {
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
        Ok((to, dest))
    }

    fn trash_dir(&self) -> PathBuf {
        self.library_root().join("trash")
    }

    /// Forget deletes older than [`TRASH_DAYS`].
    fn purge_trash(&self) {
        let Ok(entries) = std::fs::read_dir(self.trash_dir()) else {
            return;
        };
        let keep = std::time::Duration::from_secs(TRASH_DAYS * 24 * 60 * 60);
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > keep);
            if old {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// `name` in `dir`, or the first of "name - copy", "name - copy 2"… that is
/// free. The extension stays at the end.
fn free_name(dir: &std::path::Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| {
            let suffix = if n == 1 {
                " - copy".to_string()
            } else {
                format!(" - copy {n}")
            };
            dir.join(format!("{stem}{suffix}{ext}"))
        })
        .find(|candidate| !candidate.exists())
        .expect("an unused name")
}

fn copy_any(src: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(to)?;
        copy_tree(src, to).map_err(|e| std::io::Error::other(e.to_string()))
    } else {
        std::fs::copy(src, to).map(|_| ())
    }
}

fn remove_any(path: &std::path::Path) -> std::io::Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}
