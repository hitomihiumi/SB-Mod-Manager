//! Tauri command surface.
//!
//! Intentionally thin: every command unlocks the shared [`sbmm_app::App`],
//! forwards the call, and maps the error to a string for the frontend. All
//! behaviour worth testing lives in `sbmm-app`.

use std::sync::Mutex;

use sbmm_app::dto::{AppSnapshot, ApplyReport, FoldersView, LibraryMoveReport, StagedInstall};
use sbmm_app::App;
use sbmm_core::model::ModType;
use sbmm_game::GameInstall;
use tauri::Manager;

struct AppState(Mutex<App>);

/// Errors reach the UI as plain strings; the frontend renders them in a toast.
fn fail<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

/// Run `body` against the locked service.
macro_rules! with_app {
    ($state:expr, |$app:ident| $body:expr) => {{
        let mut guard = $state
            .0
            .lock()
            .map_err(|_| "app state is poisoned".to_string())?;
        let $app = &mut *guard;
        $body.map_err(fail)
    }};
}

#[tauri::command]
fn snapshot(state: tauri::State<'_, AppState>) -> Result<AppSnapshot, String> {
    with_app!(state, |app| app.snapshot())
}

#[tauri::command]
fn discover_games(state: tauri::State<'_, AppState>) -> Result<Vec<GameInstall>, String> {
    let guard = state
        .0
        .lock()
        .map_err(|_| "app state is poisoned".to_string())?;
    Ok(guard.discover_games())
}

#[tauri::command]
fn set_game_root(state: tauri::State<'_, AppState>, path: String) -> Result<GameInstall, String> {
    with_app!(state, |app| app.set_game_root(path))
}

#[tauri::command]
fn stage_archive(state: tauri::State<'_, AppState>, path: String) -> Result<StagedInstall, String> {
    with_app!(state, |app| app.stage_archive(path))
}

#[tauri::command]
fn stage_folder(state: tauri::State<'_, AppState>, path: String) -> Result<StagedInstall, String> {
    with_app!(state, |app| app.stage_folder(path))
}

#[tauri::command]
fn confirm_install(
    state: tauri::State<'_, AppState>,
    staging_id: String,
    name: String,
    type_override: Option<String>,
) -> Result<i64, String> {
    let forced = match type_override {
        Some(id) => {
            Some(ModType::from_str_id(&id).ok_or_else(|| format!("unknown mod type {id}"))?)
        }
        None => None,
    };
    with_app!(state, |app| app.confirm_install(&staging_id, &name, forced))
}

#[tauri::command]
fn cancel_install(state: tauri::State<'_, AppState>, staging_id: String) -> Result<(), String> {
    with_app!(state, |app| app.cancel_install(&staging_id))
}

#[tauri::command]
fn set_enabled(
    state: tauri::State<'_, AppState>,
    mod_ids: Vec<i64>,
    enabled: bool,
) -> Result<(), String> {
    with_app!(state, |app| app.set_enabled(&mod_ids, enabled))
}

#[tauri::command]
fn set_order(state: tauri::State<'_, AppState>, mod_ids: Vec<i64>) -> Result<(), String> {
    with_app!(state, |app| app.set_order(&mod_ids))
}

#[tauri::command]
fn apply(state: tauri::State<'_, AppState>) -> Result<ApplyReport, String> {
    with_app!(state, |app| app.apply())
}

#[tauri::command]
fn uninstall(state: tauri::State<'_, AppState>, mod_id: i64) -> Result<(), String> {
    with_app!(state, |app| app.uninstall(mod_id))
}

#[tauri::command]
fn rename_mod(state: tauri::State<'_, AppState>, mod_id: i64, name: String) -> Result<(), String> {
    with_app!(state, |app| app.rename_mod(mod_id, &name))
}

#[tauri::command]
fn create_group(
    state: tauri::State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> Result<i64, String> {
    with_app!(state, |app| app.create_group(&name, color.as_deref()))
}

#[tauri::command]
fn delete_group(state: tauri::State<'_, AppState>, group_id: i64) -> Result<(), String> {
    with_app!(state, |app| app.delete_group(group_id))
}

#[tauri::command]
fn set_group_collapsed(
    state: tauri::State<'_, AppState>,
    group_id: i64,
    collapsed: bool,
) -> Result<(), String> {
    with_app!(state, |app| app.set_group_collapsed(group_id, collapsed))
}

#[tauri::command]
fn assign_group(
    state: tauri::State<'_, AppState>,
    mod_ids: Vec<i64>,
    group_id: Option<i64>,
) -> Result<(), String> {
    with_app!(state, |app| app.assign_group(&mod_ids, group_id))
}

#[tauri::command]
fn set_auto_apply(state: tauri::State<'_, AppState>, enabled: bool) -> Result<(), String> {
    with_app!(state, |app| app.set_auto_apply(enabled))
}

/// Where the library lives, and whether hard links to the game will work.
#[tauri::command]
fn folders(state: tauri::State<'_, AppState>) -> Result<FoldersView, String> {
    with_app!(state, |app| app.folders())
}

#[tauri::command]
fn set_library_root(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<LibraryMoveReport, String> {
    with_app!(state, |app| app.set_library_root(path))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let service = App::new(data_dir)?;
            app.manage(AppState(Mutex::new(service)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            discover_games,
            set_game_root,
            stage_archive,
            stage_folder,
            confirm_install,
            cancel_install,
            set_enabled,
            set_order,
            apply,
            uninstall,
            rename_mod,
            create_group,
            delete_group,
            set_group_collapsed,
            assign_group,
            set_auto_apply,
            folders,
            set_library_root,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start SB Mod Manager");
}
