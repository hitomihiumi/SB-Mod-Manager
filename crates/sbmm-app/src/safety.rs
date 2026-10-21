//! Getting back from things going wrong: a game update under the mods, files
//! changed behind the manager's back, and a change to the mod list that did
//! not work out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use sbmm_deploy::{DeployBackend, DeployedFile, Repair};
use sbmm_game::GameBuild;
use sbmm_store::PointEntry;

use crate::dto::{
    ApplyFailure, ApplyReport, GameUpdate, RepairReport, RestorePointView, RestoreReport,
};
use crate::{App, AppError, Result};

/// The build the user last accepted, as JSON.
const SETTING_KNOWN_BUILD: &str = "knownGameBuild";
/// Automatic restore points kept per profile; the user's own are never pruned.
const KEEP_AUTOMATIC_POINTS: usize = 30;

impl App {
    // -- game updates -------------------------------------------------------

    fn current_build(&self) -> Result<Option<GameBuild>> {
        Ok(self
            .game()?
            .map(|game| GameBuild::read(&game))
            .filter(|build| !build.is_unknown()))
    }

    /// The game changed build since the user last said it was fine. The
    /// first build seen is simply remembered.
    pub(crate) fn game_update(&self, drifted: usize) -> Result<Option<GameUpdate>> {
        let Some(current) = self.current_build()? else {
            return Ok(None);
        };
        let known = self
            .store
            .get_setting(SETTING_KNOWN_BUILD)?
            .and_then(|json| serde_json::from_str::<GameBuild>(&json).ok());
        match known {
            None => {
                self.remember_build(&current)?;
                Ok(None)
            }
            Some(known) if known.same_as(&current) => Ok(None),
            Some(previous) => Ok(Some(GameUpdate {
                previous,
                current,
                drifted,
            })),
        }
    }

    fn remember_build(&self, build: &GameBuild) -> Result<()> {
        self.store
            .set_setting(SETTING_KNOWN_BUILD, &serde_json::to_string(build)?)?;
        Ok(())
    }

    /// The user has seen that the game was updated; stop saying so.
    pub fn acknowledge_game_update(&mut self) -> Result<()> {
        if let Some(build) = self.current_build()? {
            self.remember_build(&build)?;
        }
        Ok(())
    }

    /// Steam's build id, for restore points; other installs have no name
    /// worth showing for their build.
    fn build_label(&self) -> Result<Option<String>> {
        Ok(self
            .current_build()?
            .and_then(|build| build.build_id)
            .map(|id| format!("build {id}")))
    }

    // -- drift --------------------------------------------------------------

    /// Put every deployed file that was changed or removed outside the
    /// manager back the way its mod has it. What was found in its place is
    /// kept: a settings edit joins the mod, anything else becomes the file
    /// disabling the mod restores.
    pub fn repair_drift(&mut self) -> Result<RepairReport> {
        let ctx = self.deploy_context()?;
        let manifest = self.global_manifest()?;
        let drifted: BTreeSet<PathBuf> = self
            .backend
            .verify(&ctx, &manifest)?
            .into_iter()
            .map(|d| d.target)
            .collect();
        let mut report = RepairReport::default();
        if drifted.is_empty() {
            return Ok(report);
        }

        let mut by_mod: BTreeMap<i64, Vec<DeployedFile>> = BTreeMap::new();
        for file in manifest.files {
            by_mod.entry(file.mod_id).or_default().push(file);
        }
        let records = self.store.list_mods()?;

        for (mod_id, files) in by_mod {
            if !files.iter().any(|f| drifted.contains(&f.target)) {
                continue;
            }
            let Some(record) = records.iter().find(|r| r.id == mod_id) else {
                continue;
            };
            let components = self.store.components_for(mod_id)?;
            let plan = self.deploy_plan_for(record, &components)?;

            let mut rows = Vec::with_capacity(files.len());
            let mut created = Vec::new();
            for file in files {
                let file = if drifted.contains(&file.target) {
                    match sbmm_deploy::repair(&ctx, &plan, &file) {
                        Ok(done) => {
                            match done.how {
                                Repair::Relinked => report.relinked += 1,
                                Repair::Recorded => report.recorded += 1,
                                Repair::Adopted => report.adopted += 1,
                                Repair::Displaced => report.displaced += 1,
                            }
                            created.extend(done.created_dirs);
                            done.file
                        }
                        Err(error) => {
                            report.failed.push(ApplyFailure {
                                name: format!("{} ({})", record.name, file.target.display()),
                                reason: error.to_string(),
                            });
                            file
                        }
                    }
                } else {
                    file
                };
                rows.push((
                    file.target.to_string_lossy().into_owned(),
                    file.backup.map(|b| b.to_string_lossy().into_owned()),
                    file.size as i64,
                ));
            }
            self.store.replace_deployment(mod_id, &rows)?;
            let created: Vec<String> = created
                .iter()
                .map(|d| d.to_string_lossy().into_owned())
                .collect();
            self.store.record_created_dirs(&created)?;
        }
        Ok(report)
    }

    // -- restore points -----------------------------------------------------

    fn point_entries(&self) -> Result<Vec<PointEntry>> {
        Ok(self
            .store
            .list_mods()?
            .into_iter()
            .map(|m| PointEntry {
                mod_id: m.id,
                enabled: m.enabled,
                priority: m.priority,
            })
            .collect())
    }

