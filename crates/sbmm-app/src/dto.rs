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
    /// True when at least one component needs the user to pick a type.
    pub needs_confirmation: bool,
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
