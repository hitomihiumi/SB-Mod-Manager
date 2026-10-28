//! ReShade presets: which there are, which one ReShade starts with, and
//! which the user wants for each profile.
//!
//! Everything ReShade reads sits beside the executable, which is never
//! virtual — the manager deploys it as real files in both modes — so the
//! game folder on disk is what ReShade sees. The one exception is a file
//! ReShade itself created while the game ran under the virtual file system:
//! that lands in the overwrite folder, which sits on top.
//!
//! The manager never decides what a preset looks like. It reads presets to
//! show what they switch on and which of their effect files are missing, and
//! writes exactly one line of `ReShade.ini`: `PresetPath`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use sbmm_core::model::ModType;
use sbmm_core::paths::{BINARIES_WIN64, PROXY_DLLS, RESHADE_PRESETS};
use sbmm_core::reshade::{self, CONFIG_FILE};

use crate::config::{encode, read_text, ConfigScope};
use crate::dto::{DeploymentMode, ReshadeInstall, ReshadePreset, ReshadeView};
use crate::{comparable_path, path_string, App, AppError, Result};

/// Where ReShade is published.
pub const DOWNLOAD_URL: &str = "https://reshade.me";

/// The preset a profile asks for, by profile id.
const SETTING_PROFILE_PRESET: &str = "reshadePreset";
/// Folders beside the executable that never hold presets and can be large.
const NOT_PRESETS: &[&str] = &["ue4ss", "reshade-shaders", "plugins"];
/// Anything larger is not a preset.
const MAX_PRESET_BYTES: u64 = 1024 * 1024;

/// A file ReShade can see, and who put it there.
struct Seen {
    /// Game-relative, `/`-separated, in its own spelling.
    path: String,
    /// The real file.
    real: PathBuf,
    origin: &'static str,
    mod_id: Option<i64>,
}

impl App {
    fn profile_preset_key(profile_id: i64) -> String {
        format!("{SETTING_PROFILE_PRESET}:{profile_id}")
    }

    /// Who deployed each file, by comparable game-relative path.
    fn deployed_owners(&self) -> Result<HashMap<String, i64>> {
        Ok(self
            .store
            .deployed_files()?
            .into_iter()
            .map(|(mod_id, target, _, _)| (comparable_path(&target), mod_id))
            .collect())
    }

    /// Every file under `dir` (game-relative) that ReShade would see, minus
    /// the folders in `skip`. Overwrite wins over the game folder, as it does
    /// for the game.
    fn reshade_files(&self, game_root: &Path, dir: &str, skip: &[&str]) -> Result<Vec<Seen>> {
        let owners = self.deployed_owners()?;
        let mut seen: Vec<Seen> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut roots = vec![(game_root.to_path_buf(), "game")];
        if self.deployment_mode()? == DeploymentMode::Virtual {
            roots.push((self.overwrite_dir(), "overwrite"));
        }
        for (root, origin) in roots {
            let mut found = Vec::new();
            walk(&root.join(dir), dir, skip, 6, &mut found);
            for (path, real) in found {
                let key = comparable_path(&path);
                let mod_id = (origin == "game")
                    .then(|| owners.get(&key).copied())
                    .flatten();
                let file = Seen {
                    path,
                    real,
                    origin: if mod_id.is_some() { "mod" } else { origin },
                    mod_id,
                };
                match index.get(&key) {
                    Some(&at) => seen[at] = file,
                    None => {
                        index.insert(key, seen.len());
                        seen.push(file);
                    }
                }
            }
        }
        Ok(seen)
    }

    /// `ReShade.ini` as ReShade would find it, if it is anywhere.
    fn reshade_config(&self, game_root: &Path) -> Result<Option<Seen>> {
        Ok(self
            .reshade_files(game_root, BINARIES_WIN64, &NOT_PRESETS_AND_SUBFOLDERS)?
            .into_iter()
            .find(|f| {
                f.path
                    .rsplit('/')
                    .next()
                    .is_some_and(|n| n.eq_ignore_ascii_case(CONFIG_FILE))
            }))
    }

    /// The DLL that loads ReShade, when there is one.
    fn reshade_dll(&self, game_root: &Path) -> Result<Option<Seen>> {
        Ok(self
            .reshade_files(game_root, BINARIES_WIN64, &NOT_PRESETS_AND_SUBFOLDERS)?
            .into_iter()
            .filter(|f| {
                let name = f
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                PROXY_DLLS.contains(&name.as_str())
            })
            .find(|f| is_reshade_dll(&f.real)))
    }

