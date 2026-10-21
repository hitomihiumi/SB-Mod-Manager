//! Sharing a setup, and bringing mods up to date.
//!
//! A mod list is a file describing one profile: which mods, in what order,
//! which are enabled, where each came from, and the load order rules between
//! them. Importing one makes a new profile: mods already installed are put
//! in place, mods from Nexus that are not are queued for download, and
//! anything else is listed for the user to fetch.
//!
//! A download can arrive long after it was asked for, so what should happen
//! when it does is written down beforehand as an *expected install*, keyed by
//! its Nexus mod id: enable it in the imported profile, or replace the
//! version it updates. [`App::commit_install`] settles it whichever way the
//! archive finally gets installed.

use std::collections::BTreeMap;
use std::path::Path;

use sbmm_nexus::ModFile;
use serde::{Deserialize, Serialize};

use crate::dto::{ImportEntry, ImportOutcome, ImportPlan, UpdatableMod};
use crate::{newer_version, App, AppError, Result};

const FORMAT: &str = "sbmm-modlist";
const SETTING_EXPECTED: &str = "expectedInstalls";

/// A profile, written out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModList {
    pub format: String,
    pub version: u32,
    /// The profile it was exported from.
    pub profile: String,
    /// In load order.
    pub mods: Vec<ListedMod>,
    #[serde(default)]
    pub rules: Vec<ListedRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedMod {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    pub source: String,
    #[serde(default)]
    pub nexus_mod_id: Option<i64>,
    #[serde(default)]
    pub nexus_file_id: Option<i64>,
    pub enabled: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// `mods[before]` loads before `mods[after]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedRule {
    pub before: usize,
    pub after: usize,
}

/// What to do when a mod from Nexus arrives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expected {
    /// The installed version it updates, to be replaced.
    #[serde(default)]
    replaces: Option<i64>,
    /// `(profile, enabled, priority)` to give it, from an imported list.
    #[serde(default)]
    places: Vec<(i64, bool, i64)>,
}

/// The file that updates an installed one: the newest upload in the same
/// category (main file, optional file…), or the newest main file when the
/// installed file is no longer listed. `None` when the newest is installed.
pub fn pick_update(files: &[ModFile], installed_file_id: Option<u64>) -> Option<ModFile> {
    let category = installed_file_id
        .and_then(|id| files.iter().find(|f| f.file_id == id))
        .and_then(|f| f.category_name.clone())
        .filter(|c| !c.eq_ignore_ascii_case("OLD_VERSION"))
        .unwrap_or_else(|| "MAIN".to_string());
    let newest = files
        .iter()
        .filter(|f| {
            f.category_name
                .as_deref()
                .is_some_and(|c| c.eq_ignore_ascii_case(&category))
        })
        .max_by_key(|f| f.file_id)
        .or_else(|| {
            files
                .iter()
                .filter(|f| f.is_primary)
                .max_by_key(|f| f.file_id)
        })?;
    (Some(newest.file_id) != installed_file_id).then(|| newest.clone())
}

impl App {
    // -- export and import --------------------------------------------------

    /// Write the active profile as a mod list. Returns how many mods it holds.
    pub fn export_mod_list(&self, path: &Path) -> Result<usize> {
        let records = self.store.list_mods()?;
        let profile = self
            .store
            .list_profiles()?
            .into_iter()
            .find(|p| p.active)
            .map(|p| p.name)
            .unwrap_or_default();
        let index: BTreeMap<i64, usize> =
            records.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        let list = ModList {
            format: FORMAT.into(),
            version: 1,
            profile,
            mods: records
                .iter()
                .map(|r| ListedMod {
                    name: r.name.clone(),
                    version: r.version.clone(),
                    source: r.source.clone(),
                    nexus_mod_id: r.nexus_mod_id,
                    nexus_file_id: r.nexus_file_id,
                    enabled: r.enabled,
                    tags: r.tags.clone(),
                })
                .collect(),
            rules: self
                .store
                .order_rules()?
                .into_iter()
                .filter_map(|(_, before, after)| {
                    Some(ListedRule {
                        before: *index.get(&before)?,
                        after: *index.get(&after)?,
                    })
                })
                .collect(),
        };
        std::fs::write(path, serde_json::to_string_pretty(&list)?)
            .map_err(|e| AppError::io(path, e))?;
        Ok(list.mods.len())
    }

