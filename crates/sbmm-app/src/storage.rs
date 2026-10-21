//! Where the library's disk space goes, getting some of it back, and
//! watching the browser's downloads folder for mod archives.
//!
//! Cleaning only ever touches what the manager can rebuild or no longer
//! needs: extracted archives no mod owns, downloaded archives that are not
//! in flight, older automatic save backups and logs. Game files the manager
//! set aside are counted but never offered for deletion — they are what
//! disabling a mod puts back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::dto::{FoundArchive, StoragePart, StorageReport, UnusedMod};
use crate::{App, AppError, Result};

const SETTING_WATCH_DIR: &str = "watchDir";
const SETTING_WATCH_INSTALL: &str = "watchAutoInstall";
const SETTING_WATCH_SEEN: &str = "watchSeen";
/// Archives remembered as already offered; older ones fall off.
const MAX_SEEN: usize = 500;
/// A folder or file this new may belong to an install or download that is
/// still going on.
const SETTLE: Duration = Duration::from_secs(10 * 60);
/// Automatic save backups kept when they are cleaned.
const KEEP_SAVE_BACKUPS: usize = 3;
const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "7z", "rar"];

/// What the user can choose to clean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Cleanup {
    /// Extracted archives no installed mod owns.
    Orphans,
    /// Downloaded archives not being fetched or installed.
    Downloads,
    /// Automatic save backups but the newest few.
    SaveBackups,
    /// Launch and hook logs.
    Logs,
}

fn older_than(path: &Path, age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|elapsed| elapsed >= age)
}

fn size_and_count(path: &Path) -> (u64, usize) {
    if path.is_file() {
        return (std::fs::metadata(path).map(|m| m.len()).unwrap_or(0), 1);
    }
    let files = sbmm_archive::walk_relative(path);
    let bytes = files
        .iter()
        .filter_map(|f| std::fs::metadata(path.join(f)).ok())
        .map(|m| m.len())
        .sum();
    (bytes, files.len())
}

fn remove(path: &Path) -> u64 {
    let (bytes, _) = size_and_count(path);
    let gone = if path.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    if gone.is_ok() {
        bytes
    } else {
        0
    }
}

fn is_archive(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|e| ARCHIVE_EXTENSIONS.contains(&e.as_str()))
}

impl App {
    // -- disk space ---------------------------------------------------------