    /// Whether ReShade is beside the executable — read cheaply, for the mod
    /// list: a proxy DLL and ReShade's settings file together.
    pub(crate) fn reshade_likely_installed(&self) -> Result<bool> {
        let Some(game) = self.game()? else {
            return Ok(true);
        };
        let files = self.reshade_files(&game.root, BINARIES_WIN64, &NOT_PRESETS_AND_SUBFOLDERS)?;
        let named = |wanted: &dyn Fn(&str) -> bool| {
            files.iter().any(|f| {
                let name = f
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                wanted(&name)
            })
        };
        Ok(
            named(&|n| n == CONFIG_FILE.to_ascii_lowercase())
                && named(&|n| PROXY_DLLS.contains(&n)),
        )
    }

    /// ReShade at a glance: whether it is there, its presets, and which one
    /// it starts with.
    pub fn reshade_view(&self) -> Result<ReshadeView> {
        let profile = self.store.active_profile()?;
        let profile_preset = self.store.get_setting(&Self::profile_preset_key(profile))?;
        let Some(game) = self.game()? else {
            return Ok(ReshadeView {
                download_url: DOWNLOAD_URL.into(),
                profile_preset,
                ..ReshadeView::default()
            });
        };
        let names: HashMap<i64, String> = self
            .store
            .list_mods()?
            .into_iter()
            .map(|m| (m.id, m.name))
            .collect();
        let provider = |file: &Seen| match file.mod_id {
            Some(id) => names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| format!("mod {id}")),
            None if file.origin == "overwrite" => "Overwrite".to_string(),
            None => "Game folder".to_string(),
        };

        let installed = self.reshade_dll(&game.root)?.map(|dll| ReshadeInstall {
            dll: dll.path.rsplit('/').next().unwrap_or_default().to_string(),
            version: sbmm_upscaler::pe::file_version_opt(&dll.real).map(|v| v.to_string()),
            provided_by: provider(&dll),
            mod_id: dll.mod_id,
        });

        let config_file = self.reshade_config(&game.root)?;
        let config = match &config_file {
            Some(file) => reshade::read_config(&read_text(&file.real)?.0),
            None => reshade::read_config(""),
        };
        let active = config
            .preset_path
            .as_deref()
            .map(|value| reshade::game_relative(BINARIES_WIN64, value).unwrap_or(value.into()));

        let available = self.available_effects(&game.root, &config.effect_paths)?;

