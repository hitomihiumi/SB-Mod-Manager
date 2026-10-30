//! Settings files: a mod's own, and the game's.
//!
//! A mod's files are edited in its library folder. Deployed through a hard
//! link, the game's copy is the same file and changes with it; a copied one
//! (library on another drive) is replaced afterwards. The game's own INI
//! files live in `%LOCALAPPDATA%\SB\Saved\Config`; the first time one is
//! saved here, the version found is kept so it can be put back.
//!
//! Each mod's settings files are also kept as installed ("pristine"). That
//! is what "reset" goes back to, and what lets a reinstall or update carry
//! the user's edits over without also carrying defaults the author changed
//! — see [`sbmm_core::ini::merge`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dto::{ConfigFileView, ConfigText};
use crate::{App, AppError, Result};

/// What can be opened in the editor.
const EXTENSIONS: &[&str] = &[
    "ini", "cfg", "conf", "txt", "json", "toml", "yaml", "yml", "xml", "lua",
];
/// Where the game's settings are, when not where Stellar Blade keeps them.
const SETTING_GAME_CONFIG_DIR: &str = "gameConfigDir";
/// Anything larger is not a settings file.
const MAX_BYTES: u64 = 1024 * 1024;

/// Whose settings files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ConfigScope {
    #[serde(rename_all = "camelCase")]
    Mod {
        mod_id: i64,
    },
    Game,
    /// INI files in the game folder itself, such as ReShade presets the
    /// user made there.
    Install,
}

impl ConfigScope {
    /// The game's own files: INI only, and the found copy kept to reset to.
    fn is_game(self) -> bool {
        matches!(self, ConfigScope::Game | ConfigScope::Install)
    }
}

/// How a file was encoded, so it is written back the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Encoding {
    Utf8 { bom: bool },
    Utf16Le,
}

fn decode(bytes: &[u8]) -> Option<(String, Encoding)> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        return String::from_utf16(&units)
            .ok()
            .map(|text| (text, Encoding::Utf16Le));
    }
    let (bom, body) = match bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        Some(body) => (true, body),
        None => (false, bytes),
    };
    String::from_utf8(body.to_vec())
        .ok()
        .map(|text| (text, Encoding::Utf8 { bom }))
}

pub(crate) fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 { bom } => {
            let mut out = if bom {
                vec![0xEF, 0xBB, 0xBF]
            } else {
                Vec::new()
            };
            out.extend_from_slice(text.as_bytes());
            out
        }
        Encoding::Utf16Le => {
            let mut out = vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
    }
}

pub(crate) fn read_text(path: &Path) -> Result<(String, Encoding)> {
    let bytes = std::fs::read(path).map_err(|e| AppError::io(path, e))?;
    decode(&bytes)
        .ok_or_else(|| AppError::Invalid(format!("{} is not a text file", path.display())))
}

fn is_settings_file(path: &Path, only_ini: bool) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if only_ini {
        ext == "ini"
    } else {
        EXTENSIONS.contains(&ext.as_str())
    }
}

/// Settings files under `root`, `/`-separated and sorted.
fn settings_files(root: &Path, only_ini: bool) -> Vec<String> {
    let mut files: Vec<String> = sbmm_archive::walk_relative(root)
        .into_iter()
        .filter(|rel| is_settings_file(rel, only_ini))
        .filter(|rel| std::fs::metadata(root.join(rel)).is_ok_and(|m| m.len() <= MAX_BYTES))
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    files
}

/// A mod's settings as they were before a reinstall replaced them.
pub(crate) struct KeptSettings {
    files: Vec<(String, String, Option<String>)>,
}

impl App {
    /// The game's own settings: `%LOCALAPPDATA%\SB\Saved\Config`.
    pub fn game_config_dir(&self) -> Option<PathBuf> {
        if let Some(dir) = self
            .store
            .get_setting(SETTING_GAME_CONFIG_DIR)
            .ok()
            .flatten()
        {
            return Some(PathBuf::from(dir));
        }
        std::env::var_os("LOCALAPPDATA")
            .map(|local| PathBuf::from(local).join("SB").join("Saved").join("Config"))
    }

