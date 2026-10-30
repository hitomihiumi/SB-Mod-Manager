//! Unreal settings tweaks: mods that ship an `Engine.ini` or the like.
//!
//! The game reads its settings from `%LOCALAPPDATA%\SB\Saved\Config\
//! WindowsNoEditor`, outside its own folder, and those files are the user's
//! as much as any mod's. So a tweak is never copied over them: each of its
//! settings is merged in, and a ledger remembers what every key held before,
//! so disabling the mod puts exactly that back. A key the user or the game
//! has changed since is left as they left it.
//!
//! Every reconcile first takes all tweaks out (newest first), then puts the
//! enabled ones back in load order, so a later mod wins where two set the
//! same key — the same rule as for files.

use std::path::{Path, PathBuf};

use sbmm_core::ini;
use sbmm_core::model::ModType;
use serde::{Deserialize, Serialize};

use crate::config::{encode, read_text, Encoding};
use crate::{App, AppError, Result};

const SETTING_LEDGER: &str = "configTweaksApplied";
/// Where a packaged Unreal 4 game on Windows keeps its settings.
const PLATFORM_DIRS: &[&str] = &["WindowsNoEditor", "Windows"];

/// One change a tweak made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Change {
    mod_id: i64,
    /// File name, e.g. `Engine.ini`.
    file: String,
    section: String,
    key: String,
    /// What was set, to tell whether anyone changed it since.
    value: String,
    /// What the key held before; `None` when the tweak added it. Unused for
    /// array lines, which are added and removed whole.
    previous: Option<String>,
    /// An Unreal array entry (`+Paths=...`), added as a whole line.
    #[serde(default)]
    line: bool,
}

impl App {
    /// The folder the game reads its INI files from.
    fn tweak_dir(&self) -> Option<PathBuf> {
        let root = self.game_config_dir()?;
        Some(
            PLATFORM_DIRS
                .iter()
                .map(|d| root.join(d))
                .find(|d| d.is_dir())
                .unwrap_or_else(|| root.join(PLATFORM_DIRS[0])),
        )
    }

    fn ledger(&self) -> Result<Vec<Change>> {
        Ok(self
            .store
            .get_setting(SETTING_LEDGER)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    /// Mods whose tweaks are in the game's settings now.
    pub(crate) fn tweaked_mods(&self) -> Result<Vec<i64>> {
        let mut ids: Vec<i64> = self.ledger()?.iter().map(|c| c.mod_id).collect();
        ids.dedup();
        Ok(ids)
    }

    /// Bring the game's settings in line with the enabled tweaks.
    pub(crate) fn sync_config_tweaks(&mut self) -> Result<()> {
        let ledger = self.ledger()?;
        let records = self.store.list_mods()?;
        let mut wanted = Vec::new();
        for record in records.iter().filter(|r| r.enabled) {
            for component in self.store.components_for(record.id)? {
                if component.mod_type == ModType::ConfigTweak {
                    for file in component.files {
                        wanted.push((record.id, record.staging_folder.clone(), file));
                    }
                }
            }
        }
        if ledger.is_empty() && wanted.is_empty() {
            return Ok(());
        }
        let Some(dir) = self.tweak_dir() else {
            return Err(AppError::Invalid(
                "the game's settings folder could not be found".into(),
            ));
        };

        let mut files = Files::new(&dir);
        // Out, newest first, leaving alone what someone changed since.
        for change in ledger.iter().rev() {
            let text = files.text(&change.file)?;
            let restored = if change.line {
                ini::remove_line(&text, &change.section, &change.value)
            } else if ini::get(&text, &change.section, &change.key).as_deref()
                == Some(change.value.as_str())
            {
                match &change.previous {
                    Some(previous) => ini::set(&text, &change.section, &change.key, previous),
                    None => ini::remove(&text, &change.section, &change.key),
                }
            } else {
                text
            };
            files.put(&change.file, restored);
        }

        // In, in load order.
        let mut applied = Vec::new();
        for (mod_id, folder, file) in wanted {
            let source = self.staging_path(&folder).join(&file.source);
            let Ok((tweak, _)) = read_text(&source) else {
                continue;
            };
            let name = file
                .target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut text = files.text(&name)?;
            for (section, key, value) in ini::entries(&tweak) {
                if ini::is_array_op(&key) {
                    let line = format!("{key}={value}");
                    if !ini::has_line(&text, &section, &line) {
                        text = ini::add_line(&text, &section, &line);
                        applied.push(Change {
                            mod_id,
                            file: name.clone(),
                            section,
                            key,
                            value: line,
                            previous: None,
                            line: true,
                        });
                    }
                    continue;
                }
                let previous = ini::get(&text, &section, &key);
                if previous.as_deref() == Some(value.as_str()) {
                    continue;
                }
                text = ini::set(&text, &section, &key, &value);
                applied.push(Change {
                    mod_id,
                    file: name.clone(),
                    section,
                    key,
                    value,
                    previous,
                    line: false,
                });
            }
            files.put(&name, text);
        }

        files.save()?;
        self.store
            .set_setting(SETTING_LEDGER, &serde_json::to_string(&applied)?)?;
        Ok(())
    }
}

/// The game's INI files being changed, read once and written once.
struct Files {
    dir: PathBuf,
    open: Vec<(String, String, Option<Encoding>, bool)>,
}

impl Files {
    fn new(dir: &Path) -> Self {
        Files {
            dir: dir.to_path_buf(),
            open: Vec::new(),
        }
    }

    fn text(&mut self, name: &str) -> Result<String> {
        if let Some((_, text, _, _)) = self
            .open
            .iter()
            .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        {
            return Ok(text.clone());
        }
        let path = self.dir.join(name);
        let (text, encoding) = if path.exists() {
            let (text, encoding) = read_text(&path)?;
            (text, Some(encoding))
        } else {
            (String::new(), None)
        };
        self.open
            .push((name.to_string(), text.clone(), encoding, false));
        Ok(text)
    }

    fn put(&mut self, name: &str, text: String) {
        if let Some(entry) = self
            .open
            .iter_mut()
            .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        {
            if entry.1 != text {
                entry.1 = text;
                entry.3 = true;
            }
        }
    }

    fn save(self) -> Result<()> {
        for (name, text, encoding, changed) in self.open {
            if !changed {
                continue;
            }
            std::fs::create_dir_all(&self.dir).map_err(|e| AppError::io(&self.dir, e))?;
            let path = self.dir.join(name);
            let bytes = encode(&text, encoding.unwrap_or(Encoding::Utf8 { bom: false }));
            std::fs::write(&path, bytes).map_err(|e| AppError::io(&path, e))?;
        }
        Ok(())
    }
}
