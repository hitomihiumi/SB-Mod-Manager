//! Tauri command surface.
//!
//! Intentionally thin: every command unlocks the shared [`sbmm_app::App`],
//! forwards the call, and maps the error to a string for the frontend. All
//! behaviour worth testing lives in `sbmm-app`.

use std::sync::Mutex;

use sbmm_app::dto::{
    AppSnapshot, ApplyReport, FoldersView, LibraryMoveReport, NexusAccount, StagedInstall,
};
use sbmm_app::App;
use sbmm_core::model::ModType;
use sbmm_game::GameInstall;
use sbmm_nexus::{NexusClient, ReqwestTransport};
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;

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

/// Check an API key with Nexus and remember whose it is.
///
/// The call is made here rather than inside `App` because the service is
/// synchronous by design; only the short store-the-result step takes the lock.
#[tauri::command]
async fn set_nexus_key(
    state: tauri::State<'_, AppState>,
    api_key: String,
) -> Result<Option<NexusAccount>, String> {
    let trimmed = api_key.trim().to_string();

    if trimmed.is_empty() {
        return with_app!(state, |app| app.set_nexus_account("", None)).map(|()| None);
    }

    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let client = NexusClient::new(transport, trimmed.clone(), sbmm_game::NEXUS_DOMAIN);
    let account = client.validate().await.map_err(fail)?;

    let account = NexusAccount {
        name: account.name,
        is_premium: account.is_premium,
        user_id: account.user_id,
    };
    with_app!(state, |app| app.set_nexus_account(&trimmed, Some(&account)))?;
    Ok(Some(account))
}

/// The remaining API allowance, for the downloads screen.
#[tauri::command]
async fn nexus_rate_limit(
    state: tauri::State<'_, AppState>,
) -> Result<sbmm_nexus::RateLimit, String> {
    let key = with_app!(state, |app| app.nexus_api_key())?.unwrap_or_default();
    if key.is_empty() {
        return Ok(sbmm_nexus::RateLimit::default());
    }
    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let client = NexusClient::new(transport, key, sbmm_game::NEXUS_DOMAIN);
    client.validate().await.map_err(fail)?;
    Ok(client.rate_limit())
}

const USER_AGENT: &str = concat!("SBModManager/", env!("CARGO_PKG_VERSION"));

// -- self update ------------------------------------------------------------

/// Where the updater looks for a manifest, per channel.
///
/// Both point at GitHub releases: the stable channel follows whatever release
/// is marked latest, the nightly channel follows the rolling `nightly` tag the
/// release workflow keeps moving.
fn updater_endpoint(channel: &str) -> &'static str {
    match channel {
        "nightly" => {
            "https://github.com/hitomihiumi/SB-Mod-Manager/releases/download/nightly/latest.json"
        }
        _ => "https://github.com/hitomihiumi/SB-Mod-Manager/releases/latest/download/latest.json",
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    current_version: String,
    available: Option<AvailableUpdate>,
    channel: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AvailableUpdate {
    version: String,
    notes: Option<String>,
    date: Option<String>,
}

#[tauri::command]
async fn check_for_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<UpdateInfo, String> {
    let channel = with_app!(state, |app| app.update_channel())?;
    let current_version = app.package_info().version.to_string();

    let endpoint = updater_endpoint(&channel)
        .parse()
        .map_err(|e| format!("bad update endpoint: {e}"))?;

    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(fail)?
        .build()
        .map_err(fail)?;

    // "Nothing new" is the ordinary answer, not a failure.
    let available = updater
        .check()
        .await
        .map_err(fail)?
        .map(|update| AvailableUpdate {
            version: update.version.clone(),
            notes: update.body.clone(),
            date: update.date.map(|d| d.to_string()),
        });

    Ok(UpdateInfo {
        current_version,
        available,
        channel,
    })
}

/// Download and install the pending update, then restart into it.
#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let channel = with_app!(state, |app| app.update_channel())?;
    let endpoint = updater_endpoint(&channel)
        .parse()
        .map_err(|e| format!("bad update endpoint: {e}"))?;

    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(fail)?
        .build()
        .map_err(fail)?;

    let Some(update) = updater.check().await.map_err(fail)? else {
        return Err("there is no update to install".into());
    };

    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(fail)?;

    app.restart();
}

#[tauri::command]
fn set_update_channel(state: tauri::State<'_, AppState>, channel: String) -> Result<(), String> {
    with_app!(state, |app| app.set_update_channel(&channel))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
            set_nexus_key,
            nexus_rate_limit,
            check_for_update,
            install_update,
            set_update_channel,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start SB Mod Manager");
}
