//! The command line and profile shortcuts.
//!
//! `--profile <name>` makes a profile active and `--launch` starts the game,
//! so a desktop shortcut can start Stellar Blade with a chosen set of mods the
//! way Mod Organizer 2's shortcuts do. A manager started only to launch stays
//! hidden and closes once the game is on its way; one already running takes
//! the request over from the second start, which the single-instance plugin
//! ends.

use std::path::PathBuf;
use std::sync::Mutex;

use sbmm_shell::{Request, Shortcut};
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::{fail, with_app, AppState};

/// Messages for the window about requests it did not make, kept until it
/// asks for them — a hidden window may not have loaded yet.
#[derive(Default)]
pub struct Notices(Mutex<Vec<Notice>>);

#[derive(Debug, Clone, Serialize)]
pub struct Notice {
    pub kind: &'static str,
    pub message: String,
}

fn notify(app: &tauri::AppHandle, kind: &'static str, message: String) {
    if let Ok(mut notices) = app.state::<Notices>().0.lock() {
        notices.push(Notice { kind, message });
    }
    let _ = app.emit("notices", ());
}

/// Act on a command line, and return what it asked for. `fresh` when this
/// process was started for it rather than handed it by a second start.
pub fn handle(app: &tauri::AppHandle, args: &[String], fresh: bool) -> Option<Request> {
    let request = match sbmm_shell::parse(args) {
        Ok(Some(request)) => request,
        Ok(None) => return None,
        Err(error) => {
            notify(app, "error", error);
            return None;
        }
    };

    let app = app.clone();
    let asked = request.clone();
    tauri::async_runtime::spawn(async move {
        match run(&app, &asked).await {
            // Started only to launch: the game is on its way and the manager
            // has nothing more to do.
            Ok(_) if asked.launch && fresh => app.exit(0),
            Ok(message) => notify(&app, "success", message),
            Err(error) => {
                notify(&app, "error", error);
                crate::focus_main_window(&app);
            }
        }
    });
    Some(request)
}

async fn run(app: &tauri::AppHandle, request: &Request) -> Result<String, String> {
    let mut said = Vec::new();
    if let Some(name) = &request.profile {
        let state = app.state::<AppState>();
        let (profile, report) = with_app!(state, |service| service
            .profile_named(name)
            .and_then(|profile| Ok((profile.name, service.switch_profile(profile.id)?))))?;
        if !report.failed.is_empty() {
            let failed = report
                .failed
                .iter()
                .map(|f| format!("{} — {}", f.name, f.reason))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(format!(
                "Profile “{profile}” is active, but {} mod(s) could not be deployed, so the game was not started: {failed}",
                report.failed.len()
            ));
        }
        said.push(format!("Profile “{profile}” is active."));
    }
    if request.launch {
        said.push(crate::vfs::launch(app).await?);
    }
    Ok(said.join(" "))
}

/// Hand the window what came in while it was not looking.
#[tauri::command]
pub fn take_notices(notices: tauri::State<'_, Notices>) -> Vec<Notice> {
    notices
        .0
        .lock()
        .map(|mut notices| std::mem::take(&mut *notices))
        .unwrap_or_default()
}

/// What a profile's shortcut runs, ready to paste into another launcher.
#[tauri::command]
pub fn profile_command(
    state: tauri::State<'_, AppState>,
    profile_id: i64,
    launch: bool,
) -> Result<String, String> {
    let name = profile_name(&state, profile_id)?;
    let exe = std::env::current_exe().map_err(fail)?;
    Ok(sbmm_shell::command_line(
        &exe,
        &sbmm_shell::arguments(Some(&name), launch),
    ))
}

/// Put a shortcut for a profile on the desktop or in the Start menu, and
/// return where it went. With `launch` it starts the game with the profile;
/// without, it opens the manager on it.
#[tauri::command]
pub fn create_profile_shortcut(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: i64,
    place: String,
    launch: bool,
) -> Result<String, String> {
    let name = profile_name(&state, profile_id)?;
    let exe = std::env::current_exe().map_err(fail)?;
    let game_exe =
        with_app!(state, |service| service.game())?.and_then(|game| game.executable_path());

    let dir = match place.as_str() {
        "desktop" => app_handle.path().desktop_dir().map_err(fail)?,
        "startMenu" => start_menu(&app_handle)?,
        other => return Err(format!("unknown place for a shortcut: {other}")),
    };
    let shortcut = if launch {
        Shortcut {
            name: format!("Stellar Blade ({name})"),
            description: format!("Start Stellar Blade with the “{name}” profile"),
            icon: Some(game_exe.unwrap_or_else(|| exe.clone())),
            ..manager_shortcut(&exe, &name, launch)
        }
    } else {
        manager_shortcut(&exe, &name, launch)
    };
    let path = sbmm_shell::create(&shortcut, &dir)
        .map_err(|e| format!("could not create the shortcut in {}: {e}", dir.display()))?;
    Ok(path.to_string_lossy().into_owned())
}

fn manager_shortcut(exe: &std::path::Path, profile: &str, launch: bool) -> Shortcut {
    Shortcut {
        name: format!("SB Mod Manager ({profile})"),
        description: format!("Open SB Mod Manager with the “{profile}” profile"),
        target: exe.to_path_buf(),
        arguments: sbmm_shell::arguments(Some(profile), launch),
        working_dir: exe.parent().map(|dir| dir.to_path_buf()),
        icon: Some(exe.to_path_buf()),
    }
}

fn profile_name(state: &tauri::State<'_, AppState>, profile_id: i64) -> Result<String, String> {
    with_app!(state, |service| service.profiles())?
        .into_iter()
        .find(|p| p.id == profile_id)
        .map(|p| p.name)
        .ok_or_else(|| "that profile no longer exists".to_string())
}

/// The user's own Start menu programs, which needs no elevation.
fn start_menu(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let data = app.path().data_dir().map_err(fail)?;
    if cfg!(windows) {
        Ok(data.join(r"Microsoft\Windows\Start Menu\Programs"))
    } else {
        Ok(data.join("applications"))
    }
}
