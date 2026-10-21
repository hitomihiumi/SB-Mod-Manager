//! Shapes handed to the UI. These mirror the TypeScript types in `src/lib/ipc.ts`.

use sbmm_core::model::DetectedComponent;
use sbmm_game::GameInstall;
use serde::{Deserialize, Serialize};

/// Everything the main window needs to render itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub game: Option<GameInstall>,
    pub mods: Vec<ModView>,
    pub groups: Vec<GroupView>,
    /// Mods whose enabled state differs from what is on disk.
    pub pending: Vec<PendingChange>,
    /// Files that were changed outside the manager.
    pub drift: Vec<String>,
    pub auto_apply: bool,
    /// `None` until an API key has been validated.
    pub nexus: Option<NexusAccount>,
    /// Which release stream the app updates itself from.
    pub update_channel: String,
    /// Whether the manager publishes a Discord presence.
    pub discord_rpc: bool,
    /// How enabled mods reach the game.
    pub deployment_mode: DeploymentMode,
    /// Every profile, the active one marked.
    pub profiles: Vec<sbmm_store::ProfileRecord>,
    /// What the game created under the virtual file system.
    pub overwrite: OverwriteView,
    /// Whether UE4SS will be loaded, and which mods need it.
    pub ue4ss: Ue4ssStatus,
    /// The game changed build since the user last acknowledged it.
    pub game_update: Option<GameUpdate>,
}

/// The game was updated under the mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameUpdate {
    pub previous: sbmm_game::GameBuild,
    pub current: sbmm_game::GameBuild,
    /// Deployed files the update replaced or removed.
    pub drifted: usize,
}

/// What putting drifted files back did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairReport {
    /// Removed files linked in again.
    pub relinked: usize,
    /// Files replaced by something else, now kept as the backup.
    pub displaced: usize,
    /// Settings files edited in the game folder, the edit taken into the mod.
    pub adopted: usize,
    /// Files changed through the link; only the record was updated.
    pub recorded: usize,
    pub failed: Vec<ApplyFailure>,
}

/// A restore point as the settings list shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePointView {
    pub id: i64,
    /// UTC, ISO 8601.
    pub created_at: String,
    pub label: String,
    pub automatic: bool,
    pub game_build: Option<String>,
    /// Mods enabled at the time.
    pub enabled: usize,
    /// Of those, how many have been uninstalled since and cannot come back.
    pub uninstalled: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReport {
    pub applied: ApplyReport,
    /// Mods the point had enabled that are no longer installed.
    pub uninstalled: usize,
}

/// The game's saves and their backups.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavesView {
    /// Where the saves are looked for; `None` off Windows with no choice made.
    pub dir: Option<String>,
    /// The folder was chosen by the user rather than the default.
    pub custom_dir: bool,
    pub exists: bool,
    /// Files in the saves folder now.
    pub files: usize,
    pub backup_on_launch: bool,
    /// Newest first.
    pub backups: Vec<SaveBackupView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveBackupView {
    /// The file name, which is also its id.
    pub file: String,
    /// Seconds since the Unix epoch.
    pub created_at: u64,
    /// `auto`, `manual` or `before-restore`.
    pub kind: String,
    pub bytes: u64,
}

/// UE4SS at a glance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ue4ssStatus {
    /// `missing`, `disabled` (installed as a mod but switched off), `managed`
    /// (installed as a mod and enabled) or `manual` (in the game folder,
    /// put there by hand).
    pub state: String,
    /// The UE4SS mod, when the manager installed it.
    pub mod_id: Option<i64>,
    pub version: Option<String>,
    /// Enabled mods that do nothing without UE4SS.
    pub needed_by: Vec<String>,
    /// Where the Stellar Blade build comes from.
    pub project_url: String,
}

/// The overwrite folder at a glance.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwriteView {
    pub path: String,
    pub files: u32,
    pub bytes: i64,
}

/// How enabled mods reach the game.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeploymentMode {
    /// Hard-linked into the game folder, which then really contains them.
    #[default]
    Hardlink,
    /// Kept out of the game folder and shown to the game through the virtual
    /// file system when it is started from the manager, as Mod Organizer 2
    /// does. Files the Windows loader reaches before the hook is in place are
    /// still linked in.
    Virtual,
}

impl DeploymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            DeploymentMode::Hardlink => "hardlink",
            DeploymentMode::Virtual => "virtual",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModView {
    pub id: i64,
    pub name: String,
    pub version: Option<String>,
    pub mod_type: String,
    /// Every distinct payload type in the mod, for the badge row.
    pub component_types: Vec<String>,
    pub group_id: Option<i64>,
    pub enabled: bool,
    /// True when the mod's files are currently in the game folder.
    pub deployed: bool,
    pub priority: i64,
    pub size_bytes: i64,
    pub source: String,
    pub installed_at: String,
    pub warnings: Vec<String>,
    /// Set only when Nexus reports a version different from the installed one.
    pub latest_version: Option<String>,
    pub nexus_mod_id: Option<i64>,
    /// Loose files this mod shares with other enabled mods. `None` when it
    /// shares none, or is disabled.
    pub file_conflicts: Option<FileConflicts>,
    /// What this enabled mod needs and the game will not have, e.g. `UE4SS`.
    pub missing_requirements: Vec<String>,
    /// Its folder in the library, which pictures are read from.
    pub folder: String,
    /// The mod's picture, relative to `folder`.
    pub image: Option<String>,
    pub notes: Option<String>,
    pub tags: Vec<String>,
    /// What the load order rules this mod breaks ask of it.
    pub misordered: Vec<String>,
}

