//! Commands for settings files and a mod's notes, tags and picture.

use sbmm_app::config::ConfigScope;
use sbmm_app::dto::{ConfigFileView, ConfigText};

use crate::{fail, with_app, AppState};

#[tauri::command]
pub fn config_files(
    state: tauri::State<'_, AppState>,
    scope: ConfigScope,
) -> Result<Vec<ConfigFileView>, String> {
    with_app!(state, |app| app.config_files(scope))
}

#[tauri::command]
pub fn read_config(
    state: tauri::State<'_, AppState>,
    scope: ConfigScope,
    path: String,
) -> Result<ConfigText, String> {
    with_app!(state, |app| app.read_config(scope, &path))
}

#[tauri::command]
pub fn write_config(
    state: tauri::State<'_, AppState>,
    scope: ConfigScope,
    path: String,
    text: String,
) -> Result<(), String> {
    with_app!(state, |app| app.write_config(scope, &path, &text))
}

#[tauri::command]
pub fn set_mod_notes(
    state: tauri::State<'_, AppState>,
    mod_id: i64,
    notes: String,
) -> Result<(), String> {
    with_app!(state, |app| app.set_mod_notes(mod_id, &notes))
}

#[tauri::command]
pub fn set_mod_tags(
    state: tauri::State<'_, AppState>,
    mod_id: i64,
    tags: Vec<String>,
) -> Result<(), String> {
    with_app!(state, |app| app.set_mod_tags(mod_id, &tags))
}

#[tauri::command]
pub fn mod_images(state: tauri::State<'_, AppState>, mod_id: i64) -> Result<Vec<String>, String> {
    with_app!(state, |app| app.mod_images(mod_id))
}

#[tauri::command]
pub fn set_mod_image(
    state: tauri::State<'_, AppState>,
    mod_id: i64,
    image: Option<String>,
) -> Result<(), String> {
    with_app!(state, |app| app.set_mod_image(mod_id, image.as_deref()))
}
