//! Commands for ReShade presets.

use sbmm_app::dto::ReshadeView;

use crate::{fail, with_app, AppState};

#[tauri::command(async)]
pub fn reshade_view(state: tauri::State<'_, AppState>) -> Result<ReshadeView, String> {
    with_app!(state, |app| app.reshade_view())
}

#[tauri::command(async)]
pub fn set_reshade_preset(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    with_app!(state, |app| app.set_reshade_preset(&path))
}

#[tauri::command(async)]
pub fn set_profile_reshade_preset(
    state: tauri::State<'_, AppState>,
    profile_id: i64,
    path: Option<String>,
) -> Result<(), String> {
    with_app!(state, |app| app
        .set_profile_reshade_preset(profile_id, path.as_deref()))
}

#[tauri::command(async)]
pub fn duplicate_reshade_preset(
    state: tauri::State<'_, AppState>,
    path: String,
    name: String,
) -> Result<String, String> {
    with_app!(state, |app| app.duplicate_reshade_preset(&path, &name))
}

#[tauri::command(async)]
pub fn rename_reshade_preset(
    state: tauri::State<'_, AppState>,
    path: String,
    name: String,
) -> Result<String, String> {
    with_app!(state, |app| app.rename_reshade_preset(&path, &name))
}

#[tauri::command(async)]
pub fn delete_reshade_preset(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    with_app!(state, |app| app.delete_reshade_preset(&path))
}