    fn read_mod_list(path: &Path) -> Result<ModList> {
        let text = std::fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;
        let list: ModList = serde_json::from_str(&text)
            .map_err(|_| AppError::Invalid(format!("{} is not a mod list", path.display())))?;
        if list.format != FORMAT {
            return Err(AppError::Invalid(format!(
                "{} is not a mod list",
                path.display()
            )));
        }
        if list.version > 1 {
            return Err(AppError::Invalid(
                "this mod list was written by a newer version of the manager".into(),
            ));
        }
        Ok(list)
    }

    /// What importing `path` would do, entry by entry.
    pub fn plan_import(&self, path: &Path) -> Result<ImportPlan> {
        let list = Self::read_mod_list(path)?;
        let records = self.store.list_mods()?;
        let entries = list
            .mods
            .iter()
            .map(|listed| {
                let installed =
                    records
                        .iter()
                        .find(|r| match (listed.nexus_mod_id, r.nexus_mod_id) {
                            (Some(a), Some(b)) => a == b,
                            _ => r.name.to_lowercase() == listed.name.to_lowercase(),
                        });
                ImportEntry {
                    name: listed.name.clone(),
                    version: listed.version.clone(),
                    enabled: listed.enabled,
                    status: match (installed, listed.nexus_mod_id) {
                        (Some(_), _) => "installed",
                        (None, Some(_)) => "download",
                        (None, None) => "manual",
                    }
                    .into(),
                    mod_id: installed.map(|r| r.id),
                    nexus_mod_id: listed.nexus_mod_id,
                    nexus_file_id: listed.nexus_file_id,
                }
            })
            .collect();
        Ok(ImportPlan {
            profile: list.profile,
            entries,
            rules: list.rules.len(),
        })
    }

    /// Import `path` as a new profile called `profile` and make it active.
    /// Mods to download are returned for the caller to queue; each is
    /// expected, and takes its place in the profile when it arrives.
    pub fn import_mod_list(&mut self, path: &Path, profile: &str) -> Result<ImportOutcome> {
        let list = Self::read_mod_list(path)?;
        let plan = self.plan_import(path)?;
        let name = self.valid_profile_name(profile, None)?;
        let profile_id = self.store.create_profile(&name, None, false)?;

        let priority = |index: usize| (index as i64 + 1) * 10;
        let mut matched = 0;
        let mut downloads = Vec::new();
        let mut manual = Vec::new();
        let mut placed: Vec<i64> = Vec::new();
        for (index, entry) in plan.entries.iter().enumerate() {
            match (entry.mod_id, entry.nexus_mod_id) {
                (Some(id), _) => {
                    self.store
                        .set_profile_state(profile_id, id, entry.enabled, priority(index))?;
                    placed.push(id);
                    matched += 1;
                }
                (None, Some(nexus)) => {
                    self.expect(nexus, |e| {
                        e.places.push((profile_id, entry.enabled, priority(index)))
                    })?;
                    downloads.push(entry.clone());
                }
                (None, None) => manual.push(entry.name.clone()),
            }
        }
        // Installed mods the list does not name stay off, after the rest.
        let rest: Vec<i64> = self
            .store
            .list_mods()?
            .into_iter()
            .map(|m| m.id)
            .filter(|id| !placed.contains(id))
            .collect();
        for (offset, id) in rest.into_iter().enumerate() {
            self.store.set_profile_state(
                profile_id,
                id,
                false,
                priority(list.mods.len() + offset),
            )?;
        }
        // Rules between mods that are both here; one that would contradict
        // the rules already set is skipped rather than failing the import.
        for rule in &list.rules {
            let ids = (
                plan.entries.get(rule.before).and_then(|e| e.mod_id),
                plan.entries.get(rule.after).and_then(|e| e.mod_id),
            );
            if let (Some(before), Some(after)) = ids {
                let _ = self.add_order_rule(before, after);
            }
        }

        let applied = self.switch_profile(profile_id)?;
        Ok(ImportOutcome {
            profile_id,
            profile: name,
            matched,
            downloads,
            manual,
            applied,
        })
    }

    // -- expected installs --------------------------------------------------

