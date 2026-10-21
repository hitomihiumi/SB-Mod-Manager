//! Backups of the game's saves.
//!
//! A mod that misbehaves can corrupt a save, and a save made with a mod can
//! refuse to load without it. Stellar Blade keeps its saves in
//! `%LOCALAPPDATA%\SB\Saved\SaveGames`; the manager zips that folder before
//! each launch (when something in it changed) and on request, and puts a
//! chosen backup back — after backing up what is there, so a restore can be
//! undone too.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::dto::{SaveBackupView, SavesView};
use crate::{App, AppError, Result};

const SETTING_SAVES_DIR: &str = "savesDir";
const SETTING_BACKUP_ON_LAUNCH: &str = "backupSavesOnLaunch";
/// Automatic backups kept; the user's own and pre-restore ones are not pruned.
const KEEP_AUTOMATIC: usize = 20;

/// Why a backup was made, which is also the last part of its file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// Before a launch.
    Automatic,
    /// Asked for.
    Manual,
    /// What was there before a backup was restored over it.
    BeforeRestore,
}

impl BackupKind {
    fn as_str(self) -> &'static str {
        match self {
            BackupKind::Automatic => "auto",
            BackupKind::Manual => "manual",
            BackupKind::BeforeRestore => "before-restore",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "auto" => Some(BackupKind::Automatic),
            "manual" => Some(BackupKind::Manual),
            "before-restore" => Some(BackupKind::BeforeRestore),
            _ => None,
        }
    }
}

impl App {
    /// Where the game keeps its saves: the user's choice, or where Stellar
    /// Blade puts them.
    pub fn saves_dir(&self) -> Result<Option<PathBuf>> {
        if let Some(dir) = self.store.get_setting(SETTING_SAVES_DIR)? {
            return Ok(Some(PathBuf::from(dir)));
        }
        Ok(std::env::var_os("LOCALAPPDATA").map(|local| {
            PathBuf::from(local)
                .join("SB")
                .join("Saved")
                .join("SaveGames")
        }))
    }

    /// Point the backups at another folder, or back at the default with `None`.
    pub fn set_saves_dir(&mut self, dir: Option<&Path>) -> Result<()> {
        match dir {
            Some(dir) => self
                .store
                .set_setting(SETTING_SAVES_DIR, &dir.to_string_lossy())?,
            None => self.store.delete_setting(SETTING_SAVES_DIR)?,
        }
        Ok(())
    }

    pub fn backup_saves_on_launch(&self) -> Result<bool> {
        Ok(self
            .store
            .get_setting(SETTING_BACKUP_ON_LAUNCH)?
            .is_none_or(|v| v == "1"))
    }

    pub fn set_backup_saves_on_launch(&mut self, on: bool) -> Result<()> {
        self.store
            .set_setting(SETTING_BACKUP_ON_LAUNCH, if on { "1" } else { "0" })?;
        Ok(())
    }

    pub fn save_backups_dir(&self) -> PathBuf {
        self.library_root().join("save-backups")
    }

    pub fn saves(&self) -> Result<SavesView> {
        let dir = self.saves_dir()?;
        let files = dir.as_deref().map(save_files).unwrap_or_default();
        Ok(SavesView {
            exists: dir.as_deref().is_some_and(Path::is_dir),
            files: files.len(),
            dir: dir.map(|d| d.to_string_lossy().into_owned()),
            custom_dir: self.store.get_setting(SETTING_SAVES_DIR)?.is_some(),
            backup_on_launch: self.backup_saves_on_launch()?,
            backups: self.save_backups()?,
        })
    }