    /// Use another folder for the game's settings, or the usual one with
    /// `None`.
    pub fn set_game_config_dir(&mut self, dir: Option<&Path>) -> Result<()> {
        match dir {
            Some(dir) => self
                .store
                .set_setting(SETTING_GAME_CONFIG_DIR, &dir.to_string_lossy())?,
            None => self.store.delete_setting(SETTING_GAME_CONFIG_DIR)?,
        }
        Ok(())
    }

    fn pristine_dir(&self, staging_folder: &str) -> PathBuf {
        self.library_root().join("pristine").join(staging_folder)
    }

    fn config_backup_dir(&self) -> PathBuf {
        self.library_root().join("config-backups")
    }

    fn config_root(&self, scope: ConfigScope) -> Result<PathBuf> {
        match scope {
            ConfigScope::Mod { mod_id } => {
                Ok(self.staging_path(&self.record(mod_id)?.staging_folder))
            }
            ConfigScope::Game => self.game_config_dir().ok_or_else(|| {
                AppError::Invalid("the game's settings folder could not be found".into())
            }),
            ConfigScope::Install => Ok(self.game()?.ok_or(AppError::NoGame)?.root),
        }
    }

    /// Where the untouched copy of a file is kept, if anywhere.
    fn original_of(&self, scope: ConfigScope, path: &str) -> Result<PathBuf> {
        Ok(match scope {
            ConfigScope::Mod { mod_id } => self
                .pristine_dir(&self.record(mod_id)?.staging_folder)
                .join(path),
            ConfigScope::Game => self
                .config_backup_dir()
                .join(format!("{}.original", path.replace('/', "__"))),
            ConfigScope::Install => self
                .config_backup_dir()
                .join("install")
                .join(format!("{}.original", path.replace('/', "__"))),
        })
    }

    /// A settings file of `scope` by its relative path, refusing anything
    /// outside it or not a settings file.
    fn config_path(&self, scope: ConfigScope, path: &str) -> Result<PathBuf> {
        let root = self.config_root(scope)?;
        let full = sbmm_deploy::backend::resolve_target(&root, Path::new(path))
            .map_err(|_| AppError::Invalid(format!("{path} is outside the folder")))?;
        if !is_settings_file(&full, scope.is_game()) || !full.is_file() {
            return Err(AppError::Invalid(format!("{path} is not a settings file")));
        }
        Ok(full)
    }

