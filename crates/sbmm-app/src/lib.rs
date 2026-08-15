//! The application service.
//!
//! All of the manager's behaviour lives here rather than in the Tauri layer,
//! which keeps it compilable and testable on any platform; `src-tauri` is only
//! a set of thin command wrappers around this type.

pub mod dto;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sbmm_core::model::{Confidence, DetectedComponent, ModType};
use sbmm_core::plan::{self, build_plan, DeployPlan};
use sbmm_core::{detect_components, FileTree};
use sbmm_deploy::backend::DeployContext;
use sbmm_deploy::{ue4ss, DeployBackend, HardlinkBackend, Manifest};
use sbmm_game::GameInstall;
use sbmm_store::{ModRecord, NewMod, Store};

use dto::*;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Store(#[from] sbmm_store::StoreError),
    #[error(transparent)]
    Archive(#[from] sbmm_archive::ArchiveError),
    #[error(transparent)]
    Deploy(#[from] sbmm_deploy::DeployError),
    #[error(transparent)]
    Discovery(#[from] sbmm_game::DiscoveryError),
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("no game folder has been selected yet")]
    NoGame,
    #[error("no staged install called {0}")]
    UnknownStaging(String),
}

impl AppError {
    fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        AppError::Io {
            path: path.into(),
            source,
        }
    }
}

type Result<T> = std::result::Result<T, AppError>;

const SETTING_GAME_ROOT: &str = "gameRoot";
const SETTING_AUTO_APPLY: &str = "autoApply";

pub struct App {
    store: Store,
    data_dir: PathBuf,
    backend: HardlinkBackend,
}

impl App {
    pub fn new(data_dir: impl Into<PathBuf>) -> Result<Self> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir).map_err(|e| AppError::io(&data_dir, e))?;
        let store = Store::open(data_dir.join("sbmm.db"))?;
        std::fs::create_dir_all(data_dir.join("mods"))
            .map_err(|e| AppError::io(data_dir.join("mods"), e))?;
        Ok(Self {
            store,
            data_dir,
            backend: HardlinkBackend,
        })
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.data_dir.join("mods")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }

    fn staging_path(&self, folder: &str) -> PathBuf {
        self.mods_dir().join(folder)
    }

    // -- game --------------------------------------------------------------

    pub fn discover_games(&self) -> Vec<GameInstall> {
        sbmm_game::discover()
    }

    pub fn game(&self) -> Result<Option<GameInstall>> {
        let Some(root) = self.store.get_setting(SETTING_GAME_ROOT)? else {
            return Ok(None);
        };
        Ok(sbmm_game::validate(root).ok())
    }

    pub fn set_game_root(&mut self, path: impl AsRef<Path>) -> Result<GameInstall> {
        let install = sbmm_game::validate(path)?;
        self.store
            .set_setting(SETTING_GAME_ROOT, &install.root.to_string_lossy())?;
        Ok(install)
    }

    fn deploy_context(&self) -> Result<DeployContext> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        Ok(DeployContext {
            game_root: game.root,
            backup_root: self.backup_dir(),
        })
    }

    // -- installing --------------------------------------------------------

    /// Extract an archive into staging and inspect it, without committing.
    ///
    /// Extraction happens once: confirming the install simply records the
    /// already-extracted folder.
    pub fn stage_archive(&mut self, archive: impl AsRef<Path>) -> Result<StagedInstall> {
        let archive = archive.as_ref();
        let suggested_name = archive_display_name(archive);
        let staging_id = self.unique_staging_folder(&suggested_name);
        let dest = self.staging_path(&staging_id);

        sbmm_archive::extract(archive, &dest)?;
        self.inspect_staged(staging_id, suggested_name, dest)
    }

    /// Register a folder the user dropped in directly, treating it as if it
    /// had been extracted from an archive.
    pub fn stage_folder(&mut self, folder: impl AsRef<Path>) -> Result<StagedInstall> {
        let folder = folder.as_ref();
        let suggested_name = folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Mod".to_string());
        let staging_id = self.unique_staging_folder(&suggested_name);
        let dest = self.staging_path(&staging_id);

        copy_tree(folder, &dest)?;
        self.inspect_staged(staging_id, suggested_name, dest)
    }

    fn inspect_staged(
        &self,
        staging_id: String,
        suggested_name: String,
        dest: PathBuf,
    ) -> Result<StagedInstall> {
        let tree = FileTree::new(sbmm_archive::walk_relative(&dest));
        let components = detect_components(&tree);
        let needs_confirmation = components
            .iter()
            .any(|c| c.confidence == Confidence::AskUser);

        Ok(StagedInstall {
            staging_id,
            suggested_name,
            size_bytes: sbmm_archive::directory_size(&dest) as i64,
            components,
            needs_confirmation,
        })
    }

    /// Commit a staged install, optionally overriding the detected type.
    pub fn confirm_install(
        &mut self,
        staging_id: &str,
        name: &str,
        type_override: Option<ModType>,
    ) -> Result<i64> {
        let dest = self.staging_path(staging_id);
        if !dest.is_dir() {
            return Err(AppError::UnknownStaging(staging_id.to_string()));
        }

        let tree = FileTree::new(sbmm_archive::walk_relative(&dest));
        let mut components = detect_components(&tree);

        if let Some(forced) = type_override {
            components = components
                .iter()
                .map(|c| plan::retarget(c, forced, name))
                .collect();
            // Remember the correction so an archive shaped like this one is
            // classified without asking next time.
            self.store
                .remember_detection(&detection_pattern(&tree), forced.as_str())?;
        }

        let primary_type = components
            .first()
            .map(|c| c.mod_type.as_str())
            .unwrap_or("unknown")
            .to_string();

        let id = self.store.insert_mod(&NewMod {
            name: name.to_string(),
            staging_folder: staging_id.to_string(),
            version: None,
            source: "manual".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            primary_type,
            components,
            size_bytes: sbmm_archive::directory_size(&dest) as i64,
            image_path: None,
        })?;
        Ok(id)
    }

    pub fn cancel_install(&mut self, staging_id: &str) -> Result<()> {
        let dest = self.staging_path(staging_id);
        if dest.is_dir() {
            std::fs::remove_dir_all(&dest).map_err(|e| AppError::io(&dest, e))?;
        }
        Ok(())
    }

    /// Remove a mod completely: undeploy, forget, and delete its files.
    pub fn uninstall(&mut self, mod_id: i64) -> Result<()> {
        let record = self.record(mod_id)?;
        if self.game()?.is_some() {
            self.undeploy_mod(mod_id)?;
        }
        let staging = self.staging_path(&record.staging_folder);
        if staging.is_dir() {
            std::fs::remove_dir_all(&staging).map_err(|e| AppError::io(&staging, e))?;
        }
        self.store.delete_mod(mod_id)?;
        Ok(())
    }

    fn unique_staging_folder(&self, name: &str) -> String {
        let base = plan::sanitize_folder_name(name);
        let mut candidate = base.clone();
        let mut counter = 2;
        while self.staging_path(&candidate).exists() {
            candidate = format!("{base} ({counter})");
            counter += 1;
        }
        candidate
    }

    // -- state -------------------------------------------------------------

    pub fn snapshot(&self) -> Result<AppSnapshot> {
        let records = self.store.list_mods()?;
        let mut mods = Vec::with_capacity(records.len());
        let mut pending = Vec::new();

        for record in &records {
            let components = self.store.components_for(record.id)?;
            let planned = self.plan_for(record, &components);
            let recorded = self.store.deployed_files_for(record.id)?;
            let deployed = !recorded.is_empty();

            if let Some(kind) = change_kind(record.enabled, deployed, &planned, &recorded) {
                pending.push(PendingChange {
                    mod_id: record.id,
                    name: record.name.clone(),
                    kind,
                });
            }

            let mut component_types: Vec<String> = components
                .iter()
                .map(|c| c.mod_type.as_str().to_string())
                .collect();
            component_types.dedup();

            mods.push(ModView {
                id: record.id,
                name: record.name.clone(),
                version: record.version.clone(),
                mod_type: record.primary_type.clone(),
                component_types,
                group_id: record.group_id,
                enabled: record.enabled,
                deployed,
                priority: record.priority,
                size_bytes: record.size_bytes,
                source: record.source.clone(),
                installed_at: record.installed_at.clone(),
                warnings: components.iter().flat_map(|c| c.warnings.clone()).collect(),
            });
        }

        let groups = self
            .store
            .list_groups()?
            .into_iter()
            .map(|g| GroupView {
                id: g.id,
                name: g.name,
                color: g.color,
                collapsed: g.collapsed,
                sort_index: g.sort_index,
            })
            .collect();

        Ok(AppSnapshot {
            game: self.game()?,
            mods,
            groups,
            pending,
            drift: self.drift()?,
            auto_apply: self.auto_apply()?,
        })
    }

    fn drift(&self) -> Result<Vec<String>> {
        let Ok(ctx) = self.deploy_context() else {
            return Ok(Vec::new());
        };
        let manifest = self.global_manifest()?;
        if manifest.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self
            .backend
            .verify(&ctx, &manifest)?
            .into_iter()
            .map(|d| d.target.to_string_lossy().into_owned())
            .collect())
    }

    pub fn auto_apply(&self) -> Result<bool> {
        Ok(self
            .store
            .get_setting(SETTING_AUTO_APPLY)?
            .as_deref()
            .map(|v| v == "1")
            .unwrap_or(true))
    }

    pub fn set_auto_apply(&mut self, on: bool) -> Result<()> {
        self.store
            .set_setting(SETTING_AUTO_APPLY, if on { "1" } else { "0" })?;
        Ok(())
    }

    // -- toggling ----------------------------------------------------------

    pub fn set_enabled(&mut self, mod_ids: &[i64], enabled: bool) -> Result<()> {
        self.store.set_enabled(mod_ids, enabled)?;
        Ok(())
    }

    /// Every mod in a group, for the group-level checkbox.
    pub fn mods_in_group(&self, group_id: Option<i64>) -> Result<Vec<i64>> {
        Ok(self
            .store
            .list_mods()?
            .into_iter()
            .filter(|m| m.group_id == group_id)
            .map(|m| m.id)
            .collect())
    }

    pub fn all_mod_ids(&self) -> Result<Vec<i64>> {
        Ok(self.store.list_mods()?.into_iter().map(|m| m.id).collect())
    }

    pub fn set_order(&mut self, ordered_ids: &[i64]) -> Result<()> {
        self.store.set_order(ordered_ids)?;
        Ok(())
    }

    pub fn assign_group(&mut self, mod_ids: &[i64], group_id: Option<i64>) -> Result<()> {
        self.store.assign_group(mod_ids, group_id)?;
        Ok(())
    }

    pub fn create_group(&mut self, name: &str, color: Option<&str>) -> Result<i64> {
        Ok(self.store.create_group(name, color)?)
    }

    pub fn delete_group(&mut self, group_id: i64) -> Result<()> {
        self.store.delete_group(group_id)?;
        Ok(())
    }

    pub fn set_group_collapsed(&mut self, group_id: i64, collapsed: bool) -> Result<()> {
        self.store.set_group_collapsed(group_id, collapsed)?;
        Ok(())
    }

    pub fn rename_mod(&mut self, mod_id: i64, name: &str) -> Result<()> {
        self.store.rename_mod(mod_id, name)?;
        Ok(())
    }

    // -- applying ----------------------------------------------------------

    /// Reconcile the game folder with the desired enabled set.
    ///
    /// Each mod is handled independently so one bad mod cannot stop the rest,
    /// and any mod that fails is reported rather than silently skipped.
    pub fn apply(&mut self) -> Result<ApplyReport> {
        let ctx = self.deploy_context()?;
        let mut report = ApplyReport::default();

        for record in self.store.list_mods()? {
            let components = self.store.components_for(record.id)?;
            let planned = self.plan_for(&record, &components);
            let recorded = self.store.deployed_files_for(record.id)?;
            let deployed = !recorded.is_empty();

            let Some(kind) = change_kind(record.enabled, deployed, &planned, &recorded) else {
                continue;
            };

            let outcome = match kind {
                ChangeKind::Disable => self.undeploy_mod(record.id).map(|_| Outcome::Removed),
                ChangeKind::Enable => self
                    .deploy_mod(&ctx, record.id, &planned)
                    .map(|_| Outcome::Deployed),
                ChangeKind::Reorder => self
                    .undeploy_mod(record.id)
                    .and_then(|_| self.deploy_mod(&ctx, record.id, &planned))
                    .map(|_| Outcome::Deployed),
            };

            match outcome {
                Ok(Outcome::Deployed) => report.deployed.push(record.name.clone()),
                Ok(Outcome::Removed) => report.removed.push(record.name.clone()),
                Err(err) => report.failed.push(ApplyFailure {
                    name: record.name.clone(),
                    reason: err.to_string(),
                }),
            }
        }

        self.sync_ue4ss_registrations(&ctx)?;
        Ok(report)
    }

    fn plan_for(&self, record: &ModRecord, components: &[DetectedComponent]) -> DeployPlan {
        build_plan(
            record.id,
            self.staging_path(&record.staging_folder),
            components,
            record.priority,
            &record.name,
        )
    }

    fn deploy_mod(&mut self, ctx: &DeployContext, mod_id: i64, plan: &DeployPlan) -> Result<()> {
        let manifest = self.backend.deploy(ctx, plan)?;

        let rows: Vec<(String, Option<String>, i64)> = manifest
            .files
            .iter()
            .map(|f| {
                (
                    f.target.to_string_lossy().into_owned(),
                    f.backup.as_ref().map(|b| b.to_string_lossy().into_owned()),
                    f.size as i64,
                )
            })
            .collect();
        self.store.replace_deployment(mod_id, &rows)?;

        let dirs: Vec<String> = manifest
            .created_dirs
            .iter()
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        self.store.record_created_dirs(&dirs)?;
        Ok(())
    }

    fn undeploy_mod(&mut self, mod_id: i64) -> Result<()> {
        let ctx = self.deploy_context()?;
        let files = self.store.deployed_files_for(mod_id)?;
        if files.is_empty() {
            return Ok(());
        }

        // Directory ownership is shared across mods, so removal is offered the
        // full set of directories we ever created; only empty ones go away.
        let manifest = Manifest {
            files: files
                .iter()
                .map(|(target, backup, size)| sbmm_deploy::DeployedFile {
                    target: PathBuf::from(target),
                    mod_id,
                    backup: backup.as_ref().map(PathBuf::from),
                    size: *size as u64,
                })
                .collect(),
            created_dirs: self
                .store
                .created_dirs()?
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        };

        self.backend.undeploy(&ctx, &manifest)?;
        self.store.clear_deployment(mod_id)?;

        // Forget directories that are gone so the list does not grow forever.
        let still_present: Vec<String> = self
            .store
            .created_dirs()?
            .into_iter()
            .filter(|d| !ctx.game_root.join(d).exists())
            .collect();
        self.store.forget_created_dirs(&still_present)?;
        Ok(())
    }

    /// The record of everything currently in the game folder.
    fn global_manifest(&self) -> Result<Manifest> {
        Ok(Manifest {
            files: self
                .store
                .deployed_files()?
                .into_iter()
                .map(|(mod_id, target, backup, size)| sbmm_deploy::DeployedFile {
                    target: PathBuf::from(target),
                    mod_id,
                    backup: backup.map(PathBuf::from),
                    size: size as u64,
                })
                .collect(),
            created_dirs: self
                .store
                .created_dirs()?
                .into_iter()
                .map(PathBuf::from)
                .collect(),
        })
    }

    /// UE4SS only loads mods listed in `mods.txt`, so enabling a Lua or C++
    /// mod means rewriting that file as well as placing its folder.
    fn sync_ue4ss_registrations(&self, ctx: &DeployContext) -> Result<()> {
        let mut states: BTreeMap<String, bool> = BTreeMap::new();
        let mut any = false;

        for record in self.store.list_mods()? {
            let components = self.store.components_for(record.id)?;
            let plan = self.plan_for(&record, &components);
            for name in plan.ue4ss_registrations {
                any = true;
                // If two mods share a folder name, enabled wins.
                let entry = states.entry(name).or_insert(false);
                *entry = *entry || record.enabled;
            }
        }

        if !any {
            return Ok(());
        }

        if states.values().any(|enabled| *enabled) {
            ue4ss::sync_mods_txt(&ctx.game_root, &ctx.backup_root, &states)?;
        } else {
            // Nothing left for UE4SS to load: take our mods.txt back out and
            // let the folders it lived in go with it.
            ue4ss::clear_registrations(&ctx.game_root, &ctx.backup_root)?;
            self.prune_empty_dirs(ctx)?;
        }
        Ok(())
    }

    /// Drop any directory we created that is now empty, and stop tracking it.
    fn prune_empty_dirs(&self, ctx: &DeployContext) -> Result<()> {
        let dirs: Vec<PathBuf> = self
            .store
            .created_dirs()?
            .into_iter()
            .map(PathBuf::from)
            .collect();
        sbmm_deploy::prune_dirs(&ctx.game_root, &dirs);

        let gone: Vec<String> = dirs
            .iter()
            .filter(|d| !ctx.game_root.join(d).exists())
            .map(|d| d.to_string_lossy().into_owned())
            .collect();
        self.store.forget_created_dirs(&gone)?;
        Ok(())
    }

    fn record(&self, mod_id: i64) -> Result<ModRecord> {
        self.store
            .list_mods()?
            .into_iter()
            .find(|m| m.id == mod_id)
            .ok_or(AppError::Store(sbmm_store::StoreError::UnknownMod(mod_id)))
    }
}

