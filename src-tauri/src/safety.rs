//! Commands for getting back from things going wrong: game updates, files
//! changed outside the manager, restore points and save backups.

use std::path::PathBuf;

use sbmm_app::dto::{RepairReport, RestorePointView, RestoreReport, SaveBackupView, SavesView};
use sbmm_app::saves::BackupKind;

use crate::{fail, with_app, AppState};

#[tauri::command]
pub fn acknowledge_game_update(state: tauri::State<'_, AppState>) -> Result<(), String> {
    with_app!(state, |app| app.acknowledge_game_update())
}

#[tauri::command]
pub fn repair_drift(state: tauri::State<'_, AppState>) -> Result<RepairReport, String> {
    with_app!(state, |app| app.repair_drift())
}

#[tauri::command]
pub fn restore_points(state: tauri::State<'_, AppState>) -> Result<Vec<RestorePointView>, String> {
    with_app!(state, |app| app.restore_points())
}

#[tauri::command]
pub fn create_restore_point(
    state: tauri::State<'_, AppState>,
    label: String,
) -> Result<i64, String> {
    with_app!(state, |app| app.create_restore_point(&label))
}

#[tauri::command]
pub fn delete_restore_point(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    with_app!(state, |app| app.delete_restore_point(id))
}

#[tauri::command]
pub fn restore_to_point(
    state: tauri::State<'_, AppState>,
    id: i64,
) -> Result<RestoreReport, String> {
    with_app!(state, |app| app.restore_to_point(id))
}

#[tauri::command]
pub fn saves(state: tauri::State<'_, AppState>) -> Result<SavesView, String> {
    with_app!(state, |app| app.saves())
}

/// A backup made on request; always written, even if nothing changed.
#[tauri::command]
pub fn back_up_saves(state: tauri::State<'_, AppState>) -> Result<Option<SaveBackupView>, String> {
    with_app!(state, |app| app.back_up_saves(BackupKind::Manual))
}

#[tauri::command]
pub fn restore_saves(state: tauri::State<'_, AppState>, file: String) -> Result<(), String> {
    with_app!(state, |app| app.restore_saves(&file))
}

#[tauri::command]
pub fn delete_save_backup(state: tauri::State<'_, AppState>, file: String) -> Result<(), String> {
    with_app!(state, |app| app.delete_save_backup(&file))
}

/// `None` goes back to where Stellar Blade keeps its saves.
#[tauri::command]
pub fn set_saves_dir(state: tauri::State<'_, AppState>, dir: Option<String>) -> Result<(), String> {
    let dir = dir.map(PathBuf::from);
    with_app!(state, |app| app.set_saves_dir(dir.as_deref()))
}

#[tauri::command]
pub fn set_backup_saves_on_launch(
    state: tauri::State<'_, AppState>,
    on: bool,
) -> Result<(), String> {
    with_app!(state, |app| app.set_backup_saves_on_launch(on))
}
