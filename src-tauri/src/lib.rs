//! Tauri command surface.
//!
//! Intentionally thin: every command unlocks the shared [`sbmm_app::App`],
//! forwards the call, and maps the error to a string for the frontend. All
//! behaviour worth testing lives in `sbmm-app`.

mod collections;
mod downloads;
mod upscaler;

use std::sync::Mutex;

use sbmm_app::dto::{
    AppSnapshot, ApplyReport, ConflictReport, FoldersView, LibraryMoveReport, NexusAccount,
    StagedInstall,
};
use sbmm_app::App;
use sbmm_core::model::ModType;
use sbmm_game::GameInstall;
use sbmm_nexus::nxm::{self, NxmLink};
use sbmm_nexus::queue::QueueItem;
use sbmm_nexus::{NexusClient, ReqwestTransport};
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_updater::UpdaterExt;

use downloads::Downloads;

struct AppState(Mutex<App>);

/// Errors reach the UI as plain strings; the frontend renders them in a toast.
pub(crate) fn fail<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

/// Run `body` against the locked service.
#[macro_export]
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

/// Which enabled mods are replacing the same assets, and which one the game
/// actually loads.
#[tauri::command]
fn conflicts(state: tauri::State<'_, AppState>) -> Result<ConflictReport, String> {
    with_app!(state, |app| app.conflicts())
}

/// Read the containers of any mod that has no complete asset index yet.
///
/// Covers mods installed before the feature existed and any whose scan failed
/// at install time. Returns how many were looked at.
#[tauri::command]
fn index_mod_assets(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    with_app!(state, |app| app.index_missing_assets())
}

const USER_AGENT: &str = concat!("SBModManager/", env!("CARGO_PKG_VERSION"));

// -- downloads --------------------------------------------------------------

/// Accept an `nxm://` link, whether it arrived from the browser or was pasted.
///
/// A link for another game is a normal thing to receive — the handler is
/// registered process-wide — so it comes back as an error the UI explains
/// rather than as a failure.
async fn accept_nxm(app: &tauri::AppHandle, url: &str) -> Result<(), String> {
    match nxm::parse(url, sbmm_game::NEXUS_DOMAIN).map_err(fail)? {
        NxmLink::File(link) => {
            let described = describe(app, link.mod_id, link.file_id).await;
            let mut item = QueueItem::new(
                link.mod_id,
                link.file_id,
                described.name,
                described.file_name,
            )
            .with_version(described.version);
            if let (Some(key), Some(expires)) = (link.key.clone(), link.expires) {
                item = item.with_credentials(key, expires);
            }

            app.state::<Downloads>().queue.push(item);
            let _ = app.emit("downloads-changed", ());
        }
        NxmLink::Collection(link) => {
            // Collections are handled by the collections screen; tell it a
            // link arrived rather than silently dropping it.
            let _ = app.emit("nxm-collection", link);
        }
    }
    Ok(())
}

/// What the API can tell us about a file before it is downloaded.
struct Described {
    name: String,
    file_name: String,
    version: Option<String>,
}

/// Ask Nexus what this file is called, falling back to the ids.
///
/// Names are cosmetic, so a failure here must not stop the download: without a
/// key, or with the API unreachable, the queue still works — the mod just
/// carries no version and so cannot be checked for updates later.
async fn describe(app: &tauri::AppHandle, mod_id: u64, file_id: u64) -> Described {
    let fallback = || Described {
        name: format!("Mod {mod_id}"),
        file_name: format!("{mod_id}-{file_id}.zip"),
        version: None,
    };

    let key = {
        let state = app.state::<AppState>();
        let Ok(guard) = state.0.lock() else {
            return fallback();
        };
        guard.nexus_api_key().ok().flatten().unwrap_or_default()
    };
    if key.is_empty() {
        return fallback();
    }

    let Ok(transport) = ReqwestTransport::new(USER_AGENT) else {
        return fallback();
    };
    let client = NexusClient::new(transport, key, sbmm_game::NEXUS_DOMAIN);

    let mut described = fallback();
    if let Ok(info) = client.mod_info(mod_id).await {
        if let Some(name) = info.name {
            described.name = name;
        }
        described.version = info.version;
    }
    if let Ok(files) = client.mod_files(mod_id).await {
        if let Some(file) = files.iter().find(|f| f.file_id == file_id) {
            described.file_name = safe_file_name(&file.file_name);
            // The file's own version is the more precise of the two: a mod page
            // can offer an older file alongside the current one.
            if file.version.is_some() {
                described.version = file.version.clone();
            }
        }
    }
    described
}

/// Keep only the final component, so a name from the API cannot write outside
/// the downloads folder.
fn safe_file_name(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "download.bin".to_string())
}