    /// Remember the active profile as it is now.
    pub fn create_restore_point(&mut self, label: &str) -> Result<i64> {
        let label = label.trim();
        if label.is_empty() {
            return Err(AppError::Invalid("a restore point needs a name".into()));
        }
        let profile = self.store.active_profile()?;
        Ok(self.store.add_restore_point(
            profile,
            label,
            false,
            self.build_label()?.as_deref(),
            &self.point_entries()?,
        )?)
    }

    /// Taken by the manager after changes reach the game, unless nothing
    /// differs from the newest point.
    pub(crate) fn record_automatic_point(&self, label: &str) -> Result<()> {
        let profile = self.store.active_profile()?;
        let entries = self.point_entries()?;
        let newest = self.store.restore_points(profile)?.into_iter().next();
        if newest.is_some_and(|point| same_setup(&point.entries, &entries)) {
            return Ok(());
        }
        self.store.add_restore_point(
            profile,
            label,
            true,
            self.build_label()?.as_deref(),
            &entries,
        )?;
        self.store
            .prune_automatic_points(profile, KEEP_AUTOMATIC_POINTS)?;
        Ok(())
    }

    /// The active profile's points, newest first.
    pub fn restore_points(&self) -> Result<Vec<RestorePointView>> {
        let profile = self.store.active_profile()?;
        let installed: BTreeSet<i64> = self.store.list_mods()?.iter().map(|m| m.id).collect();
        Ok(self
            .store
            .restore_points(profile)?
            .into_iter()
            .map(|point| RestorePointView {
                id: point.id,
                created_at: point.created_at,
                label: point.label,
                automatic: point.automatic,
                game_build: point.game_build,
                enabled: point.entries.iter().filter(|e| e.enabled).count(),
                uninstalled: point
                    .entries
                    .iter()
                    .filter(|e| e.enabled && !installed.contains(&e.mod_id))
                    .count(),
            })
            .collect())
    }

    pub fn delete_restore_point(&mut self, id: i64) -> Result<()> {
        Ok(self.store.delete_restore_point(id)?)
    }

    /// Put the active profile back as a point had it, and apply. The setup
    /// being left is kept as a point first, so the restore can be undone.
    /// Mods installed since the point are disabled; mods uninstalled since
    /// cannot come back and are counted.
    pub fn restore_to_point(&mut self, id: i64) -> Result<RestoreReport> {
        let profile = self.store.active_profile()?;
        let point = self
            .store
            .restore_points(profile)?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::Invalid("that restore point no longer exists".into()))?;
        self.record_automatic_point(&format!("Before going back to “{}”", point.label))?;

        let mods = self.store.list_mods()?;
        let installed: BTreeSet<i64> = mods.iter().map(|m| m.id).collect();
        let mut entries: Vec<&PointEntry> = point
            .entries
            .iter()
            .filter(|e| installed.contains(&e.mod_id))
            .collect();
        entries.sort_by_key(|e| e.priority);
        let known: BTreeSet<i64> = entries.iter().map(|e| e.mod_id).collect();

        // The point's order first, then anything newer in its present order.
        let mut order: Vec<i64> = entries.iter().map(|e| e.mod_id).collect();
        order.extend(mods.iter().map(|m| m.id).filter(|id| !known.contains(id)));
        self.store.set_order(&order)?;

        let (on, off): (Vec<i64>, Vec<i64>) = order
            .iter()
            .partition(|id| entries.iter().any(|e| e.mod_id == **id && e.enabled));
        self.store.set_enabled(&off, false)?;
        self.store.set_enabled(&on, true)?;

        let applied = if self.game()?.is_some() {
            self.reconcile()?
        } else {
            ApplyReport::default()
        };
        Ok(RestoreReport {
            applied,
            uninstalled: point
                .entries
                .iter()
                .filter(|e| e.enabled && !installed.contains(&e.mod_id))
                .count(),
        })
    }
}

/// Whether two points describe the same setup: the same mods enabled, in
/// the same relative order. Priorities themselves are renumbered freely.
fn same_setup(a: &[PointEntry], b: &[PointEntry]) -> bool {
    let enabled = |entries: &[PointEntry]| {
        let mut on: Vec<&PointEntry> = entries.iter().filter(|e| e.enabled).collect();
        on.sort_by_key(|e| e.priority);
        on.into_iter().map(|e| e.mod_id).collect::<Vec<_>>()
    };
    enabled(a) == enabled(b)
}

/// "Enabled A, B · removed C", for an automatic point's label.
pub(crate) fn describe(report: &ApplyReport) -> String {
    fn names(list: &[String]) -> String {
        let shown: Vec<&str> = list.iter().take(3).map(String::as_str).collect();
        let more = list.len().saturating_sub(3);
        if more > 0 {
            format!("{} and {more} more", shown.join(", "))
        } else {
            shown.join(", ")
        }
    }
    let mut parts = Vec::new();
    if !report.deployed.is_empty() {
        parts.push(format!("Applied {}", names(&report.deployed)));
    }
    if !report.removed.is_empty() {
        parts.push(format!("removed {}", names(&report.removed)));
    }
    if parts.is_empty() {
        "Applied changes".into()
    } else {
        parts.join(" · ")
    }
}