    /// Newest first.
    fn save_backups(&self) -> Result<Vec<SaveBackupView>> {
        let dir = self.save_backups_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut backups: Vec<SaveBackupView> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let (created_at, kind) = parse_name(&name)?;
                Some(SaveBackupView {
                    bytes: entry.metadata().map(|m| m.len()).unwrap_or(0),
                    file: name,
                    created_at,
                    kind: kind.as_str().to_string(),
                })
            })
            .collect();
        backups.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        Ok(backups)
    }

    /// Zip the saves folder. An automatic backup is skipped (`None`) when
    /// nothing changed since the newest backup.
    pub fn back_up_saves(&self, kind: BackupKind) -> Result<Option<SaveBackupView>> {
        let dir = self
            .saves_dir()?
            .ok_or_else(|| AppError::Invalid("choose the folder Stellar Blade saves to".into()))?;
        let files = save_files(&dir);
        if files.is_empty() {
            return Err(AppError::Invalid(format!(
                "there are no saves in {}",
                dir.display()
            )));
        }
        let fingerprint = fingerprint(&dir, &files);
        let backups = self.save_backups_dir();
        if kind != BackupKind::Manual {
            let newest = self.save_backups()?.into_iter().next();
            if newest
                .is_some_and(|b| zip_comment(&backups.join(&b.file)) == Some(fingerprint.clone()))
            {
                return Ok(None);
            }
        }

        std::fs::create_dir_all(&backups).map_err(|e| AppError::io(&backups, e))?;
        // Backups are told apart, and ordered, by their second.
        let taken: Vec<u64> = self.save_backups()?.iter().map(|b| b.created_at).collect();
        let mut created_at = now().max(taken.iter().max().map_or(0, |t| t + 1));
        while taken.contains(&created_at) {
            created_at += 1;
        }
        let path = backups.join(file_name(created_at, kind));
        write_zip(&path, &dir, &files, &fingerprint)?;

        if kind == BackupKind::Automatic {
            for old in self
                .save_backups()?
                .into_iter()
                .filter(|b| b.kind == BackupKind::Automatic.as_str())
                .skip(KEEP_AUTOMATIC)
            {
                let _ = std::fs::remove_file(backups.join(old.file));
            }
        }
        Ok(self
            .save_backups()?
            .into_iter()
            .find(|b| b.created_at == created_at))
    }

    /// Before a launch, if the user wants it. A failure is logged, never in
    /// the way of starting the game.
    pub fn back_up_saves_before_launch(&self) {
        if !self.backup_saves_on_launch().unwrap_or(false) {
            return;
        }
        let has_saves = self
            .saves_dir()
            .ok()
            .flatten()
            .is_some_and(|dir| !save_files(&dir).is_empty());
        if has_saves {
            if let Err(error) = self.back_up_saves(BackupKind::Automatic) {
                tracing::warn!(%error, "could not back up saves before launch");
            }
        }
    }

    /// Put a backup's saves in place of the current ones, which are backed
    /// up first. The game must not be running, or it will write its own
    /// saves back over them.
    pub fn restore_saves(&self, file: &str) -> Result<()> {
        let backup = self.save_backup_path(file)?;
        let dir = self
            .saves_dir()?
            .ok_or_else(|| AppError::Invalid("choose the folder Stellar Blade saves to".into()))?;
        if !save_files(&dir).is_empty() {
            self.back_up_saves(BackupKind::BeforeRestore)?;
        }
        // Extract beside the folder first, so a broken zip leaves the
        // current saves alone.
        let incoming = dir.with_extension("sbmm-restore");
        let _ = std::fs::remove_dir_all(&incoming);
        sbmm_archive::extract(&backup, &incoming)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
        }
        std::fs::rename(&incoming, &dir).map_err(|e| AppError::io(&dir, e))?;
        Ok(())
    }

    pub fn delete_save_backup(&self, file: &str) -> Result<()> {
        let path = self.save_backup_path(file)?;
        std::fs::remove_file(&path).map_err(|e| AppError::io(&path, e))
    }

    /// A backup by file name, refusing anything that is not one of ours.
    fn save_backup_path(&self, file: &str) -> Result<PathBuf> {
        if file.contains(['/', '\\']) || parse_name(file).is_none() {
            return Err(AppError::Invalid(format!("{file} is not a save backup")));
        }
        let path = self.save_backups_dir().join(file);
        if !path.is_file() {
            return Err(AppError::Invalid(format!("{file} no longer exists")));
        }
        Ok(path)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `saves-<unix seconds>-<kind>.zip`; seconds sort in time order for the
/// next 250 years.
fn file_name(created_at: u64, kind: BackupKind) -> String {
    format!("saves-{created_at:010}-{}.zip", kind.as_str())
}

fn parse_name(name: &str) -> Option<(u64, BackupKind)> {
    let rest = name.strip_prefix("saves-")?.strip_suffix(".zip")?;
    let (secs, kind) = rest.split_once('-')?;
    Some((secs.parse().ok()?, BackupKind::parse(kind)?))
}

/// Every file in the saves folder, relative to it.
fn save_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = sbmm_archive::walk_relative(dir);
    files.sort();
    files
}

/// Names, sizes and modification times — enough to notice any save written
/// since — hashed with FNV-1a, which stays the same across builds.
fn fingerprint(dir: &Path, files: &[PathBuf]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for file in files {
        feed(file.to_string_lossy().as_bytes());
        if let Ok(meta) = std::fs::metadata(dir.join(file)) {
            feed(&meta.len().to_le_bytes());
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            feed(&modified.to_le_bytes());
        }
    }
    format!("sbmm-saves:{hash:016x}")
}

fn write_zip(path: &Path, dir: &Path, files: &[PathBuf], comment: &str) -> Result<()> {
    let zip_error =
        |e: zip::result::ZipError| AppError::Invalid(format!("{}: {e}", path.display()));
    let partial = path.with_extension("partial");
    let out = std::fs::File::create(&partial).map_err(|e| AppError::io(&partial, e))?;
    let mut zip = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default();
    for file in files {
        let name = file.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(dir.join(file)).map_err(|e| AppError::io(dir.join(file), e))?;
        zip.start_file(name, options).map_err(zip_error)?;
        zip.write_all(&bytes)
            .map_err(|e| AppError::io(&partial, e))?;
    }
    zip.set_comment(comment);
    zip.finish().map_err(zip_error)?;
    std::fs::rename(&partial, path).map_err(|e| AppError::io(path, e))
}

fn zip_comment(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let zip = zip::ZipArchive::new(file).ok()?;
    Some(String::from_utf8_lossy(zip.comment()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_names_carry_their_time_and_kind() {
        let name = file_name(1_760_000_000, BackupKind::BeforeRestore);
        assert_eq!(name, "saves-1760000000-before-restore.zip");
        assert_eq!(
            parse_name(&name),
            Some((1_760_000_000, BackupKind::BeforeRestore))
        );
        assert_eq!(parse_name("saves-12-other.zip"), None);
        assert_eq!(parse_name("notes.txt"), None);
    }
}