enum Outcome {
    Deployed,
    Removed,
}

/// What, if anything, needs to happen to bring this mod in line.
fn change_kind(
    enabled: bool,
    deployed: bool,
    planned: &DeployPlan,
    recorded: &[sbmm_store::DeployedFileRow],
) -> Option<ChangeKind> {
    match (enabled, deployed) {
        (true, false) if !planned.is_empty() => Some(ChangeKind::Enable),
        (false, true) => Some(ChangeKind::Disable),
        (true, true) => {
            // A priority change renames pak files, so compare destinations.
            let mut want: Vec<String> = planned
                .files
                .iter()
                .map(|f| f.target.to_string_lossy().into_owned())
                .collect();
            let mut have: Vec<String> = recorded.iter().map(|(t, _, _)| t.clone()).collect();
            want.sort();
            have.sort();
            (want != have).then_some(ChangeKind::Reorder)
        }
        _ => None,
    }
}

/// A rough fingerprint of an archive's shape, used to remember user
/// corrections for similarly structured mods.
fn detection_pattern(tree: &FileTree) -> String {
    let mut extensions: Vec<String> = tree
        .entries()
        .iter()
        .filter_map(|e| e.extension().map(str::to_string))
        .collect();
    extensions.sort();
    extensions.dedup();
    extensions.join(",")
}

fn archive_display_name(archive: &Path) -> String {
    let stem = archive
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Mod".to_string());
    // Nexus downloads look like `Cool Outfit-1234-1-0-1699999999`: the mod id,
    // the version split on dots, then a timestamp. Strip that whole tail, but
    // only when several numeric segments are present, so an ordinary name like
    // `Outfit-2` survives intact.
    let segments: Vec<&str> = stem.split('-').collect();
    let numeric_tail = segments
        .iter()
        .rev()
        .take_while(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .count();

    let trimmed = if numeric_tail >= 3 && numeric_tail < segments.len() {
        segments[..segments.len() - numeric_tail].join("-")
    } else {
        stem
    };
    trimmed.trim().to_string()
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for relative in sbmm_archive::walk_relative(from) {
        let src = from.join(&relative);
        let dst = to.join(&relative);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
        }
        std::fs::copy(&src, &dst).map_err(|e| AppError::io(&dst, e))?;
    }
    Ok(())
}