/// What one mod changes, for the details panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModContents {
    /// Paths it puts in the game folder (the first 500).
    pub files: Vec<String>,
    pub file_count: usize,
    /// Its paks have been read; without that `areas` is empty, not "none".
    pub indexed: bool,
    /// Distinct assets its paks replace.
    pub asset_count: usize,
    /// Where those assets are, largest first.
    pub areas: Vec<AssetArea>,
    /// Other mods changing the same assets or files.
    pub clashes: Vec<ModClash>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetArea {
    /// Two folders under `Content`, e.g. `Characters/Eve`.
    pub area: String,
    pub count: usize,
    /// A few asset names from it.
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModClash {
    pub mod_id: i64,
    pub name: String,
    /// Assets and files both change.
    pub shared: usize,
    pub enabled: bool,
    /// This mod is later in the load order, so its versions are the ones used.
    pub wins: bool,
}

/// A load order rule, by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderRuleView {
    pub id: i64,
    pub before_id: i64,
    pub before: String,
    pub after_id: i64,
    pub after: String,
    /// The current order breaks it.
    pub broken: bool,
}

/// What Nexus says about an installed mod.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusView {
    pub nexus_mod_id: i64,
    /// `None` until fetched.
    pub details: Option<sbmm_nexus::ModDetails>,
    pub fetched_at: Option<String>,
    pub requirements: Vec<RequirementView>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementView {
    #[serde(flatten)]
    pub requirement: sbmm_nexus::Requirement,
    /// `enabled`, `disabled`, `missing`, or `external` when it cannot be checked.
    pub state: String,
    /// The installed mod that satisfies it, if any.
    pub installed_as: Option<i64>,
}

/// One settings file in the editor's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFileView {
    /// Relative to the mod's folder or the game's settings folder.
    pub path: String,
    pub bytes: u64,
    /// Differs from the copy it was installed (or first found) as.
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigText {
    pub text: String,
    /// The file as installed or first found, to reset to; `None` when no
    /// copy was kept (mods installed before copies were).
    pub original: Option<String>,
}

/// How a mod's loose files fare against the other enabled mods — the flags
/// Mod Organizer 2 shows beside each mod.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileConflicts {
    /// Paths it provides over a mod earlier in the load order.
    pub wins: u32,
    /// Paths a mod later in the load order takes from it.
    pub loses: u32,
    /// Every file it has is taken by later mods, so it does nothing at all.
    pub redundant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub collapsed: bool,
    pub sort_index: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Enable,
    Disable,
    /// Already enabled, but the load order changed so files must be renamed.
    Reorder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub mod_id: i64,
    pub name: String,
    pub kind: ChangeKind,
}

/// An asset that more than one enabled mod replaces.
///
/// Only one of them reaches the game, so the point of reporting these is to
/// say which — and to let the load order be changed on evidence rather than
/// on guesswork about what a mod touches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// The asset path, or a chunk id when the container had no directory
    /// index and the name is genuinely not recorded anywhere.
    pub asset: String,
    pub named: bool,
    /// Every enabled mod replacing it, in load order.
    pub claimants: Vec<Claimant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claimant {
    pub mod_id: i64,
    pub name: String,
    pub priority: i64,
    /// True for the one the game actually loads.
    pub wins: bool,
}

/// The conflict picture as a whole.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictReport {
    pub conflicts: Vec<Conflict>,
    /// How many assets each mod loses to a later one, keyed by mod id. What
    /// the mod list badges.
    pub overridden: Vec<ModConflictCount>,
    /// Mods whose containers could not be read, so their part of the picture
    /// is missing rather than empty.
    pub unreadable: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConflictCount {
    pub mod_id: i64,
    /// Assets this mod provides that a later mod replaces.
    pub losing: i64,
    /// Assets this mod takes from an earlier one.
    pub winning: i64,
}

/// An archive that has been extracted and inspected but not yet committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedInstall {
    /// Staging folder name, used to confirm or cancel the install.
    pub staging_id: String,
    /// Name guessed from the archive file name.
    pub suggested_name: String,
    pub components: Vec<DetectedComponent>,
    pub size_bytes: i64,
    /// True when the user has to be asked: a component whose type could not
    /// be worked out, or an archive offering several versions.
    pub needs_confirmation: bool,
    /// The versions the archive offers, when it is several mods in one.
    pub variants: Option<sbmm_core::VariantChoice>,
    /// The options chosen when this mod was installed before, to preselect.
    pub remembered_variants: Vec<String>,
    /// The archive's FOMOD installer, when it has one.
    pub fomod: Option<FomodView>,
    /// Pictures in the archive, relative to the staging folder, previews
    /// first — shown while the user decides.
    pub previews: Vec<String>,
}