        let mut presets = Vec::new();
        for file in self.reshade_files(&game.root, BINARIES_WIN64, NOT_PRESETS)? {
            let is_ini = file.path.to_ascii_lowercase().ends_with(".ini");
            let is_config = file
                .path
                .rsplit('/')
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case(CONFIG_FILE));
            if !is_ini || is_config {
                continue;
            }
            let bytes = std::fs::metadata(&file.real).map(|m| m.len()).unwrap_or(0);
            if bytes > MAX_PRESET_BYTES {
                continue;
            }
            let Some(preset) = read_text(&file.real)
                .ok()
                .and_then(|(text, _)| reshade::read_preset(&text))
            else {
                continue;
            };
            let is_active = active
                .as_deref()
                .is_some_and(|a| comparable_path(a) == comparable_path(&file.path));
            presets.push(ReshadePreset {
                name: stem(&file.path),
                folder: folder_label(&file.path),
                techniques: preset.techniques.iter().map(|t| t.name.clone()).collect(),
                missing_effects: preset.missing(&available),
                unchecked: preset.unchecked() as u32,
                active: is_active,
                bytes,
                provided_by: provider(&file),
                origin: file.origin.to_string(),
                mod_id: file.mod_id,
                source: self.preset_source(&file)?,
                path: file.path,
            });
        }
        presets.sort_by_key(|p| (p.folder.to_lowercase(), p.name.to_lowercase()));

        let active_found = active.as_deref().is_none_or(|a| {
            presets
                .iter()
                .any(|p| comparable_path(&p.path) == comparable_path(a))
        });
        Ok(ReshadeView {
            installed,
            config: config_file.as_ref().map(|f| f.path.clone()),
            config_provided_by: config_file.as_ref().map(provider),
            active,
            active_found,
            presets,
            effects: available.len() as u32,
            profile_preset,
            download_url: DOWNLOAD_URL.into(),
        })
    }

    /// The effect files ReShade can find, lower-cased.
    fn available_effects(
        &self,
        game_root: &Path,
        search: &[reshade::SearchPath],
    ) -> Result<BTreeSet<String>> {
        let mut found = BTreeSet::new();
        for path in search {
            let files: Vec<PathBuf> = match reshade::game_relative(BINARIES_WIN64, &path.path) {
                Some(dir) => self
                    .reshade_files(game_root, &dir, &[])?
                    .into_iter()
                    .filter(|f| {
                        path.recursive || !f.path[dir.len()..].trim_start_matches('/').contains('/')
                    })
                    .map(|f| f.real)
                    .collect(),
                None if reshade::is_absolute(&path.path) => {
                    let mut out = Vec::new();
                    walk(
                        Path::new(&path.path),
                        "",
                        &[],
                        if path.recursive { 6 } else { 0 },
                        &mut out,
                    );
                    out.into_iter().map(|(_, real)| real).collect()
                }
                None => Vec::new(),
            };
            found.extend(files.iter().filter_map(|f| {
                let name = f.file_name()?.to_string_lossy().to_lowercase();
                name.ends_with(".fx").then_some(name)
            }));
        }
        Ok(found)
    }

    /// Where a preset can be edited: in its mod, when a mod put it there;
    /// otherwise in the game folder. `None` for one in overwrite.
    fn preset_source(&self, file: &Seen) -> Result<Option<crate::dto::PresetSource>> {
        Ok(match file.mod_id {
            Some(mod_id) => {
                self.mod_source_of(mod_id, &file.path)?
                    .map(|path| crate::dto::PresetSource {
                        scope: ConfigScope::Mod { mod_id },
                        path,
                    })
            }
            None if file.origin == "game" => Some(crate::dto::PresetSource {
                scope: ConfigScope::Install,
                path: file.path.clone(),
            }),
            None => None,
        })
    }

    /// The file in a mod's folder that is deployed to `target`.
    fn mod_source_of(&self, mod_id: i64, target: &str) -> Result<Option<String>> {
        let record = self.record(mod_id)?;
        let components = self.store.components_for(mod_id)?;
        let wanted = comparable_path(target);
        Ok(self
            .plan_for(&record, &components)
            .files
            .iter()
            .find(|f| comparable_path(&f.target.to_string_lossy()) == wanted)
            .map(|f| f.source.to_string_lossy().replace('\\', "/")))
    }

    /// Have ReShade start with the preset at `path` (game-relative).
    ///
    /// Only `PresetPath` changes. A `ReShade.ini` that came with a mod is
    /// changed in the mod, so the choice survives a redeploy; one ReShade's
    /// installer wrote is changed where it is. With no `ReShade.ini` yet —
    /// ReShade writes it the first time it runs — one is started with just
    /// this line, which ReShade fills in around it.
    pub fn set_reshade_preset(&mut self, path: &str) -> Result<()> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let presets = self.reshade_files(&game.root, BINARIES_WIN64, NOT_PRESETS)?;
        let preset = presets
            .iter()
            .find(|f| comparable_path(&f.path) == comparable_path(path))
            .ok_or_else(|| AppError::Invalid(format!("there is no preset at {path}")))?;
        let value = reshade::relative_value(BINARIES_WIN64, &preset.path)
            .unwrap_or_else(|| path_string(&preset.real));

        match self.reshade_config(&game.root)? {
            Some(config) => {
                let (text, encoding) = read_text(&config.real)?;
                if reshade::read_config(&text).preset_path.as_deref() == Some(value.as_str()) {
                    return Ok(());
                }
                let text = reshade::with_preset_path(&text, &value);
                match config.mod_id {
                    Some(mod_id) => match self.mod_source_of(mod_id, &config.path)? {
                        Some(source) => {
                            self.write_config(ConfigScope::Mod { mod_id }, &source, &text)?
                        }
                        None => write(&config.real, &encode(&text, encoding))?,
                    },
                    None => write(&config.real, &encode(&text, encoding))?,
                }
            }
            None => {
                if self.reshade_dll(&game.root)?.is_none() {
                    return Err(AppError::Invalid(
                        "ReShade is not installed, so there is nothing to start a preset".into(),
                    ));
                }
                let file = game.root.join(BINARIES_WIN64).join(CONFIG_FILE);
                let text = reshade::with_preset_path("", &value).replace('\n', "\r\n");
                write(&file, text.as_bytes())?;
            }
        }
        Ok(())
    }

    /// The preset a profile starts ReShade with, or none to leave ReShade's
    /// choice alone. Set for the active profile, it takes effect at once.
    pub fn set_profile_reshade_preset(
        &mut self,
        profile_id: i64,
        path: Option<&str>,
    ) -> Result<()> {
        let key = Self::profile_preset_key(profile_id);
        match path {
            Some(path) => {
                self.store.set_setting(&key, path)?;
                if profile_id == self.store.active_profile()? {
                    self.set_reshade_preset(path)?;
                }
            }
            None => self.store.delete_setting(&key)?,
        }
        Ok(())
    }

    /// Put the active profile's preset in place, if it has one and it is
    /// still there. Run on switching profile and before each launch. Returns
    /// a note for the user when the preset has gone.
    pub fn apply_profile_reshade_preset(&mut self) -> Result<Option<String>> {
        let profile = self.store.active_profile()?;
        let Some(path) = self.store.get_setting(&Self::profile_preset_key(profile))? else {
            return Ok(None);
        };
        let Some(game) = self.game()? else {
            return Ok(None);
        };
        let present = self
            .reshade_files(&game.root, BINARIES_WIN64, NOT_PRESETS)?
            .iter()
            .any(|f| comparable_path(&f.path) == comparable_path(&path));
        if !present {
            return Ok(Some(format!(
                "The ReShade preset this profile uses, {}, is not in the game folder any more.",
                stem(&path)
            )));
        }
        if self.reshade_config(&game.root)?.is_none() && self.reshade_dll(&game.root)?.is_none() {
            return Ok(None);
        }
        self.set_reshade_preset(&path)?;
        Ok(None)
    }

    /// A copy of a preset to change freely, in `reshade-presets` in the game
    /// folder: the way to adjust a mod's preset without changing the mod.
    /// Returns the new preset's path.
    pub fn duplicate_reshade_preset(&mut self, path: &str, name: &str) -> Result<String> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let from = self.own_or_any_preset(&game.root, path, false)?;
        let target = format!("{RESHADE_PRESETS}/{}.ini", preset_file_name(name)?);
        let full = game.root.join(&target);
        if full.exists() || self.preset_exists(&game.root, &target)? {
            return Err(AppError::Invalid(format!(
                "there is already a preset called “{}”",
                name.trim()
            )));
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }
        std::fs::copy(&from.real, &full).map_err(|e| AppError::io(&full, e))?;
        Ok(target)
    }

    /// Rename a preset of the user's own, keeping it ReShade's and any
    /// profile's choice if it was. Presets from a mod belong to the mod.
    pub fn rename_reshade_preset(&mut self, path: &str, name: &str) -> Result<String> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let from = self.own_or_any_preset(&game.root, path, true)?;
        let folder = from
            .path
            .rsplit_once('/')
            .map(|(f, _)| f)
            .unwrap_or_default();
        let target = format!("{folder}/{}.ini", preset_file_name(name)?);
        if comparable_path(&target) == comparable_path(&from.path) && target == from.path {
            return Ok(target);
        }
        let case_only = comparable_path(&target) == comparable_path(&from.path);
        if !case_only && self.preset_exists(&game.root, &target)? {
            return Err(AppError::Invalid(format!(
                "there is already a preset called “{}”",
                name.trim()
            )));
        }
        let was_active = self
            .reshade_active(&game.root)?
            .is_some_and(|a| comparable_path(&a) == comparable_path(&from.path));
        let full = game.root.join(&target);
        std::fs::rename(&from.real, &full).map_err(|e| AppError::io(&full, e))?;
        self.repoint_profiles(&from.path, Some(&target))?;
        if was_active {
            self.set_reshade_preset(&target)?;
        }
        Ok(target)
    }

    /// Delete a preset of the user's own. Not the one ReShade starts with:
    /// ReShade would quietly start a new, empty one in its place.
    pub fn delete_reshade_preset(&mut self, path: &str) -> Result<()> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let preset = self.own_or_any_preset(&game.root, path, true)?;
        if self
            .reshade_active(&game.root)?
            .is_some_and(|a| comparable_path(&a) == comparable_path(&preset.path))
        {
            return Err(AppError::Invalid(
                "ReShade starts with this preset; pick another one first".into(),
            ));
        }
        std::fs::remove_file(&preset.real).map_err(|e| AppError::io(&preset.real, e))?;
        self.repoint_profiles(&preset.path, None)?;
        Ok(())
    }

    /// The preset ReShade starts with, game-relative when it is in the game.
    fn reshade_active(&self, game_root: &Path) -> Result<Option<String>> {
        let Some(config) = self.reshade_config(game_root)? else {
            return Ok(None);
        };
        let config = reshade::read_config(&read_text(&config.real)?.0);
        Ok(config
            .preset_path
            .map(|value| reshade::game_relative(BINARIES_WIN64, &value).unwrap_or(value)))
    }

    fn preset_exists(&self, game_root: &Path, path: &str) -> Result<bool> {
        Ok(self
            .reshade_files(game_root, BINARIES_WIN64, NOT_PRESETS)?
            .iter()
            .any(|f| comparable_path(&f.path) == comparable_path(path)))
    }

    /// A preset by path; with `own_only`, refused unless it is the user's
    /// own file in the game folder.
    fn own_or_any_preset(&self, game_root: &Path, path: &str, own_only: bool) -> Result<Seen> {
        let file = self
            .reshade_files(game_root, BINARIES_WIN64, NOT_PRESETS)?
            .into_iter()
            .find(|f| comparable_path(&f.path) == comparable_path(path))
            .ok_or_else(|| AppError::Invalid(format!("there is no preset at {path}")))?;
        if own_only && file.origin != "game" {
            return Err(AppError::Invalid(if file.origin == "mod" {
                "this preset belongs to a mod; duplicate it to make one of your own".into()
            } else {
                "this preset is in the overwrite folder; make a mod of it first".into()
            }));
        }
        let is_preset = read_text(&file.real)
            .ok()
            .is_some_and(|(text, _)| reshade::is_preset(&text));
        if !is_preset {
            return Err(AppError::Invalid(format!("{path} is not a ReShade preset")));
        }
        Ok(file)
    }

    /// Profiles that used `from` use `to` now, or nothing.
    fn repoint_profiles(&mut self, from: &str, to: Option<&str>) -> Result<()> {
        for profile in self.store.list_profiles()? {
            let key = Self::profile_preset_key(profile.id);
            if self
                .store
                .get_setting(&key)?
                .is_some_and(|p| comparable_path(&p) == comparable_path(from))
            {
                match to {
                    Some(to) => self.store.set_setting(&key, to)?,
                    None => self.store.delete_setting(&key)?,
                }
            }
        }
        Ok(())
    }

    /// Enabled mods with presets, which do nothing without ReShade.
    pub(crate) fn needs_reshade(components: &[sbmm_core::DetectedComponent]) -> bool {
        components
            .iter()
            .any(|c| c.mod_type == ModType::ReshadePreset)
    }
}

