//! Commands for disk space, and the loop that watches a downloads folder.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use sbmm_app::dto::{FoundArchive, StorageReport};
use sbmm_app::storage::{settled, Cleanup};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::{fail, with_app, AppState};

/// How often the watched folder is looked at.
const POLL: Duration = Duration::from_secs(3);

#[tauri::command]
pub async fn storage_report(app_handle: tauri::AppHandle) -> Result<StorageReport, String> {
    // Every file in the library is visited, so off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        with_app!(state, |app| app.storage_report())
    })
    .await
    .map_err(fail)?
}

/// Returns the bytes freed.
#[tauri::command]
pub fn clean_storage(state: tauri::State<'_, AppState>, what: Cleanup) -> Result<u64, String> {
    with_app!(state, |app| app.clean(what))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchSettings {
    dir: Option<String>,
    auto_install: bool,
}

#[tauri::command]
pub fn watch_settings(state: tauri::State<'_, AppState>) -> Result<WatchSettings, String> {
    with_app!(state, |app| app.watch_dir().and_then(|dir| {
        Ok(WatchSettings {
            dir: dir.map(|d| d.to_string_lossy().into_owned()),
            auto_install: app.watch_auto_install()?,
        })
    }))
}

#[tauri::command]
pub fn set_watch_dir(state: tauri::State<'_, AppState>, dir: Option<String>) -> Result<(), String> {
    let dir = dir.map(PathBuf::from);
    with_app!(state, |app| app.set_watch_dir(dir.as_deref()))
}

#[tauri::command]
pub fn set_watch_auto_install(state: tauri::State<'_, AppState>, on: bool) -> Result<(), String> {
    with_app!(state, |app| app.set_watch_auto_install(on))
}

/// Archives the user installed or turned down, not to be offered again.
#[tauri::command]
pub fn mark_seen(state: tauri::State<'_, AppState>, keys: Vec<String>) -> Result<(), String> {
    with_app!(state, |app| app.mark_seen(&keys))
}

/// Look at the watched folder every few seconds and tell the window about
/// each finished archive once per session; the window installs it or asks.
pub fn spawn_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
        let mut told: BTreeSet<String> = BTreeSet::new();
        loop {
            std::thread::sleep(POLL);
            let found: Vec<FoundArchive> = {
                let state = app.state::<AppState>();
                let Ok(service) = state.0.lock() else {
                    return;
                };
                service.new_watched_archives().unwrap_or_default()
            };
            let ready: Vec<FoundArchive> = settled(&sizes, &found)
                .into_iter()
                .filter(|a| told.insert(a.key.clone()))
                .collect();
            sizes = found.into_iter().map(|a| (a.path, a.bytes)).collect();
            if !ready.is_empty() {
                let _ = app.emit("watch-found", ready);
            }
        }
    });
}