    pub fn config_files(&self, scope: ConfigScope) -> Result<Vec<ConfigFileView>> {
        if scope == ConfigScope::Install {
            // The whole game folder is no list to pick from; its files are
            // opened from where they are shown.
            return Ok(Vec::new());
        }
        let root = self.config_root(scope)?;
        let mut views = Vec::new();
        for path in settings_files(&root, scope.is_game()) {
            let full = root.join(&path);
            let original = self.original_of(scope, &path)?;
            let edited =
                original.is_file() && std::fs::read(&original).ok() != std::fs::read(&full).ok();
            views.push(ConfigFileView {
                bytes: std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0),
                path,
                edited,
            });
        }
        Ok(views)
    }

    pub fn read_config(&self, scope: ConfigScope, path: &str) -> Result<ConfigText> {
        let full = self.config_path(scope, path)?;
        let (text, _) = read_text(&full)?;
        let original = self.original_of(scope, path)?;
        Ok(ConfigText {
            text,
            original: original
                .is_file()
                .then(|| read_text(&original).ok().map(|(t, _)| t))
                .flatten(),
        })
    }

    /// Save a settings file, in the encoding it already had.
    pub fn write_config(&mut self, scope: ConfigScope, path: &str, text: &str) -> Result<()> {
        let full = self.config_path(scope, path)?;
        let (_, encoding) = read_text(&full)?;

        if scope.is_game() {
            // The first save keeps what was there, to reset to.
            let original = self.original_of(scope, path)?;
            if !original.exists() {
                let dir = original
                    .parent()
                    .unwrap_or(&self.config_backup_dir())
                    .to_path_buf();
                std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
                std::fs::copy(&full, &original).map_err(|e| AppError::io(&original, e))?;
            }
        }

        // Written in place, so a hard link into the game folder stays one.
        std::fs::write(&full, encode(text, encoding)).map_err(|e| AppError::io(&full, e))?;

        if let ConfigScope::Mod { mod_id } = scope {
            self.refresh_deployed(mod_id, path)?;
        }
        Ok(())
    }

    /// Bring the game's copy of one of a mod's files in line with the mod.
    fn refresh_deployed(&mut self, mod_id: i64, path: &str) -> Result<()> {
        let rows = self.store.deployed_files_for(mod_id)?;
        if rows.is_empty() || self.game()?.is_none() {
            return Ok(());
        }
        let ctx = self.deploy_context()?;
        let record = self.record(mod_id)?;
        let components = self.store.components_for(mod_id)?;
        let plan = self.deploy_plan_for(&record, &components)?;
        let targets: Vec<PathBuf> = plan
            .files
            .iter()
            .filter(|f| f.source.to_string_lossy().replace('\\', "/") == path)
            .map(|f| f.target.clone())
            .collect();
        if targets.is_empty() {
            return Ok(());
        }

        let mut updated = Vec::with_capacity(rows.len());
        for (target, backup, size) in rows {
            let file = sbmm_deploy::DeployedFile {
                target: PathBuf::from(&target),
                mod_id,
                backup: backup.as_ref().map(PathBuf::from),
                size: size as u64,
            };
            let file = if targets.contains(&file.target) {
                sbmm_deploy::refresh(&ctx, &plan, &file)?
            } else {
                file
            };
            updated.push((target, backup, file.size as i64));
        }
        self.store.replace_deployment(mod_id, &updated)?;
        Ok(())
    }

    // -- carrying settings across a reinstall -------------------------------

    /// Keep a copy of a newly installed mod's settings files as they came,
    /// unless one is kept already.
    pub(crate) fn keep_pristine(&self, staging_folder: &str) -> Result<()> {
        let root = self.staging_path(staging_folder);
        let pristine = self.pristine_dir(staging_folder);
        for path in settings_files(&root, false) {
            let to = pristine.join(&path);
            if to.exists() {
                continue;
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            std::fs::copy(root.join(&path), &to).map_err(|e| AppError::io(&to, e))?;
        }
        Ok(())
    }

    pub(crate) fn forget_pristine(&self, staging_folder: &str) {
        let _ = std::fs::remove_dir_all(self.pristine_dir(staging_folder));
    }

    /// A mod's INI files as the user has them, with the copies they were
    /// installed as, read before the mod is replaced.
    pub(crate) fn keep_settings(&self, staging_folder: &str) -> KeptSettings {
        let root = self.staging_path(staging_folder);
        let pristine = self.pristine_dir(staging_folder);
        let files = settings_files(&root, true)
            .into_iter()
            .filter_map(|path| {
                let (user, _) = read_text(&root.join(&path)).ok()?;
                let original = read_text(&pristine.join(&path)).ok().map(|(t, _)| t);
                Some((path, user, original))
            })
            .collect();
        KeptSettings { files }
    }

    /// Put kept settings into the same files of a new install, over its
    /// defaults. The new install's own copies are kept as its pristine ones
    /// first. Returns how many values were carried.
    pub(crate) fn restore_settings(
        &self,
        staging_folder: &str,
        kept: &KeptSettings,
    ) -> Result<usize> {
        self.keep_pristine(staging_folder)?;
        let root = self.staging_path(staging_folder);
        let fresh_files = settings_files(&root, true);
        let mut carried = 0;
        for (path, user, original) in &kept.files {
            let Some(fresh_path) = fresh_files
                .iter()
                .find(|f| f.eq_ignore_ascii_case(path))
                .map(|f| root.join(f))
            else {
                continue;
            };
            let (fresh, encoding) = read_text(&fresh_path)?;
            let merged = sbmm_core::ini::merge(original.as_deref(), user, &fresh);
            if merged.text != fresh {
                std::fs::write(&fresh_path, encode(&merged.text, encoding))
                    .map_err(|e| AppError::io(&fresh_path, e))?;
            }
            carried += merged.carried;
        }
        Ok(carried)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_written_back_in_their_own_encoding() {
        for encoding in [
            Encoding::Utf8 { bom: false },
            Encoding::Utf8 { bom: true },
            Encoding::Utf16Le,
        ] {
            let bytes = encode("[Общие]\r\nKey=1\r\n", encoding);
            assert_eq!(
                decode(&bytes),
                Some(("[Общие]\r\nKey=1\r\n".to_string(), encoding))
            );
        }
        assert_eq!(decode(&[0xFF, 0x00, 0xC3]), None);
    }
}