/// Only the files directly beside the executable.
const NOT_PRESETS_AND_SUBFOLDERS: [&str; 1] = ["*"];

/// Files under `dir`, as `(game-relative path, real path)`. `skip` names
/// folders not to enter; `*` enters none. `depth` is how many folders down.
fn walk(dir: &Path, relative: &str, skip: &[&str], depth: usize, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                let skipped = skip
                    .iter()
                    .any(|s| *s == "*" || s.eq_ignore_ascii_case(&name));
                if depth > 0 && !skipped {
                    walk(&entry.path(), &child, skip, depth - 1, out);
                }
            }
            Ok(_) => out.push((child, entry.path())),
            Err(_) => {}
        }
    }
}

/// Whether a DLL is ReShade: its version information names it, or its
/// author.
fn is_reshade_dll(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let wide = |text: &str| -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() };
    [wide("ReShade"), wide("crosire")]
        .iter()
        .any(|needle| bytes.windows(needle.len()).any(|w| w == needle.as_slice()))
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|e| AppError::io(path, e))
}

fn stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.')
        .map(|(s, _)| s)
        .unwrap_or(name)
        .to_string()
}

/// The folder a preset is in, relative to the executable's: empty beside it.
fn folder_label(path: &str) -> String {
    let inside = path
        .get(BINARIES_WIN64.len()..)
        .filter(|_| comparable_path(path).starts_with(&comparable_path(BINARIES_WIN64)))
        .unwrap_or(path)
        .trim_start_matches('/');
    inside
        .rsplit_once('/')
        .map(|(f, _)| f.to_string())
        .unwrap_or_default()
}

/// A preset name as a file name, refusing what Windows would.
fn preset_file_name(name: &str) -> Result<String> {
    let name = name.trim().trim_end_matches(".ini").trim();
    let bad = |c: char| {
        matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()
    };
    if name.is_empty() || name.contains(bad) || name.ends_with('.') {
        return Err(AppError::Invalid(format!(
            "“{name}” cannot be a file name: avoid < > : \" / \\ | ? *"
        )));
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_shown_from_the_executable() {
        assert_eq!(
            folder_label("SB/Binaries/Win64/reshade-presets/A.ini"),
            "reshade-presets"
        );
        assert_eq!(folder_label("SB/Binaries/Win64/A.ini"), "");
        assert_eq!(stem("SB/Binaries/Win64/My.Preset.ini"), "My.Preset");
    }

    #[test]
    fn preset_names_must_be_file_names() {
        assert_eq!(preset_file_name(" Night.ini ").unwrap(), "Night");
        assert!(preset_file_name("a/b").is_err());
        assert!(preset_file_name("  ").is_err());
    }
}
