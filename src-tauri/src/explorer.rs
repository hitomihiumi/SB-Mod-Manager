//! Commands for the explorer: the mod library as real folders, and opening
//! a file in another program.
//!
//! The work, and the checks that keep it inside the library, live in
//! `sbmm-app`; this module only carries the calls across.

use sbmm_app::dto::{LibraryChange, LibraryListing, LibraryPreview};
use tauri_plugin_opener::OpenerExt;

use crate::{fail, AppState};

/// One folder of the mod library; empty is the top, which lists the mods.
#[tauri::command(async)]
pub fn library_list(
    state: tauri::State<'_, AppState>,
    dir: Option<String>,
) -> Result<LibraryListing, String> {
    crate::with_app!(state, |app| app.library_list(dir.as_deref().unwrap_or("")))
}

/// Move files and folders of the library into another folder of it.
#[tauri::command(async)]
pub fn library_move(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
    to_dir: String,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_move(&paths, &to_dir))
}

/// Give a file or folder of the library another name.
#[tauri::command(async)]
pub fn library_rename(
    state: tauri::State<'_, AppState>,
    path: String,
    new_name: String,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_rename(&path, &new_name))
}

/// Copy files and folders of the library into a folder of it.
#[tauri::command(async)]
pub fn library_copy(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
    to_dir: String,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_copy(&paths, &to_dir))
}

/// Copy files dropped from the system's file manager into a library folder.
#[tauri::command(async)]
pub fn library_import(
    state: tauri::State<'_, AppState>,
    sources: Vec<String>,
    to_dir: String,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_import(&sources, &to_dir))
}

/// Delete into the library's trash; the result says how to undo it.
#[tauri::command(async)]
pub fn library_delete(
    state: tauri::State<'_, AppState>,
    paths: Vec<String>,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_delete(&paths))
}

/// Take back a delete.
#[tauri::command(async)]
pub fn library_restore(
    state: tauri::State<'_, AppState>,
    undo: String,
) -> Result<LibraryChange, String> {
    crate::with_app!(state, |app| app.library_restore(&undo))
}

/// A picture or the start of a text file, for the preview.
#[tauri::command(async)]
pub fn library_preview(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<LibraryPreview, String> {
    crate::with_app!(state, |app| app.library_preview(&path))
}

/// A new folder inside a mod's folder.
#[tauri::command(async)]
pub fn library_mkdir(
    state: tauri::State<'_, AppState>,
    dir: String,
    name: String,
) -> Result<(), String> {
    crate::with_app!(state, |app| app.library_mkdir(&dir, &name))
}

/// Open a file or folder with the program Windows has for it, or with the
/// program at `with`. Only things that exist are handed on.
#[tauri::command(async)]
pub fn open_path(
    app_handle: tauri::AppHandle,
    path: String,
    with: Option<String>,
) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err(format!("{path} is not there any more"));
    }
    app_handle.opener().open_path(path, with).map_err(fail)
}
