//! The application service.
//!
//! All of the manager's behaviour lives here rather than in the Tauri layer,
//! which keeps it compilable and testable on any platform; `src-tauri` is only
//! a set of thin command wrappers around this type.

pub mod credentials;
pub mod dto;
pub mod library;

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
    #[error("{0}")]
    BadLibraryRoot(String),
    #[error(transparent)]
    KeyStore(#[from] credentials::KeyStoreError),
    #[error(transparent)]
    Encode(#[from] serde_json::Error),
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

/// Where an installed mod came from.
///
/// Only mods that carry a Nexus id can be checked for updates later, so the
/// origin is recorded at install time rather than guessed afterwards.
#[derive(Debug, Clone, Default)]
pub struct Origin {
    pub source: String,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
    pub version: Option<String>,
}

impl Origin {
    /// An archive the user dropped in themselves.
    pub fn manual() -> Self {
        Self {
            source: "manual".into(),
            ..Self::default()
        }
    }

    pub fn nexus(mod_id: i64, file_id: i64) -> Self {
        Self {
            source: "nexus".into(),
            nexus_mod_id: Some(mod_id),
            nexus_file_id: Some(file_id),
            version: None,
        }
    }

    pub fn with_version(mut self, version: Option<String>) -> Self {
        self.version = version;
        self
    }
}

const SETTING_GAME_ROOT: &str = "gameRoot";
const SETTING_AUTO_APPLY: &str = "autoApply";
const SETTING_LIBRARY_ROOT: &str = "libraryRoot";
const SETTING_NEXUS_ACCOUNT: &str = "nexusAccount";
const SETTING_UPDATE_CHANNEL: &str = "updateChannel";

pub struct App {
    store: Store,
    data_dir: PathBuf,
    backend: HardlinkBackend,
    keys: Box<dyn credentials::KeyStore>,
}

impl App {
    pub fn new(data_dir: impl Into<PathBuf>) -> Result<Self> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir).map_err(|e| AppError::io(&data_dir, e))?;
        // The database has to be found before any setting can be read, so it
        // always lives here even when the library itself is on another drive.
        let store = Store::open(data_dir.join("sbmm.db"))?;
        let app = Self {
            store,
            data_dir,
            backend: HardlinkBackend,
            keys: Box::new(credentials::OsKeyStore),
        };
        std::fs::create_dir_all(app.mods_dir()).map_err(|e| AppError::io(app.mods_dir(), e))?;
        Ok(app)
    }

    /// Build with a different credential store, for tests.
    pub fn with_key_store(mut self, keys: Box<dyn credentials::KeyStore>) -> Self {
        self.keys = keys;
        self
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.library_root().join("downloads")
    }

    // -- Nexus credentials -------------------------------------------------

    /// The stored API key, if there is one.
    pub fn nexus_api_key(&self) -> Result<Option<String>> {
        Ok(self.keys.get()?)
    }

    /// Save a key and remember the account it belongs to.
    ///
    /// The key itself never reaches the database — only the account summary,
    /// which the settings screen shows so the user can tell whose key is in
    /// use without revealing it.
    pub fn set_nexus_account(
        &mut self,
        api_key: &str,
        account: Option<&NexusAccount>,
    ) -> Result<()> {
        if api_key.is_empty() {
            self.keys.clear()?;
            self.store.set_setting(SETTING_NEXUS_ACCOUNT, "")?;
            return Ok(());
        }
        self.keys.set(api_key)?;
        let summary = account
            .map(serde_json::to_string)
            .transpose()?
            .unwrap_or_default();
        self.store.set_setting(SETTING_NEXUS_ACCOUNT, &summary)?;
        Ok(())
    }

    /// Who the stored key belongs to, as of the last check.
    pub fn nexus_account(&self) -> Result<Option<NexusAccount>> {
        let Some(raw) = self.store.get_setting(SETTING_NEXUS_ACCOUNT)? else {
            return Ok(None);
        };
        if raw.is_empty() {
            return Ok(None);
        }
        Ok(serde_json::from_str(&raw).ok())
    }

    // -- self update -------------------------------------------------------

    /// Which release stream to update from: `stable` or `nightly`.
    pub fn update_channel(&self) -> Result<String> {
        Ok(self
            .store
            .get_setting(SETTING_UPDATE_CHANNEL)?
            .filter(|c| c == "nightly")
            .unwrap_or_else(|| "stable".to_string()))
    }

    pub fn set_update_channel(&mut self, channel: &str) -> Result<()> {
        let normalised = if channel == "nightly" {
            "nightly"
        } else {
            "stable"
        };
        self.store.set_setting(SETTING_UPDATE_CHANNEL, normalised)?;
        Ok(())
    }

    /// Where mods and backups are kept. Defaults to the app data directory.
    pub fn library_root(&self) -> PathBuf {
        self.store
            .get_setting(SETTING_LIBRARY_ROOT)
            .ok()
            .flatten()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.data_dir.clone())
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.library_root().join("mods")
    }

    pub fn backup_dir(&self) -> PathBuf {
        self.library_root().join("backups")
    }

    /// Where the library sits, and whether hard links to the game will work.
    pub fn folders(&self) -> Result<FoldersView> {
        let game = self.game()?.map(|g| g.root);
        let library = self.library_root();
        Ok(FoldersView {
            same_volume_as_game: game
                .as_ref()
                .map(|g| library::same_volume(&library, g))
                .unwrap_or(true),
            game: game.map(path_string),
            mods: path_string(self.mods_dir()),
            backups: path_string(self.backup_dir()),
            library_bytes: library::size_of(&library) as i64,
            downloads: path_string(self.downloads_dir()),
            library: path_string(library),
        })
    }

    /// Relocate the library, carrying its contents across.
    ///
    /// Enabled mods are taken out of the game folder before the move and put
    /// back afterwards. Doing it in that order matters: a hard link left in
    /// place would still point at the old location, and moving its target
    /// across volumes would quietly turn it into an orphaned copy.
    pub fn set_library_root(&mut self, path: impl AsRef<Path>) -> Result<LibraryMoveReport> {
        let new_root = path.as_ref().to_path_buf();
        let old_root = self.library_root();

        if new_root == old_root {
            return Ok(LibraryMoveReport {
                folders: self.folders()?,
                redeployed: ApplyReport::default(),
            });
        }

        let game = self.game()?.map(|g| g.root);
        library::validate_root(&new_root, game.as_deref())?;

        // Reuse the normal reconciliation path so mods.txt and created
        // directories are handled exactly as they are for any other change.
        let enabled = self.store.enabled_mod_ids()?;
        let restore = game.is_some() && !enabled.is_empty();
        if restore {
            self.store.set_enabled(&enabled, false)?;
            self.apply()?;
        }

        library::move_tree(&old_root.join("mods"), &new_root.join("mods"))?;
        library::move_tree(&old_root.join("backups"), &new_root.join("backups"))?;
        library::move_tree(&old_root.join("downloads"), &new_root.join("downloads"))?;

        self.store
            .set_setting(SETTING_LIBRARY_ROOT, &new_root.to_string_lossy())?;
        std::fs::create_dir_all(self.mods_dir()).map_err(|e| AppError::io(self.mods_dir(), e))?;

        let redeployed = if restore {
            self.store.set_enabled(&enabled, true)?;
            self.apply()?
        } else {
            ApplyReport::default()
        };

        Ok(LibraryMoveReport {
            folders: self.folders()?,
            redeployed,
        })
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
        self.commit_install(staging_id, name, type_override, Origin::manual())
    }

    /// Install a file the download queue fetched from Nexus.
    ///
    /// Same pipeline as a hand-dropped archive, except the mod remembers where
    /// it came from, which is what later lets it be checked for updates.
    pub fn install_download(
        &mut self,
        archive: impl AsRef<Path>,
        name: &str,
        origin: Origin,
    ) -> Result<i64> {
        let staged = self.stage_archive(archive)?;
        let name = if name.trim().is_empty() {
            staged.suggested_name.clone()
        } else {
            name.to_string()
        };
        self.commit_install(&staged.staging_id, &name, None, origin)
    }

    // -- conflicts ---------------------------------------------------------

    /// Read what one mod replaces and record it.
    ///
    /// Only the container indexes are read, never the payload, so this costs a
    /// few kilobytes however large the mod is. A container that cannot be read
    /// is reported rather than skipped silently: an empty answer and an
    /// unreadable one look the same in a conflict list, and only one of them
    /// means "this mod replaces nothing".
    pub fn index_mod_assets(&mut self, mod_id: i64) -> Result<Vec<String>> {
        let record = self.record(mod_id)?;
        let root = self.staging_path(&record.staging_folder);

        let containers: Vec<PathBuf> = self
            .store
            .components_for(mod_id)?
            .iter()
            .flat_map(|c| c.files.iter())
            .map(|f| root.join(&f.source))
            .filter(|p| {
                matches!(
                    p.extension()
                        .map(|e| e.to_string_lossy().to_ascii_lowercase())
                        .as_deref(),
                    Some("pak") | Some("utoc")
                )
            })
            .collect();

        let (assets, problems) = sbmm_assets::read_all(containers.iter().map(|p| p.as_path()));

        let rows: Vec<(String, bool)> = assets
            .into_iter()
            .map(|asset| (asset.label().to_string(), asset.is_named()))
            .collect();
        self.store
            .replace_mod_assets(mod_id, &rows, problems.is_empty())?;

        Ok(problems.into_iter().map(|e| e.to_string()).collect())
    }

    /// Index every mod that has not been looked at yet.
    ///
    /// Existing installs predate the index, and a mod installed while the
    /// scan failed should get another chance rather than stay invisible.
    pub fn index_missing_assets(&mut self) -> Result<usize> {
        let pending = self.store.mods_without_assets()?;
        for mod_id in &pending {
            // One bad mod must not stop the rest from being indexed.
            let _ = self.index_mod_assets(*mod_id);
        }
        Ok(pending.len())
    }

    /// Which enabled mods are replacing the same assets, and who wins.
    ///
    /// `~mods` is mounted alphanumerically and a later `_P` pak overrides an
    /// earlier one, so the mod furthest down the load order is the one the
    /// game ends up loading. That is the same ordering the load-order screen
    /// shows, read the same way.
    pub fn conflicts(&self) -> Result<ConflictReport> {
        let records = self.store.list_mods()?;
        let enabled: BTreeMap<i64, &ModRecord> = records
            .iter()
            .filter(|m| m.enabled)
            .map(|m| (m.id, m))
            .collect();

        let mut report = ConflictReport {
            // No complete index means the picture for this mod is missing,
            // not empty — saying nothing would read as "conflicts with
            // nothing", which is the opposite of what is known.
            unreadable: records
                .iter()
                .filter(|m| m.enabled && m.assets_indexed_at.is_none())
                .map(|m| m.name.clone())
                .collect(),
            ..Default::default()
        };

        let mut losing: BTreeMap<i64, i64> = BTreeMap::new();
        let mut winning: BTreeMap<i64, i64> = BTreeMap::new();

        for contested in self.store.contested_assets()? {
            // An asset two disabled mods share is not a conflict: neither is
            // in the game folder.
            let mut claimants: Vec<&ModRecord> = contested
                .mod_ids
                .iter()
                .filter_map(|id| enabled.get(id).copied())
                .collect();
            if claimants.len() < 2 {
                continue;
            }
            claimants.sort_by_key(|m| (m.priority, m.id));

            let winner = claimants.last().map(|m| m.id);
            for record in &claimants {
                if Some(record.id) == winner {
                    *winning.entry(record.id).or_default() += 1;
                } else {
                    *losing.entry(record.id).or_default() += 1;
                }
            }

            report.conflicts.push(Conflict {
                asset: contested.asset,
                named: contested.named,
                claimants: claimants
                    .iter()
                    .map(|m| Claimant {
                        mod_id: m.id,
                        name: m.name.clone(),
                        priority: m.priority,
                        wins: Some(m.id) == winner,
                    })
                    .collect(),
            });
        }

        let mut counts: BTreeMap<i64, ModConflictCount> = BTreeMap::new();
        for (mod_id, count) in losing {
            counts
                .entry(mod_id)
                .or_insert(ModConflictCount {
                    mod_id,
                    losing: 0,
                    winning: 0,
                })
                .losing = count;
        }
        for (mod_id, count) in winning {
            counts
                .entry(mod_id)
                .or_insert(ModConflictCount {
                    mod_id,
                    losing: 0,
                    winning: 0,
                })
                .winning = count;
        }
        report.overridden = counts.into_values().collect();

        Ok(report)
    }

    // -- download queue ----------------------------------------------------

    /// The queue as the last run left it.
    ///
    /// Entries that had already finished are dropped: their file is installed
    /// and the row would only be clutter on the next start.
    pub fn restore_download_queue(&self) -> Result<Vec<sbmm_nexus::QueueItem>> {
        Ok(self
            .store
            .list_downloads()?
            .into_iter()
            .filter_map(|row| {
                let state = sbmm_nexus::DownloadState::from_str_id(&row.state);
                if matches!(state, sbmm_nexus::DownloadState::Done) {
                    return None;
                }
                Some(sbmm_nexus::QueueItem::restored(
                    row.id as u64,
                    row.mod_id as u64,
                    row.file_id as u64,
                    row.name,
                    row.file_name,
                    row.version,
                    state,
                    row.bytes_total.map(|b| b as u64),
                    row.error,
                    row.collection,
                ))
            })
            .collect())
    }

    /// Write the queue out so closing the manager does not lose it.
    pub fn save_download_queue(&mut self, items: &[sbmm_nexus::QueueItem]) -> Result<()> {
        let rows: Vec<sbmm_store::DownloadRecord> = items
            .iter()
            .map(|item| sbmm_store::DownloadRecord {
                id: item.id as i64,
                mod_id: item.mod_id as i64,
                file_id: item.file_id as i64,
                name: item.name.clone(),
                file_name: item.file_name.clone(),
                version: item.version.clone(),
                state: item.state.as_str().to_string(),
                bytes_done: item.bytes_done as i64,
                bytes_total: item.bytes_total.map(|b| b as i64),
                error: item.error.clone(),
                collection: item.collection.clone(),
            })
            .collect();
        self.store.replace_downloads(&rows)?;
        Ok(())
    }

    // -- upscalers ---------------------------------------------------------

    /// Which upscaler DLLs the game currently has, and their versions.
    pub fn upscalers(&self) -> Result<Vec<sbmm_upscaler::InstalledFile>> {
        let Some(game) = self.game()? else {
            return Ok(Vec::new());
        };
        Ok(sbmm_upscaler::scan(&game.root))
    }

    /// The upscaler swaps the manager itself has made, and their versions.
    pub fn managed_upscalers(&self) -> Result<Vec<(sbmm_upscaler::Component, String)>> {
        let mut out = Vec::new();
        for record in self.store.list_mods()? {
            let Some(component) = sbmm_upscaler::Component::all()
                .iter()
                .copied()
                .find(|c| upscaler_staging_id(*c) == record.staging_folder)
            else {
                continue;
            };
            out.push((component, record.version.unwrap_or_default()));
        }
        Ok(out)
    }

    /// Undo a swap, putting the game's own DLL back.
    ///
    /// Doing nothing when there is no swap is the right answer, not an error:
    /// the game's file is then already what is on disk.
    pub fn restore_upscaler(&mut self, component: sbmm_upscaler::Component) -> Result<bool> {
        let staging_id = upscaler_staging_id(component);
        let Some(installed) = self
            .store
            .list_mods()?
            .into_iter()
            .find(|m| m.staging_folder == staging_id)
        else {
            return Ok(false);
        };
        self.uninstall(installed.id)?;
        Ok(true)
    }

    /// Put a newer upscaler DLL in place of the game's own.
    ///
    /// Registered as an ordinary mod so the swap goes through the same
    /// deployment record as everything else: the displaced original is backed
    /// up, and removing the entry puts it back byte for byte. `files` pairs a
    /// downloaded DLL with the game-relative path it replaces — one entry per
    /// copy found, because the game may ship the same DLL twice and only one
    /// of them is the one the loader reaches.
    ///
    /// Installing over a previous swap removes it first, so the file that
    /// eventually gets restored is the game's, never one of ours.
    pub fn install_upscaler(
        &mut self,
        component: sbmm_upscaler::Component,
        version: &str,
        files: &[(PathBuf, PathBuf)],
    ) -> Result<i64> {
        if files.is_empty() {
            return Err(AppError::BadLibraryRoot(format!(
                "{} was not found in the game folder",
                component.label()
            )));
        }

        let staging_id = upscaler_staging_id(component);
        if let Some(previous) = self
            .store
            .list_mods()?
            .into_iter()
            .find(|m| m.staging_folder == staging_id)
        {
            self.uninstall(previous.id)?;
        }

        // The staging tree mirrors the game tree, which keeps two copies of
        // the same DLL from colliding on one file name.
        let staging_root = self.staging_path(&staging_id);
        let mut component_files = Vec::with_capacity(files.len());
        for (downloaded, target) in files {
            let staged = staging_root.join(target);
            if let Some(parent) = staged.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            std::fs::copy(downloaded, &staged).map_err(|e| AppError::io(&staged, e))?;
            component_files.push(sbmm_core::model::ComponentFile {
                source: target.clone(),
                target: target.clone(),
            });
        }

        let detected = DetectedComponent {
            mod_type: ModType::GameRootOverlay,
            confidence: Confidence::High,
            files: component_files,
            target_subdir: PathBuf::new(),
            ue4ss_mod_name: None,
            pak_sets: Vec::new(),
            warnings: Vec::new(),
            notes: vec![format!(
                "Replaces the game's {} in place; disabling this puts the original back.",
                component.file_name()
            )],
        };

        let name = format!("{} {version}", component.label());
        let id = self.store.insert_mod(&NewMod {
            name,
            staging_folder: staging_id,
            version: Some(version.to_string()),
            source: "upscaler".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            primary_type: ModType::GameRootOverlay.as_str().to_string(),
            components: vec![detected],
            size_bytes: sbmm_archive::directory_size(&staging_root) as i64,
            image_path: None,
        })?;

        // A staged upscaler does nothing at all, so it goes into the game
        // folder now rather than sitting in the pending list where "I updated
        // DLSS" and "DLSS is updated" would quietly disagree. Only this mod is
        // deployed; anything else the user has staged stays staged.
        self.set_enabled(&[id], true)?;
        let ctx = self.deploy_context()?;
        let record = self.record(id)?;
        let components = self.store.components_for(id)?;
        let plan = self.plan_for(&record, &components);
        self.deploy_mod(&ctx, id, &plan)?;

        Ok(id)
    }

    /// Commit a staged install, recording where the files came from.
    pub fn commit_install(
        &mut self,
        staging_id: &str,
        name: &str,
        type_override: Option<ModType>,
        origin: Origin,
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
            version: origin.version,
            source: origin.source,
            nexus_mod_id: origin.nexus_mod_id,
            nexus_file_id: origin.nexus_file_id,
            primary_type,
            components,
            size_bytes: sbmm_archive::directory_size(&dest) as i64,
            image_path: None,
        })?;

        // Read what it replaces now, while the containers are to hand. A
        // failure here is not worth failing the install over — the mod is
        // installed either way, it just has no conflict picture yet, and
        // `index_missing_assets` will come back to it.
        let _ = self.index_mod_assets(id);

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
                latest_version: newer_version(&record.version, &record.latest_version),
                nexus_mod_id: record.nexus_mod_id,
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
            nexus: self.nexus_account()?,
            update_channel: self.update_channel()?,
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

    // -- update checks -----------------------------------------------------

    /// Every installed mod that came from Nexus, so its page can be queried.
    pub fn nexus_mods(&self) -> Result<Vec<NexusModRef>> {
        Ok(self
            .store
            .list_mods()?
            .into_iter()
            .filter_map(|m| {
                Some(NexusModRef {
                    id: m.id,
                    nexus_mod_id: m.nexus_mod_id?,
                    version: m.version,
                    latest_version: m.latest_version,
                })
            })
            .collect())
    }

    /// Remember what Nexus said the newest version is.
    pub fn record_update_check(&self, mod_id: i64, latest: Option<&str>) -> Result<()> {
        self.store.record_update_check(mod_id, latest)?;
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

/// An installed mod that can be looked up on Nexus.
#[derive(Debug, Clone)]
pub struct NexusModRef {
    /// The row id in our database, not the Nexus one.
    pub id: i64,
    pub nexus_mod_id: i64,
    pub version: Option<String>,
    /// What the last update check found, if there has been one.
    pub latest_version: Option<String>,
}

impl NexusModRef {
    /// Whether this mod is currently showing an update.
    pub fn is_outdated(&self) -> bool {
        newer_version(&self.version, &self.latest_version).is_some()
    }
}

/// The version to show as available, or nothing.
///
/// Version strings on Nexus are free text, so no ordering is inferred: an
/// update is reported only when both versions are known and differ, which is
/// the same rule other managers use. Guessing which of `1.0a` and `1.1-beta`
/// is newer would produce confident wrong answers.
pub fn newer_version(installed: &Option<String>, latest: &Option<String>) -> Option<String> {
    let installed = installed.as_deref()?;
    let latest = latest.as_deref()?;
    if normalise_version(installed) == normalise_version(latest) {
        return None;
    }
    Some(latest.to_string())
}

fn normalise_version(version: &str) -> String {
    version
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_ascii_lowercase()
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

/// Paths cross to the UI as strings, so lossy conversion happens in one place.
/// One staging folder per upscaler component, so installing a newer version
/// replaces the previous swap instead of stacking on top of it.
fn upscaler_staging_id(component: sbmm_upscaler::Component) -> String {
    format!("upscaler-{}", component.file_name().replace(".dll", ""))
}

fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn some(value: &str) -> Option<String> {
        Some(value.to_string())
    }

    #[test]
    fn an_update_is_reported_only_when_both_versions_are_known_and_differ() {
        assert_eq!(newer_version(&some("1.2"), &some("1.4")), some("1.4"));
        assert_eq!(newer_version(&some("1.2"), &some("1.2")), None);
        // Nothing to compare against is not the same as being up to date.
        assert_eq!(newer_version(&None, &some("1.4")), None);
        assert_eq!(newer_version(&some("1.2"), &None), None);
    }

    #[test]
    fn cosmetic_differences_do_not_count_as_an_update() {
        assert_eq!(newer_version(&some("v1.2"), &some("1.2")), None);
        assert_eq!(newer_version(&some(" 1.2 "), &some("1.2")), None);
        assert_eq!(newer_version(&some("1.2B"), &some("1.2b")), None);
    }
}