    fn expected(&self) -> Result<BTreeMap<String, Expected>> {
        Ok(self
            .store
            .get_setting(SETTING_EXPECTED)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    fn expect(&mut self, nexus_mod_id: i64, change: impl FnOnce(&mut Expected)) -> Result<()> {
        let mut all = self.expected()?;
        change(all.entry(nexus_mod_id.to_string()).or_default());
        self.store
            .set_setting(SETTING_EXPECTED, &serde_json::to_string(&all)?)?;
        Ok(())
    }

    /// Whatever was waiting for this Nexus mod, now that it is installed as
    /// `mod_id`.
    pub(crate) fn settle_expected(&mut self, mod_id: i64, nexus_mod_id: i64) -> Result<()> {
        let mut all = self.expected()?;
        let Some(expected) = all.remove(&nexus_mod_id.to_string()) else {
            return Ok(());
        };
        self.store
            .set_setting(SETTING_EXPECTED, &serde_json::to_string(&all)?)?;
        for (profile, enabled, priority) in expected.places {
            // A profile deleted since is simply skipped.
            let _ = self
                .store
                .set_profile_state(profile, mod_id, enabled, priority);
        }
        if let Some(old) = expected.replaces.filter(|old| *old != mod_id) {
            if self.store.list_mods()?.iter().any(|m| m.id == old) {
                self.replace_mod(old, mod_id)?;
            }
        }
        Ok(())
    }

    /// Put `new` where `old` was — in every profile, its group, rules, tags,
    /// notes and settings edits — then uninstall `old`. Applied at once when
    /// changes apply automatically.
    pub fn replace_mod(&mut self, old: i64, new: i64) -> Result<()> {
        let before = self.record(old)?;
        let after = self.record(new)?;
        for (profile, enabled, priority) in self.store.profile_states(old)? {
            self.store
                .set_profile_state(profile, new, enabled, priority)?;
        }
        if before.group_id.is_some() {
            self.store.assign_group(&[new], before.group_id)?;
        }
        if after.tags.is_empty() && !before.tags.is_empty() {
            self.store.set_tags(new, &before.tags)?;
        }
        if after.notes.is_none() {
            self.store.set_notes(new, before.notes.as_deref())?;
        }
        self.store.repoint_order_rules(old, new)?;
        let kept = self.keep_settings(&before.staging_folder);
        self.restore_settings(&after.staging_folder, &kept)?;

        self.uninstall(old)?;
        if self.game()?.is_some() && self.auto_apply()? {
            self.apply()?;
        }
        Ok(())
    }

    // -- updates ------------------------------------------------------------

    /// Nexus mods the last update check found a newer version of.
    pub fn updatable_mods(&self) -> Result<Vec<UpdatableMod>> {
        Ok(self
            .store
            .list_mods()?
            .into_iter()
            .filter_map(|m| {
                let latest = newer_version(&m.version, &m.latest_version)?;
                Some(UpdatableMod {
                    id: m.id,
                    name: m.name,
                    nexus_mod_id: m.nexus_mod_id?,
                    nexus_file_id: m.nexus_file_id,
                    version: m.version,
                    latest,
                })
            })
            .collect())
    }

    /// The new file for `mod_id` is about to be downloaded: when it is
    /// installed it takes the old version's place.
    pub fn expect_update(&mut self, mod_id: i64) -> Result<()> {
        let record = self.record(mod_id)?;
        let nexus = record
            .nexus_mod_id
            .ok_or_else(|| AppError::Invalid(format!("{} is not from Nexus", record.name)))?;
        self.expect(nexus, |e| e.replaces = Some(mod_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, category: &str, primary: bool) -> ModFile {
        ModFile {
            file_id: id,
            name: format!("file {id}"),
            version: None,
            file_name: format!("f{id}.zip"),
            size_in_bytes: None,
            category_name: Some(category.into()),
            is_primary: primary,
        }
    }

    #[test]
    fn an_update_comes_from_the_installed_files_category() {
        let files = [
            file(10, "OLD_VERSION", false),
            file(20, "MAIN", true),
            file(21, "OPTIONAL", false),
            file(30, "MAIN", false),
            file(31, "OPTIONAL", false),
        ];
        assert_eq!(pick_update(&files, Some(20)).unwrap().file_id, 30);
        assert_eq!(pick_update(&files, Some(21)).unwrap().file_id, 31);
        // Archived as an old version, or never known: the newest main file.
        assert_eq!(pick_update(&files, Some(10)).unwrap().file_id, 30);
        assert_eq!(pick_update(&files, None).unwrap().file_id, 30);
        // Already the newest.
        assert!(pick_update(&files, Some(30)).is_none());
    }
}
