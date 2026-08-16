//! Commands for updating the game's DLSS and FSR DLLs.
//!
//! The rules — which DLL the hardware can use, which release line the game is
//! on, how to read what is already installed — live in `sbmm-upscaler` and are
//! tested there. This module only fetches, reports, and hands work to the app.

use std::collections::HashMap;
use std::path::PathBuf;

use sbmm_nexus::http::Fetcher;
use sbmm_nexus::{ReqwestTransport, Transport};
use sbmm_upscaler::{catalog, gpu, Adapter, Capability, Component, InstalledFile};

use crate::{fail, AppState, USER_AGENT};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscalerStatus {
    /// `None` when no display adapter could be identified.
    adapter: Option<Adapter>,
    entries: Vec<Entry>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    component: Component,
    label: String,
    /// Every copy in the game folder — a build can ship the same DLL twice,
    /// and only the one the loader reaches first actually runs.
    installed: Vec<InstalledFile>,
    /// The oldest version present, since that is the one holding the game back.
    installed_version: Option<String>,
    /// Newest release on the line this game uses.
    latest_version: Option<String>,
    /// False when the card cannot run this DLL at all.
    supported: bool,
    /// Why it is not on offer, in words the user can act on.
    blocked_reason: Option<String>,
    /// Set when the manager put the current file there.
    managed_version: Option<String>,
    note: Option<String>,
}

/// What is installed, what the card can take, and what is available.
#[tauri::command]
pub async fn upscaler_status(state: tauri::State<'_, AppState>) -> Result<UpscalerStatus, String> {
    let installed = crate::with_app!(state, |app| app.upscalers())?;
    let managed: HashMap<Component, String> = crate::with_app!(state, |app| app
        .managed_upscalers())?
    .into_iter()
    .collect();
    let adapter = gpu::primary_adapter();

    // One request per family rather than per DLL: the three DLSS files are
    // published together under the same tag.
    let mut newest = HashMap::new();
    if let Ok(transport) = ReqwestTransport::new(USER_AGENT) {
        for source in catalog::catalog() {
            if let Some(release) = latest_release(&transport, &source).await {
                newest.insert(source.family, release);
            }
        }
    }

    let mut entries = Vec::new();
    for &component in Component::all() {
        let present: Vec<InstalledFile> = installed
            .iter()
            .filter(|f| f.component == component)
            .cloned()
            .collect();
        // A DLL the game does not have is nothing to offer: a copy put where
        // the loader never looks would only take up space.
        if present.is_empty() {
            continue;
        }

        // An unknown card is treated as capable, because refusing on a failed
        // detection would be worse than letting the user decide.
        let supported = adapter
            .as_ref()
            .map(|a| a.supports(component.requires()))
            .unwrap_or(true);
        let release = newest.get(&component.family());

        entries.push(Entry {
            component,
            label: component.label().to_string(),
            installed_version: present
                .iter()
                .filter_map(|f| f.version)
                .min()
                .map(|v| v.to_string()),
            installed: present,
            latest_version: release.map(|r| r.version.to_string()),
            supported,
            blocked_reason: (!supported).then(|| unsupported_reason(component, adapter.as_ref())),
            managed_version: managed.get(&component).cloned(),
            note: catalog::source_for(component.family())
                .and_then(|s| s.version_note)
                .map(str::to_string),
        });
    }

    Ok(UpscalerStatus { adapter, entries })
}

/// Download the newest DLL for one component and put it in place.
#[tauri::command]
pub async fn update_upscaler(
    state: tauri::State<'_, AppState>,
    component: Component,
) -> Result<String, String> {
    let source = catalog::source_for(component.family())
        .ok_or_else(|| format!("no download source for {}", component.label()))?;

    if let Some(adapter) = gpu::primary_adapter() {
        if !adapter.supports(component.requires()) {
            return Err(unsupported_reason(component, Some(&adapter)));
        }
    }

    let targets: Vec<PathBuf> = crate::with_app!(state, |app| app.upscalers())?
        .into_iter()
        .filter(|f| f.component == component)
        .map(|f| f.path)
        .collect();
    if targets.is_empty() {
        return Err(format!(
            "the game folder has no {}, so there is nothing to replace",
            component.file_name()
        ));
    }

    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let release = latest_release(&transport, &source)
        .await
        .ok_or_else(|| format!("could not find a published {} release", component.label()))?;

    let into = crate::with_app!(state, |app| app.folders())?;
    let into = PathBuf::from(into.downloads).join("upscalers");
    std::fs::create_dir_all(&into).map_err(fail)?;

    // Fetched once even when the same DLL is replaced in several places.
    let downloaded = into.join(format!("{}-{}", release.tag, component.file_name()));
    // A partial file from an earlier attempt would be resumed as though it
    // belonged to this download; starting over is cheap enough to be safer.
    let _ = std::fs::remove_file(&downloaded);

    let url = source.download_url(&release.tag, component);
    transport
        .fetch(&url, &downloaded, &|_progress| {})
        .await
        .map_err(fail)?;

    // Whatever came back has to be a Windows DLL before it goes anywhere near
    // the game folder — an error page saved under the right name would
    // otherwise be installed as if it were an upscaler.
    sbmm_upscaler::pe::file_version(&downloaded)
        .map_err(|_| format!("what {} returned is not a Windows DLL", source.repo))?;

    let files: Vec<(PathBuf, PathBuf)> = targets
        .into_iter()
        .map(|target| (downloaded.clone(), target))
        .collect();
    let version = release.version.to_string();
    crate::with_app!(state, |app| app.install_upscaler(component, &version, &files))?;
    let _ = std::fs::remove_file(&downloaded);

    Ok(format!("{} {version}", component.label()))
}

/// Put the game's own DLL back.
#[tauri::command]
pub fn restore_upscaler(
    state: tauri::State<'_, AppState>,
    component: Component,
) -> Result<bool, String> {
    crate::with_app!(state, |app| app.restore_upscaler(component))
}

/// The newest release on the line this game uses.
async fn latest_release<T: Transport>(
    transport: &T,
    source: &catalog::Source,
) -> Option<catalog::Release> {
    let response = transport
        .get(
            &source.tags_url(),
            &[("Accept", "application/vnd.github+json")],
        )
        .await
        .ok()?;
    // Anonymous GitHub calls are rate limited; a refusal means "unknown", not
    // "there is nothing newer", so nothing is offered rather than guessed.
    if response.status != 200 {
        return None;
    }
    source.newest(catalog::parse_tags(&response.body).iter().map(String::as_str))
}

fn unsupported_reason(component: Component, adapter: Option<&Adapter>) -> String {
    let card = adapter.map(|a| a.name.as_str()).unwrap_or("this card");
    match component.requires() {
        Capability::DlssFrameGeneration => {
            format!("{card} cannot run DLSS Frame Generation — that needs an RTX 40 series or newer.")
        }
        Capability::DlssSuperResolution => {
            format!("{card} cannot run DLSS — that needs an RTX 20 series or newer.")
        }
        Capability::Universal => String::new(),
    }
}
