//! UE4SS: knowing whether it is there, and installing the build that works.
//!
//! Lua, C++ and Blueprint logic mods do nothing without UE4SS, and nothing on
//! the outside says so — the mod is enabled, its files are in place, and the
//! game simply never runs it. So every mod that needs UE4SS is checked
//! against whether UE4SS will actually be loaded, and the answer is shown.
//!
//! Stellar Blade needs its own build: the upstream RE-UE4SS releases do not
//! hook this game. The community build that does is Chrisr0's fork, published
//! on GitHub with tags like `3.1.0-6` and a `UE4SS_v<tag>.zip` asset, which is
//! what the install offers. Any UE4SS archive the user installs by hand is
//! recognised just the same.

use std::path::{Path, PathBuf};

use sbmm_core::model::{DetectedComponent, ModType};
use sbmm_store::ModRecord;

use crate::dto::Ue4ssStatus;
use crate::{App, AppError, Origin, Result};

/// `owner/repo` of the Stellar Blade build of UE4SS.
pub const REPO: &str = "Chrisr0/RE-UE4SS";

/// Shown beside the install button, so it is clear whose build this is.
pub const PROJECT_URL: &str = "https://github.com/Chrisr0/RE-UE4SS";

/// The source recorded on a UE4SS the manager installed itself.
pub const SOURCE: &str = "ue4ss";

/// Where to ask GitHub for the tag list.
pub fn tags_url() -> String {
    format!("https://api.github.com/repos/{REPO}/tags?per_page=100")
}

/// The release archive for one tag.
pub fn download_url(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}/UE4SS_v{tag}.zip")
}

/// A tag such as `3.1.0-6` as comparable numbers. `None` for anything that is
/// not dotted or dashed numbers, so a branch-like tag is never offered.
pub fn parse_tag(tag: &str) -> Option<Vec<u32>> {
    let tag = tag.trim().trim_start_matches(['v', 'V']);
    let parts: Option<Vec<u32>> = tag.split(['.', '-']).map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() >= 2)
}

/// The newest release among `tags`, compared as numbers: `3.1.0-10` is newer
/// than `3.1.0-9`.
pub fn newest<'a>(tags: impl IntoIterator<Item = &'a str>) -> Option<String> {
    tags.into_iter()
        .filter_map(|tag| Some((parse_tag(tag)?, tag)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tag)| tag.to_string())
}

/// Whether a mod made of `components` only works with UE4SS loaded.
pub fn requires_ue4ss(components: &[DetectedComponent]) -> bool {
    components.iter().any(|c| {
        matches!(
            c.mod_type,
            ModType::LogicMod | ModType::Ue4ssLua | ModType::Ue4ssDll
        )
    })
}

fn provides_ue4ss(components: &[DetectedComponent]) -> bool {
    components
        .iter()
        .any(|c| c.mod_type == ModType::Ue4ssFramework)
}

/// Where UE4SS stands, in the order it is looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Installed as a mod; `enabled` says whether the game gets it.
    Managed {
        mod_id: i64,
        version: Option<String>,
        enabled: bool,
    },
    /// In the game folder, put there by hand.
    Manual {
        version: Option<String>,
    },
    Missing,
}

impl Presence {
    /// Whether the game will load UE4SS as things stand.
    pub fn loads(&self) -> bool {
        matches!(
            self,
            Presence::Managed { enabled: true, .. } | Presence::Manual { .. }
        )
    }
}

impl App {
    /// Where UE4SS stands for the active profile.
    ///
    /// A UE4SS mod wins over files already in the folder, because the files
    /// may be that very mod's deployment. An enabled one wins over a disabled
    /// one, for a user who keeps two versions installed.
    pub fn ue4ss_presence(&self, records: &[ModRecord]) -> Result<Presence> {
        let mut managed: Option<&ModRecord> = None;
        for record in records {
            if provides_ue4ss(&self.store.components_for(record.id)?)
                && managed.is_none_or(|m| !m.enabled && record.enabled)
            {
                managed = Some(record);
            }
        }
        if let Some(record) = managed {
            if record.enabled || self.game()?.is_none() {
                return Ok(Presence::Managed {
                    mod_id: record.id,
                    version: record.version.clone(),
                    enabled: record.enabled,
                });
            }
        }

        // A disabled UE4SS mod still leaves a hand-placed copy working.
        if let Some(dll) = self.manual_ue4ss_dll()? {
            return Ok(Presence::Manual {
                version: sbmm_upscaler::pe::file_version_opt(&dll).map(|v| v.to_string()),
            });
        }
        Ok(match managed {
            Some(record) => Presence::Managed {
                mod_id: record.id,
                version: record.version.clone(),
                enabled: false,
            },
            None => Presence::Missing,
        })
    }

