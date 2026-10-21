//! Commands for installing and updating UE4SS.
//!
//! Which build, how to tell it from anything else, and how it goes in all
//! live in `sbmm_app::ue4ss`; this module only asks GitHub and downloads.

use std::path::PathBuf;

use sbmm_app::ue4ss;
use sbmm_nexus::http::Fetcher;
use sbmm_nexus::{ReqwestTransport, Transport};
use sbmm_upscaler::catalog;

use crate::{fail, AppState, USER_AGENT};

/// The newest Stellar Blade build of UE4SS, or `None` when GitHub could not
/// be asked — which is "unknown", never "you are up to date".
#[tauri::command]
pub async fn ue4ss_latest() -> Result<Option<String>, String> {
    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    Ok(latest(&transport).await)
}

/// Download the newest build and install it, replacing the one the manager
/// installed before.
#[tauri::command]
pub async fn install_ue4ss(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let tag = latest(&transport)
        .await
        .ok_or("could not find a published UE4SS release for Stellar Blade on GitHub")?;

    let downloads = crate::with_app!(state, |app| app.folders())?.downloads;
    let archive = PathBuf::from(downloads)
        .join("ue4ss")
        .join(format!("UE4SS_v{tag}.zip"));
    // A partial file from an earlier attempt would be resumed as though it
    // were this one; the archive is small enough to fetch again.
    let _ = std::fs::remove_file(&archive);
    transport
        .fetch(&ue4ss::download_url(&tag), &archive, &|_| {})
        .await
        .map_err(fail)?;

    let result = crate::with_app!(state, |app| app.install_ue4ss(&archive, &tag));
    let _ = std::fs::remove_file(&archive);
    result?;
    Ok(format!("UE4SS {tag} for Stellar Blade installed."))
}

async fn latest<T: Transport>(transport: &T) -> Option<String> {
    let response = transport
        .get(
            &ue4ss::tags_url(),
            &[("Accept", "application/vnd.github+json")],
        )
        .await
        .ok()?;
    if response.status != 200 {
        return None;
    }
    ue4ss::newest(catalog::parse_tags(&response.body).iter().map(String::as_str))
}
