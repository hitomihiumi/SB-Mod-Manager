//! Commands for mod lists and for updating mods from Nexus in one go.
//!
//! Both end in the download queue, which already handles Premium downloads
//! and walks a free account through the website. What happens when each file
//! arrives — taking its place in an imported profile, or replacing the
//! version it updates — was arranged by `sbmm-app` before it was queued.

use std::path::PathBuf;

use sbmm_app::dto::{ImportOutcome, ImportPlan, UpdatableMod};
use sbmm_app::modlist::pick_update;
use sbmm_nexus::queue::QueueItem;
use sbmm_nexus::{NexusClient, ReqwestTransport};
use serde::Serialize;

use crate::downloads::Downloads;
use crate::{fail, with_app, AppState, USER_AGENT};

fn client(state: &tauri::State<'_, AppState>) -> Result<NexusClient<ReqwestTransport>, String> {
    let key = with_app!(state, |app| app.nexus_api_key())?.unwrap_or_default();
    if key.is_empty() {
        return Err("add your Nexus API key first".into());
    }
    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    Ok(NexusClient::new(transport, key, sbmm_game::NEXUS_DOMAIN))
}

fn queue_file(
    downloads: &Downloads,
    mod_id: u64,
    file_id: u64,
    name: &str,
    version: Option<String>,
) {
    downloads.queue.push(
        QueueItem::new(mod_id, file_id, name, format!("{mod_id}-{file_id}.zip"))
            .with_version(version),
    );
}

/// Write the active profile to `path`; returns how many mods it lists.
#[tauri::command(async)]
pub fn export_mod_list(state: tauri::State<'_, AppState>, path: String) -> Result<usize, String> {
    with_app!(state, |app| app.export_mod_list(&PathBuf::from(&path)))
}

#[tauri::command(async)]
pub fn plan_import(state: tauri::State<'_, AppState>, path: String) -> Result<ImportPlan, String> {
    with_app!(state, |app| app.plan_import(&PathBuf::from(&path)))
}

/// Import as a new profile, and queue what has to come from Nexus. A list
/// entry without a file id gets the mod's newest main file.
#[tauri::command]
pub async fn import_mod_list(
    state: tauri::State<'_, AppState>,
    downloads: tauri::State<'_, Downloads>,
    path: String,
    profile: String,
) -> Result<ImportOutcome, String> {
    let mut outcome = with_app!(state, |app| app
        .import_mod_list(&PathBuf::from(&path), &profile))?;
    let mut unreachable = Vec::new();
    let client = if outcome.downloads.is_empty() {
        None
    } else {
        client(&state).ok()
    };
    for entry in &outcome.downloads {
        let Some(nexus) = entry.nexus_mod_id else {
            continue;
        };
        let file = match (entry.nexus_file_id, &client) {
            (Some(file), _) => Some((file as u64, entry.version.clone())),
            (None, Some(client)) => client
                .mod_files(nexus as u64)
                .await
                .ok()
                .and_then(|files| pick_update(&files, None))
                .map(|f| (f.file_id, f.version)),
            (None, None) => None,
        };
        match file {
            Some((file_id, version)) => {
                queue_file(&downloads, nexus as u64, file_id, &entry.name, version)
            }
            None => unreachable.push(entry.name.clone()),
        }
    }
    // What could not be queued has to be fetched by hand, like the rest.
    outcome.downloads.retain(|e| !unreachable.contains(&e.name));
    outcome.manual.extend(unreachable);
    Ok(outcome)
}

#[tauri::command(async)]
pub fn updatable_mods(state: tauri::State<'_, AppState>) -> Result<Vec<UpdatableMod>, String> {
    with_app!(state, |app| app.updatable_mods())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesQueued {
    queued: usize,
    /// Already on the newest file of their kind.
    current: Vec<String>,
    /// Name and reason.
    failed: Vec<(String, String)>,
}

/// Queue the newest file of each mod in `ids`; each replaces its old
/// version when it is installed.
#[tauri::command]
pub async fn update_mods(
    state: tauri::State<'_, AppState>,
    downloads: tauri::State<'_, Downloads>,
    ids: Vec<i64>,
) -> Result<UpdatesQueued, String> {
    let client = client(&state)?;
    let candidates: Vec<UpdatableMod> = with_app!(state, |app| app.updatable_mods())?
        .into_iter()
        .filter(|m| ids.contains(&m.id))
        .collect();

    let mut report = UpdatesQueued {
        queued: 0,
        current: Vec::new(),
        failed: Vec::new(),
    };
    for candidate in candidates {
        let files = match client.mod_files(candidate.nexus_mod_id as u64).await {
            Ok(files) => files,
            Err(error) => {
                report.failed.push((candidate.name, error.to_string()));
                continue;
            }
        };
        let installed = candidate.nexus_file_id.map(|id| id as u64);
        match pick_update(&files, installed) {
            Some(file) => {
                with_app!(state, |app| app.expect_update(candidate.id))?;
                queue_file(
                    &downloads,
                    candidate.nexus_mod_id as u64,
                    file.file_id,
                    &candidate.name,
                    file.version.or(Some(candidate.latest.clone())),
                );
                report.queued += 1;
            }
            None => report.current.push(candidate.name),
        }
    }
    Ok(report)
}