    /// `UE4SS.dll` in the game folder that no mod of ours put there.
    fn manual_ue4ss_dll(&self) -> Result<Option<PathBuf>> {
        let Some(game) = self.game()? else {
            return Ok(None);
        };
        let deployed: Vec<String> = self
            .store
            .deployed_files()?
            .into_iter()
            .map(|(_, target, _, _)| crate::comparable_path(&target))
            .collect();
        // UE4SS 3 keeps itself in `ue4ss/`; 2.x sat beside the executable.
        for relative in ["ue4ss/UE4SS.dll", "UE4SS.dll"] {
            let target = Path::new(sbmm_core::paths::BINARIES_WIN64).join(relative);
            let full = game.root.join(&target);
            let ours = deployed.contains(&crate::comparable_path(&target.to_string_lossy()));
            if full.is_file() && !ours {
                return Ok(Some(full));
            }
        }
        Ok(None)
    }

    /// UE4SS at a glance, with the enabled mods that need it.
    pub fn ue4ss_status(&self) -> Result<Ue4ssStatus> {
        let records = self.store.list_mods()?;
        let presence = self.ue4ss_presence(&records)?;
        let mut needed_by = Vec::new();
        for record in records.iter().filter(|m| m.enabled) {
            if requires_ue4ss(&self.store.components_for(record.id)?) {
                needed_by.push(record.name.clone());
            }
        }
        let (state, mod_id, version) = match presence {
            Presence::Managed {
                mod_id,
                version,
                enabled,
            } => (
                if enabled { "managed" } else { "disabled" },
                Some(mod_id),
                version,
            ),
            Presence::Manual { version } => ("manual", None, version),
            Presence::Missing => ("missing", None, None),
        };
        Ok(Ue4ssStatus {
            state: state.to_string(),
            mod_id,
            version,
            needed_by,
            project_url: PROJECT_URL.to_string(),
        })
    }

    /// Install a downloaded UE4SS release archive as a mod and put it in the
    /// game straight away, replacing a UE4SS the manager installed before.
    ///
    /// The user's changes to `UE4SS-settings.ini` are carried over from the
    /// previous install onto the new version's file, so keys a newer version
    /// adds keep their defaults.
    pub fn install_ue4ss(&mut self, archive: &Path, version: &str) -> Result<i64> {
        let staged = self.stage_archive(archive)?;
        if !provides_ue4ss(&staged.components) {
            self.cancel_install(&staged.staging_id)?;
            return Err(AppError::Invalid(
                "the download is not a UE4SS release: it has no UE4SS.dll".into(),
            ));
        }

        let records = self.store.list_mods()?;
        let mut previous = None;
        for record in &records {
            if record.source == SOURCE && provides_ue4ss(&self.store.components_for(record.id)?) {
                previous = Some(record.clone());
            }
        }
        if let Some(old) = &previous {
            let kept = self.keep_settings(&old.staging_folder);
            self.restore_settings(&staged.staging_id, &kept)?;
            self.uninstall(old.id)?;
        }

        let name = format!("UE4SS for Stellar Blade {version}");
        let id = self.commit_install(
            &staged.staging_id,
            &name,
            None,
            Origin {
                source: SOURCE.into(),
                nexus_mod_id: None,
                nexus_file_id: None,
                version: Some(version.to_string()),
            },
        )?;
        self.set_enabled(&[id], true)?;
        if self.game()?.is_some() {
            if self.auto_apply()? {
                self.apply()?;
            } else {
                // Changes the user staged on purpose stay staged: only UE4SS
                // goes in, with `mods.txt` brought in line with it.
                let ctx = self.deploy_context()?;
                let record = self.record(id)?;
                let components = self.store.components_for(id)?;
                let plan = self.deploy_plan_for(&record, &components)?;
                self.deploy_mod(&ctx, id, &plan)?;
                if self.deployment_mode()? == crate::dto::DeploymentMode::Hardlink {
                    self.sync_ue4ss_registrations(&ctx)?;
                }
            }
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_compare_as_numbers() {
        assert_eq!(
            newest(["3.1.0-2", "3.1.0-10", "3.1.0-9", "nightly"]).as_deref(),
            Some("3.1.0-10")
        );
        assert_eq!(parse_tag("v3.0.1"), Some(vec![3, 0, 1]));
        assert_eq!(parse_tag("experimental-latest"), None);
        assert_eq!(newest(["main"]), None);
    }

    #[test]
    fn the_download_is_the_release_archive() {
        assert_eq!(
            download_url("3.1.0-6"),
            "https://github.com/Chrisr0/RE-UE4SS/releases/download/3.1.0-6/UE4SS_v3.1.0-6.zip"
        );
    }
}