/// A FOMOD installer ready to run in the install dialog.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FomodView {
    /// The folder holding `fomod/`, relative to the staging folder, which
    /// every path in the installer — pictures included — is relative to.
    pub root: String,
    pub installer: sbmm_core::fomod::Fomod,
    /// What to tick when the wizard opens, as `[step, group, option]`.
    pub selection: Vec<[usize; 3]>,
    /// True when `selection` is the user's choice from last time.
    pub remembered: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub deployed: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<ApplyFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyFailure {
    pub name: String,
    pub reason: String,
}

/// Where everything lives, for the settings screen.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldersView {
    /// The relocatable library root.
    pub library: String,
    pub mods: String,
    pub backups: String,
    pub downloads: String,
    /// What the game created while running under the virtual file system.
    pub overwrite: String,
    pub game: Option<String>,
    /// False when the library is on a different drive from the game, which
    /// means deployment falls back to copying and every enabled mod is stored
    /// twice.
    pub same_volume_as_game: bool,
    pub library_bytes: i64,
}

/// Who the stored Nexus key belongs to.
///
/// Premium status decides whether a collection can install unattended, so it
/// is recorded rather than rediscovered from a failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusAccount {
    pub name: String,
    pub is_premium: bool,
    pub user_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMoveReport {
    pub folders: FoldersView,
    /// The result of putting previously enabled mods back after the move.
    pub redeployed: ApplyReport,
}

/// The RTX 20/30 frame generation package, as installed by the manager.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramegenInstall {
    pub mod_id: i64,
    pub version: String,
    /// The name the proxy DLL went in under.
    pub proxy: sbmm_upscaler::framegen::Proxy,
    /// False when the user switched it off in the mod list.
    pub enabled: bool,
}

/// One entry in the virtual file tree, in the manner of Mod Organizer 2's
/// "Data" tab: the game folder as the enabled mods make it, with who provides
/// each file and who it hides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VfsEntry {
    pub name: String,
    pub is_dir: bool,
    /// Game-relative path of this entry, for navigating into a folder.
    pub path: String,
    /// What provides the winning version: a mod's name, "Game", "Overwrite"
    /// or the generated UE4SS list.
    pub provided_by: String,
    /// `mod`, `game`, `overwrite` or `generated`.
    pub origin: String,
    /// The mod's id, when a mod provides it, so the UI can jump to it.
    pub mod_id: Option<i64>,
    /// Other providers of the same path, hidden beneath the winner, the next
    /// in line first. A non-empty list is a conflict.
    pub shadowed_by: Vec<String>,
    /// The real file on disk the winning version comes from, for opening it.
    pub source: Option<String>,
}

/// A directory of the virtual file tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VfsListing {
    /// The game-relative directory shown. Empty is the game root.
    pub dir: String,
    pub entries: Vec<VfsEntry>,
}

/// Where the library's disk space goes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageReport {
    pub total: u64,
    pub parts: Vec<StoragePart>,
    /// Mods not enabled in any profile, largest first.
    pub unused_mods: Vec<UnusedMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoragePart {
    /// Which cleanup applies: `mods` (orphans), `downloads`, `saves`, `vfs`
    /// (logs); others cannot be cleaned.
    pub key: String,
    pub label: String,
    pub path: String,
    pub bytes: u64,
    pub files: usize,
    /// How much of it cleaning would free.
    pub cleanable_bytes: u64,
    pub cleanable_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnusedMod {
    pub id: i64,
    pub name: String,
    pub bytes: u64,
}

/// A mod archive that appeared in the watched downloads folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundArchive {
    /// Name and size, which is how it is remembered as offered.
    pub key: String,
    pub path: String,
    pub name: String,
    pub bytes: u64,
}

/// What importing a mod list would do.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPlan {
    /// The profile it was exported from, to suggest as the new one's name.
    pub profile: String,
    /// In load order.
    pub entries: Vec<ImportEntry>,
    /// Load order rules it carries.
    pub rules: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEntry {
    pub name: String,
    pub version: Option<String>,
    pub enabled: bool,
    /// `installed`, `download` (from Nexus) or `manual`.
    pub status: String,
    /// The installed mod it matched.
    pub mod_id: Option<i64>,
    pub nexus_mod_id: Option<i64>,
    pub nexus_file_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub profile_id: i64,
    pub profile: String,
    /// Installed mods put in place.
    pub matched: usize,
    /// Nexus mods to download; each takes its place when it arrives.
    pub downloads: Vec<ImportEntry>,
    /// Mods from elsewhere, to fetch by hand.
    pub manual: Vec<String>,
    /// What switching to the new profile changed in the game folder.
    pub applied: ApplyReport,
}

/// A Nexus mod with a newer version out.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatableMod {
    pub id: i64,
    pub name: String,
    pub nexus_mod_id: i64,
    pub nexus_file_id: Option<i64>,
    pub version: Option<String>,
    pub latest: String,
}