/// Hand the app a link the user pasted by hand.
#[tauri::command]
async fn add_nxm_link(app: tauri::AppHandle, url: String) -> Result<(), String> {
    accept_nxm(&app, url.trim()).await
}

// -- update checks ----------------------------------------------------------

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCheckReport {
    /// How many mod pages were actually queried this time.
    checked: usize,
    /// How many installed mods are showing an update, including ones found by
    /// an earlier check.
    outdated: usize,
}

/// Ask Nexus which installed mods have moved on.
///
/// `updated.json` is one request that names every mod in the game that changed
/// in the period, so it is used to narrow the list first; without it, checking
/// forty installed mods would cost forty requests every time.
#[tauri::command]
async fn check_mod_updates(state: tauri::State<'_, AppState>) -> Result<UpdateCheckReport, String> {
    let key = with_app!(state, |app| app.nexus_api_key())?.unwrap_or_default();
    if key.is_empty() {
        return Err("add your Nexus API key first".into());
    }
    let installed = with_app!(state, |app| app.nexus_mods())?;
    if installed.is_empty() {
        return Ok(UpdateCheckReport {
            checked: 0,
            outdated: 0,
        });
    }

    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let client = NexusClient::new(transport, key, sbmm_game::NEXUS_DOMAIN);

    // A month covers the gap between reasonable check intervals; anything
    // older than that was already caught by an earlier check.
    let recently_changed = client.updated("1m").await.map_err(fail)?;
    let changed: std::collections::HashSet<u64> =
        recently_changed.iter().map(|m| m.mod_id).collect();

    let mut checked = 0;
    for entry in installed {
        let nexus_id = entry.nexus_mod_id as u64;
        if !changed.contains(&nexus_id) {
            continue;
        }
        let Ok(info) = client.mod_info(nexus_id).await else {
            continue;
        };
        checked += 1;
        with_app!(state, |app| app
            .record_update_check(entry.id, info.version.as_deref()))?;
    }

    // Counted from the stored result rather than this pass, so a mod found
    // outdated last week still counts even though it was not re-queried.
    let outdated = with_app!(state, |app| app.nexus_mods())?
        .iter()
        .filter(|m| m.is_outdated())
        .count();

    Ok(UpdateCheckReport { checked, outdated })
}

#[tauri::command]
fn download_queue(downloads: tauri::State<'_, Downloads>) -> Vec<sbmm_nexus::QueueItem> {
    downloads.queue.items()
}

#[tauri::command]
fn cancel_download(downloads: tauri::State<'_, Downloads>, id: u64) {
    downloads.queue.cancel(id);
}

#[tauri::command]
fn clear_finished_downloads(downloads: tauri::State<'_, Downloads>) {
    downloads.queue.clear_finished();
}

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

/// Queue a link that arrived from outside the window, and say so if it cannot
/// be used — a silent no-op after clicking a download button looks broken.
fn deliver_nxm(app: &tauri::AppHandle, url: &str) {
    if !url.to_ascii_lowercase().starts_with("nxm://") {
        return;
    }
    let app = app.clone();
    let url = url.to_string();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = accept_nxm(&app, &url).await {
            let _ = app.emit("nxm-rejected", (url, error));
        }
    });
}

/// Bring the window forward, since the click that sent the link happened in
/// the browser.
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.webview_windows().values().next() {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must come first: on Windows the OS starts a fresh process for every
        // nxm:// link, and this is what forwards the link to the running
        // window instead of opening a second manager.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            focus_main_window(app);
            for arg in argv.iter().skip(1) {
                deliver_nxm(app, arg);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let service = App::new(data_dir)?;
            app.manage(AppState(Mutex::new(service)));
            app.manage(Downloads::new());

            // Only needed during development and on Linux; the installer
            // registers the scheme on Windows.
            #[cfg(any(debug_assertions, target_os = "linux"))]
            let _ = app.deep_link().register("nxm");

            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    deliver_nxm(&handle, url.as_str());
                }
            });

            // The link that started the app arrives before the handler above
            // is installed, so it is collected separately.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                let handle = app.handle().clone();
                for url in urls {
                    deliver_nxm(&handle, url.as_str());
                }
            }

            downloads::restore(app.handle());
            downloads::spawn_driver(app.handle().clone());
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
            conflicts,
            index_mod_assets,
            set_library_root,
            set_nexus_key,
            nexus_rate_limit,
            check_for_update,
            install_update,
            set_update_channel,
            add_nxm_link,
            check_mod_updates,
            download_queue,
            cancel_download,
            clear_finished_downloads,
            collections::resolve_collection,
            collections::install_collection,
            upscaler::upscaler_status,
            upscaler::update_upscaler,
            upscaler::restore_upscaler,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start SB Mod Manager");
}