    /// Folders in the library's mod folder that no mod owns and that are
    /// not fresh enough to be an install waiting for an answer.
    fn orphan_folders(&self) -> Result<Vec<PathBuf>> {
        let owned: BTreeSet<String> = self
            .store
            .list_mods()?
            .into_iter()
            .map(|m| m.staging_folder)
            .collect();
        let Ok(entries) = std::fs::read_dir(self.mods_dir()) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                !owned.contains(name.as_ref())
            })
            .filter(|p| older_than(p, SETTLE))
            .collect())
    }

    /// Files in the downloads folder that no download in flight writes to.
    fn spare_downloads(&self) -> Result<Vec<PathBuf>> {
        let busy: BTreeSet<String> = self
            .store
            .list_downloads()?
            .into_iter()
            .filter(|d| matches!(d.state.as_str(), "queued" | "running" | "needsUserAction"))
            .map(|d| d.file_name)
            .collect();
        let dir = self.downloads_dir();
        Ok(sbmm_archive::walk_relative(&dir)
            .into_iter()
            .map(|rel| dir.join(rel))
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                // A part-finished file is named after its archive.
                !busy.iter().any(|b| name.starts_with(b.as_str()))
            })
            .filter(|p| older_than(p, SETTLE))
            .collect())
    }

    fn old_save_backups(&self) -> Result<Vec<PathBuf>> {
        let dir = self.save_backups_dir();
        Ok(self
            .saves()?
            .backups
            .into_iter()
            .filter(|b| b.kind == "auto")
            .skip(KEEP_SAVE_BACKUPS)
            .map(|b| dir.join(b.file))
            .collect())
    }

    fn log_files(&self) -> Vec<PathBuf> {
        let dir = self.vfs_dir();
        sbmm_archive::walk_relative(&dir)
            .into_iter()
            .map(|rel| dir.join(rel))
            .filter(|p| p.extension().is_some_and(|e| e == "log"))
            .collect()
    }

    /// Where the library's space goes. Walks every file, so it is run on
    /// request rather than with every snapshot.
    pub fn storage_report(&self) -> Result<StorageReport> {
        let library = self.library_root();
        let sum = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|p| size_and_count(p))
                .fold((0, 0), |(b, n), (pb, pn)| (b + pb, n + pn))
        };
        let part = |key: &str, label: &str, path: PathBuf, cleanable: Option<(u64, usize)>| {
            let (bytes, files) = size_and_count(&path);
            let (spare_bytes, spare_files) = cleanable.unwrap_or((0, 0));
            StoragePart {
                key: key.to_string(),
                label: label.to_string(),
                path: path.to_string_lossy().into_owned(),
                bytes,
                files,
                cleanable_bytes: spare_bytes,
                cleanable_files: spare_files,
            }
        };

        let orphans = sum(&self.orphan_folders()?);
        let downloads = sum(&self.spare_downloads()?);
        let saves = sum(&self.old_save_backups()?);
        let logs = sum(&self.log_files());
        let parts = vec![
            part("mods", "Installed mods", self.mods_dir(), Some(orphans)),
            part(
                "downloads",
                "Downloads",
                self.downloads_dir(),
                Some(downloads),
            ),
            part("backups", "Game files set aside", self.backup_dir(), None),
            part(
                "saves",
                "Save backups",
                self.save_backups_dir(),
                Some(saves),
            ),
            part(
                "overwrite",
                "Files the game created",
                self.overwrite_dir(),
                None,
            ),
            part(
                "settings",
                "Settings copies",
                library.join("pristine"),
                None,
            ),
            part(
                "vfs",
                "Virtual file system and logs",
                self.vfs_dir(),
                Some(logs),
            ),
        ];

        let enabled_somewhere: BTreeSet<i64> = self.store.mods_enabled_anywhere()?;
        let mut unused: Vec<UnusedMod> = self
            .store
            .list_mods()?
            .into_iter()
            .filter(|m| !enabled_somewhere.contains(&m.id))
            .map(|m| UnusedMod {
                id: m.id,
                name: m.name,
                bytes: m.size_bytes.max(0) as u64,
            })
            .collect();
        unused.sort_by_key(|m| std::cmp::Reverse(m.bytes));

        Ok(StorageReport {
            total: parts.iter().map(|p| p.bytes).sum(),
            parts,
            unused_mods: unused,
        })
    }

    /// Delete what `what` names and return the bytes freed.
    pub fn clean(&mut self, what: Cleanup) -> Result<u64> {
        let paths = match what {
            Cleanup::Orphans => self.orphan_folders()?,
            Cleanup::Downloads => self.spare_downloads()?,
            Cleanup::SaveBackups => self.old_save_backups()?,
            Cleanup::Logs => self.log_files(),
        };
        Ok(paths.iter().map(|p| remove(p)).sum())
    }

    // -- watching a downloads folder ---------------------------------------

    pub fn watch_dir(&self) -> Result<Option<PathBuf>> {
        Ok(self
            .store
            .get_setting(SETTING_WATCH_DIR)?
            .map(PathBuf::from))
    }

    pub fn watch_auto_install(&self) -> Result<bool> {
        Ok(self.store.get_setting(SETTING_WATCH_INSTALL)?.as_deref() == Some("1"))
    }

    pub fn set_watch_auto_install(&mut self, on: bool) -> Result<()> {
        self.store
            .set_setting(SETTING_WATCH_INSTALL, if on { "1" } else { "0" })?;
        Ok(())
    }

    /// Watch `dir`, or stop with `None`. What is already in it counts as
    /// seen: only archives that arrive from now on are offered.
    pub fn set_watch_dir(&mut self, dir: Option<&Path>) -> Result<()> {
        match dir {
            Some(dir) => {
                if !dir.is_dir() {
                    return Err(AppError::Invalid(format!(
                        "{} is not a folder",
                        dir.display()
                    )));
                }
                self.store
                    .set_setting(SETTING_WATCH_DIR, &dir.to_string_lossy())?;
                let present: Vec<String> = self
                    .archives_in_watch_dir()?
                    .into_iter()
                    .map(|a| a.key)
                    .collect();
                self.mark_seen(&present)?;
            }
            None => self.store.delete_setting(SETTING_WATCH_DIR)?,
        }
        Ok(())
    }

    fn seen(&self) -> Result<Vec<String>> {
        Ok(self
            .store
            .get_setting(SETTING_WATCH_SEEN)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    /// Remember archives as offered (installed or ignored), by name and size.
    pub fn mark_seen(&mut self, keys: &[String]) -> Result<()> {
        let mut seen = self.seen()?;
        for key in keys {
            if !seen.contains(key) {
                seen.push(key.clone());
            }
        }
        let overflow = seen.len().saturating_sub(MAX_SEEN);
        seen.drain(..overflow);
        self.store
            .set_setting(SETTING_WATCH_SEEN, &serde_json::to_string(&seen)?)?;
        Ok(())
    }

    fn archives_in_watch_dir(&self) -> Result<Vec<FoundArchive>> {
        let Some(dir) = self.watch_dir()? else {
            return Ok(Vec::new());
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut found: Vec<FoundArchive> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_archive(p))
            .filter_map(|p| {
                let bytes = std::fs::metadata(&p).ok()?.len();
                let name = p.file_name()?.to_string_lossy().into_owned();
                Some(FoundArchive {
                    key: format!("{name}|{bytes}"),
                    path: p.to_string_lossy().into_owned(),
                    name,
                    bytes,
                })
            })
            .collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// Archives in the watched folder not offered before. A browser writes
    /// to a temporary name and renames at the end, but some do not, so the
    /// caller also waits for a size to hold still between two looks — see
    /// [`settled`].
    pub fn new_watched_archives(&self) -> Result<Vec<FoundArchive>> {
        let seen: BTreeSet<String> = self.seen()?.into_iter().collect();
        Ok(self
            .archives_in_watch_dir()?
            .into_iter()
            .filter(|a| !seen.contains(&a.key))
            .collect())
    }
}

/// Of `now`, the archives whose size was the same at the previous look —
/// finished downloads. `previous` maps path to size.
pub fn settled(previous: &BTreeMap<String, u64>, now: &[FoundArchive]) -> Vec<FoundArchive> {
    now.iter()
        .filter(|a| a.bytes > 0 && previous.get(&a.path) == Some(&a.bytes))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, bytes: u64) -> FoundArchive {
        FoundArchive {
            key: format!("{path}|{bytes}"),
            path: path.into(),
            name: path.into(),
            bytes,
        }
    }

    #[test]
    fn an_archive_is_ready_once_its_size_holds_still() {
        let first = [found("a.zip", 10), found("b.7z", 0)];
        let previous: BTreeMap<String, u64> =
            first.iter().map(|a| (a.path.clone(), a.bytes)).collect();
        let now = [found("a.zip", 10), found("b.7z", 0), found("c.rar", 5)];
        let ready: Vec<String> = settled(&previous, &now)
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(ready, ["a.zip"], "empty and brand new files wait");
    }
}
