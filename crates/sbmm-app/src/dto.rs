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
