//! Commands for what a mod changes, load order rules and Nexus mod pages.

use sbmm_app::dto::{ModContents, NexusView, OrderRuleView};
use sbmm_nexus::{Fetcher, NexusClient, ReqwestTransport};

use tauri::Manager;

use crate::{fail, with_app, AppState, USER_AGENT};

#[tauri::command]
pub fn mod_contents(state: tauri::State<'_, AppState>, mod_id: i64) -> Result<ModContents, String> {
    with_app!(state, |app| app.mod_contents(mod_id))
}

#[tauri::command]
pub fn order_rules(state: tauri::State<'_, AppState>) -> Result<Vec<OrderRuleView>, String> {
    with_app!(state, |app| app.order_rules())
}

/// `before` loads before `after`; the list is sorted to obey it at once.
#[tauri::command]
pub fn add_order_rule(
    state: tauri::State<'_, AppState>,
    before: i64,
    after: i64,
) -> Result<usize, String> {
    with_app!(state, |app| app
        .add_order_rule(before, after)
        .and_then(|_| app.sort_by_rules()))
}

#[tauri::command]
pub fn delete_order_rule(state: tauri::State<'_, AppState>, id: i64) -> Result<(), String> {
    with_app!(state, |app| app.delete_order_rule(id))
}

/// Returns how many mods moved.
#[tauri::command]
pub fn sort_by_rules(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    with_app!(state, |app| app.sort_by_rules())
}

#[tauri::command]
pub fn nexus_view(
    state: tauri::State<'_, AppState>,
    mod_id: i64,
) -> Result<Option<NexusView>, String> {
    with_app!(state, |app| app.nexus_view(mod_id))
}

/// Ask Nexus about a mod again if the last answer is old (or `force`), and
/// fetch the page picture into the mod's folder the first time.
#[tauri::command]
pub async fn refresh_nexus_details(
    app_handle: tauri::AppHandle,
    mod_id: i64,
    force: bool,
) -> Result<Option<NexusView>, String> {
    refresh(&app_handle, mod_id, force).await
}

/// The refresh itself, also run for a mod just installed from Nexus so its
/// requirements are checked before anyone opens its page.
pub async fn refresh(
    app_handle: &tauri::AppHandle,
    mod_id: i64,
    force: bool,
) -> Result<Option<NexusView>, String> {
    let state = app_handle.state::<AppState>();
    let (due, view) = with_app!(state, |app| app
        .nexus_details_due(mod_id)
        .and_then(|due| Ok((due, app.nexus_view(mod_id)?))))?;
    // Mod pages can be read without an account, so no key is no obstacle.
    let key = with_app!(state, |app| Ok::<_, sbmm_app::AppError>(
        app.nexus_api_key().ok().flatten().unwrap_or_default()
    ))?;
    let Some(view) = view else {
        return Ok(None);
    };
    if !force && !due {
        return Ok(Some(view));
    }

    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let client = NexusClient::new(transport.clone(), key, sbmm_game::NEXUS_DOMAIN);
    let Some(details) = client
        .mod_details(sbmm_game::NEXUS_GAME_ID, view.nexus_mod_id as u64)
        .await
        .map_err(fail)?
    else {
        return Err("Nexus has no page for this mod any more".into());
    };
    with_app!(state, |app| app.record_nexus_details(&details))?;

    if let Some(url) = &details.picture_url {
        let path = with_app!(state, |app| app.nexus_picture_path(mod_id, url))?;
        if let Some(path) = path.filter(|p| !p.exists()) {
            // A missing picture is not worth failing the refresh over.
            if transport.fetch(url, &path, &|_| {}).await.is_ok() {
                with_app!(state, |app| app.adopt_nexus_picture(mod_id, &path))?;
            }
        }
    }
    with_app!(state, |app| app.nexus_view(mod_id))
}
