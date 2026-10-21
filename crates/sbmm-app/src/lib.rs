//! The application service.
//!
//! All of the manager's behaviour lives here rather than in the Tauri layer,
//! which keeps it compilable and testable on any platform; `src-tauri` is only
//! a set of thin command wrappers around this type.

pub mod credentials;
pub mod diagnostics;
pub mod dto;
pub mod installer;
pub mod library;
pub mod ue4ss;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sbmm_core::model::{Confidence, DetectedComponent, ModType};
use sbmm_core::plan::{self, build_plan, DeployPlan};
use sbmm_core::{detect_components, FileTree};
use sbmm_deploy::backend::DeployContext;
use sbmm_deploy::{ue4ss as mods_txt, DeployBackend, HardlinkBackend, Manifest};
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
    #[error(transparent)]
    Vfs(#[from] sbmm_vfs::MappingError),
    #[error("{0}")]
    Invalid(String),
    /// A download holds several versions of the mod and the user has not
    /// chosen before; it is staged, waiting to be asked about.
    #[error("{} holds several versions; choose which to install", .0.suggested_name)]
    NeedsChoice(Box<StagedInstall>),
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
const SETTING_DISCORD_RPC: &str = "discordRpc";
const SETTING_DEPLOYMENT_MODE: &str = "deploymentMode";

pub struct App {
    store: Store,
    data_dir: PathBuf,
    backend: HardlinkBackend,
    keys: Box<dyn credentials::KeyStore>,
    /// Where staged installs waiting on the user came from, so a Nexus
    /// download that stops to ask which variant to install still remembers
    /// it is a Nexus mod once the user answers.
    pending_origins: std::collections::HashMap<String, Origin>,
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
            pending_origins: std::collections::HashMap::new(),
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

    /// Whether the manager tells Discord what it is doing.
    ///
    /// On by default, because that is what a presence integration is for, and
    /// off with one switch — it broadcasts to the user's friends list, so it
    /// has to be theirs to turn off.
    pub fn discord_rpc(&self) -> Result<bool> {
        Ok(self
            .store
            .get_setting(SETTING_DISCORD_RPC)?
            .map(|v| v != "off")
            .unwrap_or(true))
    }

    pub fn set_discord_rpc(&mut self, enabled: bool) -> Result<()> {
        self.store
            .set_setting(SETTING_DISCORD_RPC, if enabled { "on" } else { "off" })?;
        Ok(())
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
            overwrite: path_string(self.overwrite_dir()),
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
        library::move_tree(&old_root.join("overwrite"), &new_root.join("overwrite"))?;

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
        // A FOMOD says itself what to ask; guessing at variants on top of it
        // would ask twice.
        let fomod = self.fomod_view(&staging_id, &suggested_name, None)?;
        let variants = if fomod.is_some() {
            None
        } else {
            sbmm_core::variant_choice(&tree)
        };
        // An archive of alternatives always asks: installing every colour of
        // an outfit at once is never what anyone meant.
        let needs_confirmation = variants.is_some()
            || fomod.is_some()
            || components
                .iter()
                .any(|c| c.confidence == Confidence::AskUser);
        let remembered_variants = match &variants {
            Some(choice) => self
                .remembered_variants(&suggested_name, None)?
                .filter(|chosen| choice.folders_to_drop(chosen).is_some())
                .unwrap_or_default(),
            None => Vec::new(),
        };

        Ok(StagedInstall {
            staging_id,
            suggested_name,
            size_bytes: sbmm_archive::directory_size(&dest) as i64,
            components,
            needs_confirmation,
            variants,
            remembered_variants,
            fomod,
            previews: installer::previews(&dest),
        })
    }

    /// Commit a staged install, optionally overriding the detected type.
    pub fn confirm_install(
        &mut self,
        staging_id: &str,
        name: &str,
        type_override: Option<ModType>,
    ) -> Result<i64> {
        self.confirm_install_with(staging_id, name, type_override, None)
    }

    /// Commit a staged install, keeping only the chosen `variants` (option
    /// labels) of an archive that offers several.
    ///
    /// The other options' folders are deleted from staging before the commit,
    /// so detection sees only what was chosen and the library does not keep
    /// colours nobody will use. The choice is remembered on the mod.
    pub fn confirm_install_with(
        &mut self,
        staging_id: &str,
        name: &str,
        type_override: Option<ModType>,
        variants: Option<&[String]>,
    ) -> Result<i64> {
        if let Some(chosen) = variants {
            self.keep_variants(staging_id, chosen)?;
        }
        let origin = self
            .pending_origins
            .remove(staging_id)
            .unwrap_or_else(Origin::manual);
        let id = self.commit_install(staging_id, name, type_override, origin)?;
        if let Some(chosen) = variants {
            self.store
                .set_variant_choice(id, Some(&serde_json::to_string(chosen)?))?;
        }
        Ok(id)
    }

    /// Delete every option folder of a staged multi-variant archive except
    /// the chosen ones.
    fn keep_variants(&self, staging_id: &str, chosen: &[String]) -> Result<()> {
        let dest = self.staging_path(staging_id);
        let tree = FileTree::new(sbmm_archive::walk_relative(&dest));
        let Some(choice) = sbmm_core::variant_choice(&tree) else {
            return Err(AppError::Invalid(
                "this archive does not offer a choice of versions".into(),
            ));
        };
        let drop = choice.folders_to_drop(chosen).ok_or_else(|| {
            AppError::Invalid("choose at least one of the versions the archive offers".into())
        })?;
        for folder in drop {
            let path = dest.join(&folder);
            std::fs::remove_dir_all(&path).map_err(|e| AppError::io(&path, e))?;
        }
        Ok(())
    }

    /// The variants chosen when this mod was last installed: matched by its
    /// Nexus id when there is one, otherwise by name.
    fn remembered_variants(
        &self,
        name: &str,
        nexus_mod_id: Option<i64>,
    ) -> Result<Option<Vec<String>>> {
        let records = self.store.list_mods()?;
        let with_choice = || records.iter().filter(|m| m.variant_choice.is_some());
        let found = nexus_mod_id
            .and_then(|id| with_choice().find(|m| m.nexus_mod_id == Some(id)))
            .or_else(|| with_choice().find(|m| m.name.eq_ignore_ascii_case(name.trim())));
        Ok(found
            .and_then(|m| m.variant_choice.as_deref())
            .and_then(|json| serde_json::from_str(json).ok()))
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
        let mut staged = self.stage_archive(archive)?;
        let name = if name.trim().is_empty() {
            staged.suggested_name.clone()
        } else {
            name.to_string()
        };
        if staged.fomod.is_some() {
            // An update installs with last time's answers; a first install
            // waits for the wizard, remembering where it came from.
            self.pending_origins
                .insert(staged.staging_id.clone(), origin.clone());
            if let Some(id) =
                self.fomod_with_remembered(&staged.staging_id, &name, origin.nexus_mod_id)?
            {
                return Ok(id);
            }
            staged.suggested_name = name;
            return Err(AppError::NeedsChoice(Box::new(staged)));
        }
        if let Some(choice) = &staged.variants {
            // An update of a mod installed before goes in the way it did then;
            // anything else waits for the user, keeping where it came from.
            let remembered = self
                .remembered_variants(&name, origin.nexus_mod_id)?
                .filter(|chosen| choice.folders_to_drop(chosen).is_some());
            match remembered {
                Some(chosen) => {
                    self.pending_origins
                        .insert(staged.staging_id.clone(), origin);
                    return self.confirm_install_with(
                        &staged.staging_id,
                        &name,
                        None,
                        Some(&chosen),
                    );
                }
                None => {
                    staged.suggested_name = name;
                    self.pending_origins
                        .insert(staged.staging_id.clone(), origin);
                    return Err(AppError::NeedsChoice(Box::new(staged)));
                }
            }
        }
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
        let plan = self.deploy_plan_for(&record, &components)?;
        self.deploy_mod(&ctx, id, &plan)?;

        Ok(id)
    }

    // -- frame generation on RTX 20/30 (experimental) ----------------------

    /// What the game folder looks like to the frame generation check, or
    /// nothing when no game has been chosen.
    pub fn framegen_facts(&self) -> Result<Option<sbmm_upscaler::framegen::GameFacts>> {
        use sbmm_upscaler::framegen::Owner;

        let Some(game) = self.game()? else {
            return Ok(None);
        };
        let records = self.store.list_mods()?;
        let owners: BTreeMap<String, Owner> = self
            .store
            .deployed_files()?
            .into_iter()
            .filter_map(|(mod_id, target, _, _)| {
                let record = records.iter().find(|m| m.id == mod_id)?;
                let owner = if record.staging_folder == FRAMEGEN_STAGING {
                    Owner::Ours
                } else {
                    Owner::Mod(record.name.clone())
                };
                Some((comparable_path(&target), owner))
            })
            .collect();

        Ok(Some(sbmm_upscaler::framegen::gather(
            &game.root,
            Path::new(sbmm_core::paths::BINARIES_WIN64),
            game.executable.clone(),
            |relative| {
                owners
                    .get(&comparable_path(&relative.to_string_lossy()))
                    .cloned()
                    .unwrap_or(Owner::Unmanaged)
            },
        )))
    }

    /// The frame generation package the manager installed, if any.
    pub fn framegen_installed(&self) -> Result<Option<FramegenInstall>> {
        use sbmm_upscaler::framegen::Proxy;

        let Some(record) = self
            .store
            .list_mods()?
            .into_iter()
            .find(|m| m.staging_folder == FRAMEGEN_STAGING)
        else {
            return Ok(None);
        };
        // Which proxy is in use is read back from the files, so there is one
        // record of it rather than a setting that could disagree.
        let proxy = self
            .store
            .components_for(record.id)?
            .iter()
            .flat_map(|c| c.files.iter())
            .find_map(|f| {
                let parent = comparable_path(&f.target.parent()?.to_string_lossy());
                (parent == comparable_path(sbmm_core::paths::BINARIES_WIN64))
                    .then(|| Proxy::from_file_name(&f.target.file_name()?.to_string_lossy()))
                    .flatten()
            })
            .unwrap_or_default();
        Ok(Some(FramegenInstall {
            mod_id: record.id,
            version: record.version.unwrap_or_default(),
            proxy,
            enabled: record.enabled,
        }))
    }

    /// Lay an unpacked, inspected `dlssg_for_sm86` release beside the game
    /// executable under `proxy`.
    ///
    /// Registered as an ordinary mod, like the upscaler swaps, so whatever it
    /// displaces is backed up and removing it restores the folder exactly.
    /// The package's files are moved into staging rather than copied — the
    /// release is over 200 MB unpacked and the caller discards it afterwards.
    pub fn install_framegen(
        &mut self,
        package: &sbmm_upscaler::framegen::Package,
        proxy: sbmm_upscaler::framegen::Proxy,
        version: &str,
    ) -> Result<i64> {
        if let Some(previous) = self.framegen_installed()? {
            self.uninstall(previous.mod_id)?;
        }

        let staging_root = self.staging_path(FRAMEGEN_STAGING);
        let binaries = Path::new(sbmm_core::paths::BINARIES_WIN64);
        let mut component_files = Vec::new();
        for (source, relative) in sbmm_upscaler::framegen::files_to_deploy(package, proxy) {
            let target = binaries.join(&relative);
            let staged = staging_root.join(&target);
            if let Some(parent) = staged.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            move_or_copy(&package.root.join(&source), &staged)?;
            component_files.push(sbmm_core::model::ComponentFile {
                source: target.clone(),
                target,
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
                "Experimental: dlssg_for_sm86 loaded through {}. Disabling this removes \
                 the proxy and puts back anything it displaced.",
                proxy.file_name()
            )],
        };

        let id = self.store.insert_mod(&NewMod {
            name: format!("DLSS Frame Generation for RTX 20/30 {version}"),
            staging_folder: FRAMEGEN_STAGING.to_string(),
            version: Some(version.to_string()),
            source: "framegen".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            primary_type: ModType::GameRootOverlay.as_str().to_string(),
            components: vec![detected],
            size_bytes: sbmm_archive::directory_size(&staging_root) as i64,
            image_path: None,
        })?;

        // Same reasoning as the upscalers: a staged frame generator does
        // nothing, so it goes in now and only this mod is deployed.
        self.set_enabled(&[id], true)?;
        let ctx = self.deploy_context()?;
        let record = self.record(id)?;
        let components = self.store.components_for(id)?;
        let plan = self.deploy_plan_for(&record, &components)?;
        self.deploy_mod(&ctx, id, &plan)?;

        Ok(id)
    }

    /// Take the frame generation package back out. Nothing installed is not
    /// an error: the game folder is already as it should be.
    pub fn remove_framegen(&mut self) -> Result<bool> {
        let Some(installed) = self.framegen_installed()? else {
            return Ok(false);
        };
        self.uninstall(installed.mod_id)?;
        Ok(true)
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
            // The best picture the archive carried, shown as the mod's.
            image_path: installer::previews(&dest).into_iter().next(),
        })?;

        // Read what it replaces now, while the containers are to hand. A
        // failure here is not worth failing the install over — the mod is
        // installed either way, it just has no conflict picture yet, and
        // `index_missing_assets` will come back to it.
        let _ = self.index_mod_assets(id);

        Ok(id)
    }

    pub fn cancel_install(&mut self, staging_id: &str) -> Result<()> {
        self.pending_origins.remove(staging_id);
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

        let virtual_mode = self.deployment_mode()? == DeploymentMode::Virtual;
        let file_conflicts = self.file_conflict_counts(&records)?;
        let ue4ss_loads = self.ue4ss_presence(&records)?.loads();
        for record in &records {
            let components = self.store.components_for(record.id)?;
            let planned = self.deploy_plan_for(record, &components)?;
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
                // Under the virtual file system an enabled mod is in effect
                // whether or not any of its files had to be linked in.
                deployed: deployed || (virtual_mode && record.enabled),
                priority: record.priority,
                size_bytes: record.size_bytes,
                source: record.source.clone(),
                installed_at: record.installed_at.clone(),
                warnings: components.iter().flat_map(|c| c.warnings.clone()).collect(),
                latest_version: newer_version(&record.version, &record.latest_version),
                nexus_mod_id: record.nexus_mod_id,
                file_conflicts: file_conflicts.get(&record.id).cloned(),
                missing_requirements: if record.enabled
                    && !ue4ss_loads
                    && ue4ss::requires_ue4ss(&components)
                {
                    vec!["UE4SS".to_string()]
                } else {
                    Vec::new()
                },
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
            discord_rpc: self.discord_rpc()?,
            deployment_mode: self.deployment_mode()?,
            profiles: self.store.list_profiles()?,
            overwrite: self.overwrite_summary(),
            ue4ss: self.ue4ss_status()?,
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

    // -- profiles ----------------------------------------------------------

    pub fn profiles(&self) -> Result<Vec<sbmm_store::ProfileRecord>> {
        Ok(self.store.list_profiles()?)
    }

    /// The profile a command line or shortcut names. Case is ignored in any
    /// script, since the name was typed or carried over from a rename.
    pub fn profile_named(&self, name: &str) -> Result<sbmm_store::ProfileRecord> {
        let wanted = name.trim().to_lowercase();
        let profiles = self.store.list_profiles()?;
        let names = profiles
            .iter()
            .map(|p| format!("“{}”", p.name))
            .collect::<Vec<_>>()
            .join(", ");
        profiles
            .into_iter()
            .find(|p| p.name.to_lowercase() == wanted)
            .ok_or_else(|| {
                AppError::Invalid(format!(
                    "there is no profile called “{}” (there are {names})",
                    name.trim()
                ))
            })
    }

    /// A new profile from the active one: its load order always, its enabled
    /// set too when `copy_enabled` (a duplicate), otherwise nothing enabled.
    pub fn create_profile(&mut self, name: &str, copy_enabled: bool) -> Result<i64> {
        let name = self.valid_profile_name(name, None)?;
        Ok(self.store.create_profile(&name, None, copy_enabled)?)
    }

    pub fn rename_profile(&mut self, id: i64, name: &str) -> Result<()> {
        let name = self.valid_profile_name(name, Some(id))?;
        Ok(self.store.rename_profile(id, &name)?)
    }

    pub fn delete_profile(&mut self, id: i64) -> Result<()> {
        Ok(self.store.delete_profile(id)?)
    }

    /// Make another profile active and bring the game folder in line with it.
    ///
    /// Enabled state and load order are per profile while the deployment
    /// record is not, so the ordinary reconciliation does the switch: it
    /// removes what the new profile does not enable, adds what it does, and
    /// renames paks whose position changed. Under the virtual file system
    /// that touches only the files that have to be real; the rest follows at
    /// the next launch.
    pub fn switch_profile(&mut self, id: i64) -> Result<ApplyReport> {
        if id == self.store.active_profile()? {
            return Ok(ApplyReport::default());
        }
        self.store.set_active_profile(id)?;
        if self.game()?.is_some() {
            self.apply()
        } else {
            Ok(ApplyReport::default())
        }
    }

    fn valid_profile_name(&self, name: &str, renaming: Option<i64>) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("a profile needs a name".into()));
        }
        let taken = self
            .store
            .list_profiles()?
            .into_iter()
            .any(|p| Some(p.id) != renaming && p.name.eq_ignore_ascii_case(name));
        if taken {
            return Err(AppError::Invalid(format!(
                "there is already a profile called “{name}”"
            )));
        }
        Ok(name.to_string())
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
            let planned = self.deploy_plan_for(&record, &components)?;
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

        // Under the virtual file system `mods.txt` is generated into the
        // view at launch; writing the real one would defeat the point.
        if self.deployment_mode()? == DeploymentMode::Hardlink {
            self.sync_ue4ss_registrations(&ctx)?;
        }
        Ok(report)
    }

    /// The part of a mod's plan that goes into the game folder for real.
    ///
    /// Everything, with hard links. Under the virtual file system only what
    /// the Windows loader reaches before the hook exists: proxy DLLs and the
    /// rest of the binaries folder, UE4SS itself, and the DLL swaps the
    /// manager makes — the same files Mod Organizer 2 needs Root Builder for.
    fn deploy_plan_for(
        &self,
        record: &ModRecord,
        components: &[DetectedComponent],
    ) -> Result<DeployPlan> {
        let mut plan = self.plan_for(record, components);
        if self.deployment_mode()? == DeploymentMode::Virtual {
            plan.files
                .retain(|file| must_be_physical(&record.source, &file.target));
        }
        Ok(plan)
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
            mods_txt::sync_mods_txt(&ctx.game_root, &ctx.backup_root, &states)?;
        } else {
            // Nothing left for UE4SS to load: take our mods.txt back out and
            // let the folders it lived in go with it.
            mods_txt::clear_registrations(&ctx.game_root, &ctx.backup_root)?;
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

    // -- virtual file system -----------------------------------------------

    pub fn deployment_mode(&self) -> Result<DeploymentMode> {
        Ok(
            match self.store.get_setting(SETTING_DEPLOYMENT_MODE)?.as_deref() {
                Some("virtual") => DeploymentMode::Virtual,
                _ => DeploymentMode::Hardlink,
            },
        )
    }

    /// Switch how mods reach the game, carrying every enabled mod across.
    ///
    /// Everything is taken out under the old mode and put back under the new
    /// one, through the ordinary reconciliation, so the game folder passes
    /// through its untouched state in between and neither mode inherits the
    /// other's files — least of all a `mods.txt` the virtual view would
    /// otherwise be generated on top of.
    pub fn set_deployment_mode(&mut self, mode: DeploymentMode) -> Result<ApplyReport> {
        if mode == self.deployment_mode()? {
            return Ok(ApplyReport::default());
        }
        let enabled = self.store.enabled_mod_ids()?;
        let game = self.game()?.is_some();
        if game && !enabled.is_empty() {
            self.store.set_enabled(&enabled, false)?;
            let cleared = self.apply();
            // Whatever happened, the user's choices are not lost.
            self.store.set_enabled(&enabled, true)?;
            cleared?;
        }
        self.store
            .set_setting(SETTING_DEPLOYMENT_MODE, mode.as_str())?;
        if game {
            self.apply()
        } else {
            Ok(ApplyReport::default())
        }
    }

    /// Everything needed to start the game under the virtual file system.
    ///
    /// Brings the real part of the deployment up to date first, then writes
    /// the mapping, so what the game sees is what the mod list says.
    pub fn launch_options(&mut self, hook: PathBuf) -> Result<sbmm_vfs::launch::LaunchOptions> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let executable = game
            .executable_path()
            .ok_or_else(|| AppError::BadLibraryRoot("the game executable was not found".into()))?;
        self.apply()?;
        let mapping = self.write_vfs_mapping()?;

        // Started outside Steam, a Steam game restarts itself through Steam —
        // and out from under the hook — unless it is told its app id.
        let mut environment = Vec::new();
        if game.source == sbmm_game::InstallSource::Steam {
            for name in ["SteamAppId", "SteamGameId"] {
                environment.push((name.to_string(), sbmm_game::STEAM_APP_ID.to_string()));
            }
        }

        Ok(sbmm_vfs::launch::LaunchOptions {
            working_dir: game.binaries_dir(),
            executable,
            arguments: Vec::new(),
            environment,
            hook,
            mapping,
            log: Some(self.vfs_dir().join("hook.log")),
            diagnostics: Some(self.vfs_dir().join("launch.log")),
        })
    }

    /// Where files the game creates while running under the virtual file
    /// system are kept — Mod Organizer 2's "overwrite" folder.
    pub fn overwrite_dir(&self) -> PathBuf {
        self.library_root().join("overwrite")
    }

    /// Where the mapping and the files the manager generates for the game
    /// are written before a launch.
    pub fn vfs_dir(&self) -> PathBuf {
        self.library_root().join("vfs")
    }

    pub fn vfs_mapping_path(&self) -> PathBuf {
        self.vfs_dir().join("mapping.json")
    }

    /// The game folder as the enabled mods would make it, described as layers
    /// rather than written into the folder.
    ///
    /// Built from exactly the plans the hard-link backend deploys, stacked in
    /// the same order the conflicts view reads — lowest priority first, so
    /// the mod furthest down the load order wins — which is what makes the
    /// two ways of running the game indistinguishable to it. UE4SS's
    /// `mods.txt` is generated from the real one, so the user's own entries
    /// carry over without the real file being rewritten.
    pub fn vfs_mapping(&self) -> Result<sbmm_vfs::Mapping> {
        let game = self.game()?.ok_or(AppError::NoGame)?;
        let mut records = self.store.list_mods()?;
        records.sort_by_key(|m| (m.priority, m.id));

        let mut layers = Vec::new();
        let mut registrations: BTreeMap<String, bool> = BTreeMap::new();
        for record in &records {
            let components = self.store.components_for(record.id)?;
            let plan = self.plan_for(record, &components);
            for name in &plan.ue4ss_registrations {
                // If two mods share a folder name, enabled wins.
                let entry = registrations.entry(name.clone()).or_insert(false);
                *entry = *entry || record.enabled;
            }
            if !record.enabled {
                continue;
            }
            layers.push(sbmm_vfs::Layer {
                origin: sbmm_vfs::Origin::Mod(record.id),
                name: record.name.clone(),
                files: plan
                    .files
                    .iter()
                    .map(|f| sbmm_vfs::LayerFile {
                        target: f.target.clone(),
                        source: plan.staging_root.join(&f.source),
                    })
                    // A planned path that is a folder, or has gone missing,
                    // is not something the game could open anyway.
                    .filter(|f| f.source.is_file())
                    .collect(),
            });
        }

        if registrations.values().any(|enabled| *enabled) {
            // Start from the `mods.txt` the game would otherwise see: the one
            // the highest enabled mod ships — UE4SS's own, which switches on
            // BPModLoaderMod and its other bundled mods — or else the real
            // one. Starting from the real file alone would drop those, and
            // logic mods need BPModLoaderMod to load at all.
            let mods_txt = comparable_path(sbmm_core::paths::UE4SS_MODS_TXT);
            let shipped = layers
                .iter()
                .rev()
                .flat_map(|layer| layer.files.iter())
                .find(|file| comparable_path(&file.target.to_string_lossy()) == mods_txt)
                .map(|file| file.source.clone());
            let real = shipped.unwrap_or_else(|| game.root.join(sbmm_core::paths::UE4SS_MODS_TXT));
            let real = std::fs::read_to_string(real).unwrap_or_default();
            let generated = self
                .vfs_dir()
                .join("generated")
                .join(sbmm_core::paths::UE4SS_MODS_TXT);
            if let Some(parent) = generated.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            std::fs::write(&generated, mods_txt::render_mods_txt(&real, &registrations))
                .map_err(|e| AppError::io(&generated, e))?;
            layers.push(sbmm_vfs::Layer {
                origin: sbmm_vfs::Origin::Generated,
                name: "UE4SS mods.txt".into(),
                files: vec![sbmm_vfs::LayerFile {
                    target: PathBuf::from(sbmm_core::paths::UE4SS_MODS_TXT),
                    source: generated,
                }],
            });
        }

        Ok(sbmm_vfs::Mapping::new(
            game.root,
            self.overwrite_dir(),
            layers,
        ))
    }

    /// Write the mapping where the hook will look for it, and return its path.
    pub fn write_vfs_mapping(&self) -> Result<PathBuf> {
        let mapping = self.vfs_mapping()?;
        std::fs::create_dir_all(self.overwrite_dir())
            .map_err(|e| AppError::io(self.overwrite_dir(), e))?;
        let path = self.vfs_mapping_path();
        mapping.save(&path)?;
        Ok(path)
    }

    /// One directory of the virtual file tree, in the manner of Mod Organizer
    /// 2's "Data" tab: the merged view the game would see, with who provides
    /// each entry and who it hides.
    ///
    /// `dir` is a game-relative path; empty is the root. The real game folder
    /// is read from disk and merged with the overlay, so an entry the game
    /// ships but no mod touches is shown too, attributed to the game.
    pub fn vfs_tree(&self, dir: &str) -> Result<VfsListing> {
        use sbmm_vfs::{Origin, VPath};

        let Some(game) = self.game()? else {
            return Ok(VfsListing {
                dir: String::new(),
                entries: Vec::new(),
            });
        };
        let (vfs, labels) = self.labelled_vfs()?;
        let label = |origin: Origin| labels.label(origin);
        let overlay = vfs.overlay();

        let dir_path = if dir.is_empty() {
            VPath::root()
        } else {
            VPath::from_relative(dir).ok_or_else(|| {
                AppError::BadLibraryRoot(format!("{dir} is not a path inside the game folder"))
            })?
        };

        // The real folder on disk, in its own spelling, so game files keep
        // their case and can be opened.
        let mut real_dir = game.root.clone();
        for component in dir_path.components() {
            real_dir.push(component);
        }
        let real: Vec<(String, bool)> = std::fs::read_dir(&real_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().map(|k| k.is_dir()).unwrap_or(false);
                (name, is_dir)
            })
            .collect();

        let mut entries = Vec::new();
        for listed in overlay.list(&dir_path, real) {
            let child = dir_path.join(&listed.name);
            let entry = match overlay.file(&child) {
                Some(file) => {
                    let (origin, provided_by, mod_id) = label(file.origin);
                    VfsEntry {
                        name: listed.name,
                        is_dir: false,
                        path: child.display().to_string(),
                        provided_by,
                        origin,
                        mod_id,
                        shadowed_by: file.shadowed.iter().map(|o| label(*o).1).collect(),
                        source: Some(path_string(&file.source)),
                    }
                }
                None => VfsEntry {
                    name: listed.name.clone(),
                    is_dir: listed.is_dir,
                    path: child.display().to_string(),
                    provided_by: "Game".into(),
                    origin: "game".into(),
                    mod_id: None,
                    shadowed_by: Vec::new(),
                    source: (!listed.is_dir).then(|| path_string(real_dir.join(&listed.name))),
                },
            };
            entries.push(entry);
        }

        // Folders first, then files, each case-insensitively by name — the
        // order a file browser shows and the load-order screen does not care
        // about here.
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(VfsListing {
            dir: dir_path.display().to_string(),
            entries,
        })
    }

    /// What is waiting in the overwrite folder.
    pub fn overwrite_summary(&self) -> OverwriteView {
        let dir = self.overwrite_dir();
        OverwriteView {
            path: path_string(&dir),
            files: sbmm_archive::walk_relative(&dir).len() as u32,
            bytes: sbmm_archive::directory_size(&dir) as i64,
        }
    }

    /// Turn everything in overwrite into a mod of its own, as Mod Organizer 2
    /// does: the files move out of overwrite into a new mod, which is enabled
    /// at the end of the load order — the same place overwrite sat, above
    /// every mod — so what the game sees does not change.
    pub fn create_mod_from_overwrite(&mut self, name: &str) -> Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("the new mod needs a name".into()));
        }
        let overwrite = self.overwrite_dir();
        let files = sbmm_archive::walk_relative(&overwrite);
        if files.is_empty() {
            return Err(AppError::Invalid("the overwrite folder is empty".into()));
        }

        let staging_id = self.unique_staging_folder(name);
        let staging = self.staging_path(&staging_id);
        let mut component_files = Vec::with_capacity(files.len());
        for relative in &files {
            let from = overwrite.join(relative);
            let to = staging.join(relative);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            move_or_copy(&from, &to)?;
            // A copy across volumes leaves the original behind; it has to go,
            // or the file would be in the view twice.
            if from.exists() {
                std::fs::remove_file(&from).map_err(|e| AppError::io(&from, e))?;
            }
            // Overwrite already mirrors the game folder, so each file goes
            // exactly where it came from.
            component_files.push(sbmm_core::model::ComponentFile {
                source: relative.clone(),
                target: relative.clone(),
            });
        }
        clear_dir(&overwrite)?;

        let detected = DetectedComponent {
            mod_type: ModType::GameRootOverlay,
            confidence: Confidence::High,
            files: component_files,
            target_subdir: PathBuf::new(),
            ue4ss_mod_name: None,
            pak_sets: Vec::new(),
            warnings: Vec::new(),
            notes: vec![
                "Files the game created under the virtual file system, kept as a mod.".into(),
            ],
        };
        let id = self.store.insert_mod(&NewMod {
            name: name.to_string(),
            staging_folder: staging_id,
            version: None,
            source: "overwrite".into(),
            nexus_mod_id: None,
            nexus_file_id: None,
            primary_type: ModType::GameRootOverlay.as_str().to_string(),
            components: vec![detected],
            size_bytes: sbmm_archive::directory_size(&staging) as i64,
            image_path: None,
        })?;
        self.store.set_enabled(&[id], true)?;
        Ok(id)
    }

    /// Delete everything in overwrite.
    pub fn clear_overwrite(&mut self) -> Result<u32> {
        let dir = self.overwrite_dir();
        let count = sbmm_archive::walk_relative(&dir).len() as u32;
        clear_dir(&dir)?;
        Ok(count)
    }

    /// Every path more than one provider claims, in the manner of Mod
    /// Organizer 2's conflicts tab: who wins it and who is hidden beneath.
    /// With `mod_id`, only the paths that mod takes part in, either side.
    pub fn vfs_conflicts(&self, mod_id: Option<i64>) -> Result<Vec<VfsEntry>> {
        if self.game()?.is_none() {
            return Ok(Vec::new());
        }
        let (vfs, labels) = self.labelled_vfs()?;
        let wanted = mod_id.map(sbmm_vfs::Origin::Mod);
        let mut entries: Vec<VfsEntry> = vfs
            .overlay()
            .conflicts()
            .filter(|(_, file)| {
                wanted.is_none_or(|o| file.origin == o || file.shadowed.contains(&o))
            })
            .map(|(path, file)| {
                let (origin, provided_by, mod_id) = labels.label(file.origin);
                VfsEntry {
                    name: path.file_name().to_string(),
                    is_dir: false,
                    path: path.display().to_string(),
                    provided_by,
                    origin,
                    mod_id,
                    shadowed_by: file.shadowed.iter().map(|o| labels.label(*o).1).collect(),
                    source: Some(path_string(&file.source)),
                }
            })
            .collect();
        entries.sort_by_key(|e| e.path.to_lowercase());
        Ok(entries)
    }

    /// The live virtual view, with a display name for every provider in it.
    fn labelled_vfs(&self) -> Result<(sbmm_vfs::Vfs, OriginLabels)> {
        let mapping = self.vfs_mapping()?;
        // Resolved before the mapping is turned into the live view, which
        // keeps the files but not the layer names.
        let mut names = std::collections::HashMap::new();
        for layer in &mapping.layers {
            names.insert(layer.origin, layer.name.clone());
        }
        names.insert(sbmm_vfs::Origin::Overwrite, "Overwrite".to_string());
        Ok((mapping.into_vfs(), OriginLabels(names)))
    }

    /// How each enabled mod's loose files fare against the other enabled
    /// mods: paths it wins because it is further down the order, and paths a
    /// mod further down takes from it. Paks never clash here — their names
    /// carry the load order — so this is the file-level picture that asset
    /// conflicts inside paks do not cover. Read from the plans alone, so it
    /// is cheap enough for every snapshot.
    fn file_conflict_counts(
        &self,
        records: &[ModRecord],
    ) -> Result<std::collections::HashMap<i64, FileConflicts>> {
        use std::collections::{HashMap, HashSet};

        let mut enabled: Vec<&ModRecord> = records.iter().filter(|m| m.enabled).collect();
        enabled.sort_by_key(|m| (m.priority, m.id));

        let mut claims: BTreeMap<sbmm_vfs::VPath, Vec<i64>> = BTreeMap::new();
        let mut totals: HashMap<i64, u32> = HashMap::new();
        for record in enabled {
            let components = self.store.components_for(record.id)?;
            let plan = self.plan_for(record, &components);
            let mut seen = HashSet::new();
            for file in &plan.files {
                let Some(path) = sbmm_vfs::VPath::from_relative(&file.target) else {
                    continue;
                };
                if seen.insert(path.clone()) {
                    claims.entry(path).or_default().push(record.id);
                }
            }
            totals.insert(record.id, seen.len() as u32);
        }

        let mut out: HashMap<i64, FileConflicts> = HashMap::new();
        for ids in claims.values().filter(|ids| ids.len() > 1) {
            let winner = ids.last().copied();
            for &id in ids {
                let entry = out.entry(id).or_default();
                if Some(id) == winner {
                    entry.wins += 1;
                } else {
                    entry.loses += 1;
                }
            }
        }
        for (id, counts) in out.iter_mut() {
            counts.redundant = counts.loses > 0 && Some(&counts.loses) == totals.get(id);
        }
        Ok(out)
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

/// One staging folder per upscaler component, so installing a newer version
/// replaces the previous swap instead of stacking on top of it.
fn upscaler_staging_id(component: sbmm_upscaler::Component) -> String {
    format!("upscaler-{}", component.file_name().replace(".dll", ""))
}

/// Whether a file has to be in the game folder for real even under the
/// virtual file system: anything in the binaries folder except UE4SS's mods,
/// which UE4SS reads through the hooked file API, and every DLL swap the
/// manager makes, which the graphics driver loads on its own terms.
fn must_be_physical(source: &str, target: &Path) -> bool {
    if matches!(source, "upscaler" | "framegen") {
        return true;
    }
    let target = comparable_path(&target.to_string_lossy());
    let inside = |dir: &str| target.starts_with(&format!("{}/", comparable_path(dir)));
    inside(sbmm_core::paths::BINARIES_WIN64) && !inside(sbmm_core::paths::UE4SS_MODS)
}

/// Display names for the providers in a virtual view.
struct OriginLabels(std::collections::HashMap<sbmm_vfs::Origin, String>);

impl OriginLabels {
    /// `(kind, name, mod id)` for one provider.
    fn label(&self, origin: sbmm_vfs::Origin) -> (String, String, Option<i64>) {
        use sbmm_vfs::Origin;
        let (kind, mod_id) = match origin {
            Origin::Mod(id) => ("mod", Some(id)),
            Origin::Generated => ("generated", None),
            Origin::Overwrite => ("overwrite", None),
        };
        let name = self
            .0
            .get(&origin)
            .cloned()
            .unwrap_or_else(|| match origin {
                Origin::Mod(id) => format!("mod {id}"),
                _ => kind.to_string(),
            });
        (kind.to_string(), name, mod_id)
    }
}

/// The staging folder of the RTX 20/30 frame generation package.
const FRAMEGEN_STAGING: &str = "framegen-dlssg-sm86";

/// A game-relative path in a form two spellings of it agree on: Windows paths
/// ignore case, and the deployment record may hold either separator.
fn comparable_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

/// Empty a folder, keeping the folder itself.
fn clear_dir(dir: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let removed = if entry.file_type().map(|k| k.is_dir()).unwrap_or(false) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        removed.map_err(|e| AppError::io(&path, e))?;
    }
    Ok(())
}

/// Rename where possible, copy across volumes.
fn move_or_copy(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| AppError::io(to, e))?;
    Ok(())
}

/// Paths cross to the UI as strings, so lossy conversion happens in one place.
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
