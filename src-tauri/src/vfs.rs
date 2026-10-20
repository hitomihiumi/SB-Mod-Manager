//! Commands for the virtual file system and for starting the game.
//!
//! Which files go where, and the launch itself, live in `sbmm-app` and
//! `sbmm-vfs`; this module supplies the hook DLL the build embedded and runs
//! the launch off the UI thread.

use std::path::{Path, PathBuf};

use sbmm_app::dto::{ApplyReport, DeploymentMode, VfsListing};
use sbmm_game::InstallSource;
use tauri_plugin_opener::OpenerExt;

use crate::{fail, AppState};

/// Built by `build.rs` from `crates/sbmm-vfs-hook`. Empty off Windows.
const HOOK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sbmm_vfs_hook.dll"));

/// One directory of the virtual file tree, for the Files view.
#[tauri::command]
pub fn vfs_tree(state: tauri::State<'_, AppState>, dir: Option<String>) -> Result<VfsListing, String> {
    crate::with_app!(state, |app| app.vfs_tree(dir.as_deref().unwrap_or("")))
}

#[tauri::command]
pub fn set_deployment_mode(
    state: tauri::State<'_, AppState>,
    mode: DeploymentMode,
) -> Result<ApplyReport, String> {
    crate::with_app!(state, |app| app.set_deployment_mode(mode))
}

/// Start the game the way the current mode needs: through the virtual file
/// system, or the ordinary way with the mods already in the folder.
#[tauri::command]
pub async fn launch_game(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let (mode, game) = crate::with_app!(state, |app| app
        .deployment_mode()
        .and_then(|mode| Ok((mode, app.game()?))))?;
    let game = game.ok_or("choose the game folder first")?;

    if mode == DeploymentMode::Hardlink {
        // Steam's own launch keeps the overlay, cloud saves and playtime.
        if game.source == InstallSource::Steam {
            let url = format!("steam://rungameid/{}", sbmm_game::STEAM_APP_ID);
            app_handle.opener().open_url(url, None::<&str>).map_err(fail)?;
        } else {
            let exe = game.executable_path().ok_or("the game executable was not found")?;
            std::process::Command::new(&exe)
                .current_dir(game.binaries_dir())
                .spawn()
                .map_err(fail)?;
        }
        return Ok("Starting Stellar Blade.".into());
    }

    if HOOK.is_empty() {
        return Err("this build does not include the virtual file system".into());
    }
    let vfs_dir = crate::with_app!(state, |app| Ok::<_, sbmm_app::AppError>(app.vfs_dir()))?;
    let hook = install_hook(&vfs_dir)?;
    let options = crate::with_app!(state, |app| app.launch_options(hook))?;
    let log = vfs_dir.join("launch.log");

    let pid = tauri::async_runtime::spawn_blocking(move || sbmm_vfs::launch::launch(&options))
        .await
        .map_err(fail)?
        .map_err(|error| format!("{error}\nLaunch log: {}", log.display()))?;
    Ok(format!(
        "Started Stellar Blade under the virtual file system (process {pid})."
    ))
}

/// Put the embedded hook where the launcher can load it from.
///
/// Only rewritten when it differs: a game still running from the last launch
/// holds the file open, and the same bytes need no write.
fn install_hook(vfs_dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(vfs_dir).map_err(fail)?;
    let path = vfs_dir.join("sbmm_vfs_hook.dll");
    if std::fs::read(&path).ok().as_deref() != Some(HOOK) {
        std::fs::write(&path, HOOK).map_err(|e| {
            format!("could not update the virtual file system hook (is the game still running?): {e}")
        })?;
    }
    Ok(path)
}
