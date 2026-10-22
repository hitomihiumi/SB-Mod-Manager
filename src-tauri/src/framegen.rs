//! Commands for the experimental RTX 20/30 frame generation package.
//!
//! What the package must contain, whether this card and game can use it, and
//! how it is laid down all live in `sbmm-upscaler::framegen` and `sbmm-app`,
//! where they are tested. This module fetches the release, unpacks it off the
//! async runtime, and hands the result to the app.

use std::path::{Path, PathBuf};

use sbmm_app::dto::FramegenInstall;
use sbmm_nexus::http::Fetcher;
use sbmm_nexus::{ReqwestTransport, Transport};
use sbmm_upscaler::framegen::{self, Assessment, Proxy};
use sbmm_upscaler::{catalog, gpu, Adapter};
use tauri::Emitter;

use crate::{fail, AppState, USER_AGENT};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramegenStatus {
    adapter: Option<Adapter>,
    /// `None` when no game folder has been chosen yet.
    assessment: Option<Assessment>,
    /// The proxy name the assessment was made for.
    proxy: Proxy,
    installed: Option<FramegenInstall>,
    /// Newest installable release, or `None` when GitHub could not be asked.
    latest_version: Option<String>,
    project_url: &'static str,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FramegenProgress {
    stage: &'static str,
    done: u64,
    total: Option<u64>,
}

/// Whether the package can be used here, and what is installed.
///
/// `proxy` is the name the user is considering; without one the installed
/// name, or upstream's first choice, is checked.
#[tauri::command]
pub async fn framegen_status(
    state: tauri::State<'_, AppState>,
    proxy: Option<Proxy>,
) -> Result<FramegenStatus, String> {
    let installed = crate::with_app!(state, |app| app.framegen_installed())?;
    let proxy = proxy
        .or(installed.as_ref().map(|i| i.proxy))
        .unwrap_or_default();
    let facts = crate::with_app!(state, |app| app.framegen_facts())?;
    let adapter = gpu::primary_adapter();
    let assessment = facts
        .as_ref()
        .map(|facts| framegen::assess(adapter.as_ref(), facts, proxy));

    let latest_version = match ReqwestTransport::new(USER_AGENT) {
        Ok(transport) => latest_release(&transport).await.map(|r| r.tag),
        Err(_) => None,
    };

    Ok(FramegenStatus {
        adapter,
        assessment,
        proxy,
        installed,
        latest_version,
        project_url: framegen::PROJECT_URL,
    })
}

/// Download the newest release, check it, and put it beside the game
/// executable under `proxy`.
#[tauri::command]
pub async fn install_framegen(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    proxy: Proxy,
) -> Result<String, String> {
    // Checked again here rather than trusted from the screen: the folder may
    // have changed since the status was read.
    let facts = crate::with_app!(state, |app| app.framegen_facts())?
        .ok_or_else(|| "choose the game folder first".to_string())?;
    let assessment = framegen::assess(gpu::primary_adapter().as_ref(), &facts, proxy);
    if !assessment.installable() {
        return Err(assessment
            .blocker()
            .unwrap_or_else(|| "this game or card cannot use the package".into()));
    }

    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let release = latest_release(&transport)
        .await
        .ok_or("could not find a published dlssg_for_sm86 release on GitHub")?;

    let downloads = crate::with_app!(state, |app| app.folders())?.downloads;
    let into = PathBuf::from(downloads).join("framegen");
    std::fs::create_dir_all(&into).map_err(fail)?;
    let archive = into.join(framegen::asset_name(&release.tag));
    let unpacked = into.join(format!("{}-unpacked", release.tag));

    // The archive is kept after a successful install, so switching the proxy
    // name later does not cost another 80 MB. A kept one that no longer
    // checks out is fetched again, once.
    let package = match unpack(&archive, &unpacked).await {
        Ok(package) => package,
        Err(_) => {
            let _ = std::fs::remove_file(&archive);
            let emitter = app_handle.clone();
            transport
                .fetch(
                    &framegen::download_url(&release.tag),
                    &archive,
                    &move |progress| {
                        let _ = emitter.emit(
                            "framegen-progress",
                            FramegenProgress {
                                stage: "download",
                                done: progress.done,
                                total: progress.total,
                            },
                        );
                    },
                )
                .await
                .map_err(fail)?;
            let _ = app_handle.emit(
                "framegen-progress",
                FramegenProgress {
                    stage: "verify",
                    done: 0,
                    total: None,
                },
            );
            unpack(&archive, &unpacked).await.inspect_err(|_| {
                // A download that fails its own manifest is not worth keeping.
                let _ = std::fs::remove_file(&archive);
            })?
        }
    };

    let version = package.version.clone().unwrap_or(release.tag.clone());
    let result = crate::with_app!(state, |app| app.install_framegen(&package, proxy, &version));
    let _ = std::fs::remove_dir_all(&unpacked);
    result?;

    Ok(format!(
        "DLSS Frame Generation for RTX 20/30 {version} installed as {}. Turn on Frame \
         Generation in the game's graphics settings.",
        proxy.file_name()
    ))
}

/// Take the package back out of the game folder.
#[tauri::command(async)]
pub fn remove_framegen(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    crate::with_app!(state, |app| app.remove_framegen())
}

/// Extract and inspect on a blocking thread: the release is over 200 MB
/// unpacked and every deployed file is hashed.
async fn unpack(archive: &Path, into: &Path) -> Result<framegen::Package, String> {
    if !archive.is_file() {
        return Err("not downloaded yet".into());
    }
    let archive = archive.to_path_buf();
    let into = into.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = std::fs::remove_dir_all(&into);
        sbmm_archive::extract(&archive, &into).map_err(fail)?;
        framegen::inspect(&into).map_err(|e| {
            let _ = std::fs::remove_dir_all(&into);
            fail(e)
        })
    })
    .await
    .map_err(fail)?
}

async fn latest_release<T: Transport>(transport: &T) -> Option<catalog::Release> {
    let response = transport
        .get(
            &framegen::tags_url(),
            &[("Accept", "application/vnd.github+json")],
        )
        .await
        .ok()?;
    // As with the upscalers: a refusal is "unknown", not "nothing published".
    if response.status != 200 {
        return None;
    }
    framegen::newest(
        catalog::parse_tags(&response.body)
            .iter()
            .map(String::as_str),
    )
}
