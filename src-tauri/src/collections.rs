//! Installing a Nexus collection.
//!
//! A collection is a list of mod files someone else put together. Resolving it
//! produces that list; installing it pushes the list into the download queue,
//! which already knows how to fetch a file for a Premium account and how to
//! walk a free account through the website one mod at a time. Nothing about
//! the two modes is repeated here — a collection is just a lot of downloads.

use std::path::PathBuf;

use sbmm_nexus::http::Fetcher;
use sbmm_nexus::queue::QueueItem;
use sbmm_nexus::{ManualMod, NexusClient, ReqwestTransport};

use crate::downloads::Downloads;
use crate::{fail, AppState, USER_AGENT};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionPlan {
    pub name: String,
    pub slug: String,
    pub revision: u64,
    /// Everything the collection asks for, in the order it listed them.
    pub mods: Vec<PlannedMod>,
    /// Entries hosted somewhere the API cannot reach. Listed rather than
    /// dropped, because a collection is not installed until these are handled.
    pub manual: Vec<ManualMod>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMod {
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
    pub optional: bool,
    /// True when this exact file is already installed, so it can be skipped.
    pub installed: bool,
}

/// Work out what a collection contains.
#[tauri::command]
pub async fn resolve_collection(
    state: tauri::State<'_, AppState>,
    link: String,
) -> Result<CollectionPlan, String> {
    let (slug, revision) = parse_link(link.trim())?;

    let key = crate::with_app!(state, |app| app.nexus_api_key())?.unwrap_or_default();
    if key.is_empty() {
        return Err("add your Nexus API key first".into());
    }
    let transport = ReqwestTransport::new(USER_AGENT).map_err(fail)?;
    let client = NexusClient::new(transport.clone(), key, sbmm_game::NEXUS_DOMAIN);

    let found = client.collection_revision(&slug, revision).await.map_err(fail)?;

    // The archive's collection.json is the authoritative list and is the only
    // place entries hosted off Nexus are named, so it is preferred whenever
    // the response offered a link to it.
    let mut manual = Vec::new();
    let mut mods = found.mods.clone();
    if let Some(url) = &found.download_url {
        match fetch_manifest(&state, &transport, url).await {
            Ok(collection) => {
                mods = collection.mods;
                manual = collection.manual;
            }
            // Falling back to the inline list is better than failing: it is
            // the same mods, only without the off-Nexus entries.
            Err(_) if !mods.is_empty() => {}
            Err(error) => return Err(error),
        }
    }

    if mods.is_empty() && manual.is_empty() {
        return Err(format!(
            "the collection {slug} came back empty — check the link, \
             or that the collection is published"
        ));
    }

    let installed = crate::with_app!(state, |app| app.nexus_mods())?;
    let planned = mods
        .into_iter()
        .map(|entry| PlannedMod {
            installed: installed
                .iter()
                .any(|m| m.nexus_mod_id as u64 == entry.mod_id),
            mod_id: entry.mod_id,
            file_id: entry.file_id,
            name: entry.name,
            version: entry.version,
            optional: entry.optional,
        })
        .collect();

    Ok(CollectionPlan {
        name: found.name,
        slug,
        revision: if found.revision > 0 {
            found.revision
        } else {
            revision
        },
        mods: planned,
        manual,
    })
}

/// Queue the chosen files.
///
/// Returns how many were added. The queue does the rest: a Premium account
/// sees them download one after another, a free account is walked through the
/// website a mod at a time, and either way installing is automatic.
#[tauri::command]
pub fn install_collection(
    downloads: tauri::State<'_, Downloads>,
    slug: String,
    files: Vec<QueuedFile>,
) -> usize {
    for file in &files {
        downloads.queue.push(
            QueueItem::new(
                file.mod_id,
                file.file_id,
                file.name.clone(),
                format!("{}-{}.zip", file.mod_id, file.file_id),
            )
            .with_version(file.version.clone())
            .in_collection(slug.clone()),
        );
    }
    files.len()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedFile {
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
}

/// Download the revision archive and read `collection.json` out of it.
async fn fetch_manifest(
    state: &tauri::State<'_, AppState>,
    fetcher: &ReqwestTransport,
    url: &str,
) -> Result<sbmm_nexus::Collection, String> {
    let folders = crate::with_app!(state, |app| app.folders())?;
    let into = PathBuf::from(folders.downloads).join("collections");
    std::fs::create_dir_all(&into).map_err(fail)?;

    let archive = into.join("revision.zip");
    // A leftover from a different collection would be resumed as if it were
    // this one, so the slate is cleared first.
    let _ = std::fs::remove_file(&archive);
    fetcher
        .fetch(url, &archive, &|_progress| {})
        .await
        .map_err(fail)?;

    let extracted = into.join("revision");
    let _ = std::fs::remove_dir_all(&extracted);
    sbmm_archive::extract(&archive, &extracted).map_err(fail)?;

    let manifest = find_manifest(&extracted)
        .ok_or_else(|| "the collection archive has no collection.json".to_string())?;
    let json = std::fs::read_to_string(&manifest).map_err(fail)?;

    let parsed = sbmm_nexus::collection::parse(&json).map_err(fail)?;
    let _ = std::fs::remove_dir_all(&extracted);
    let _ = std::fs::remove_file(&archive);
    Ok(parsed)
}

/// The manifest is usually at the root, but a wrapper folder is common enough
/// in archives generally that it is worth looking one level down.
fn find_manifest(root: &std::path::Path) -> Option<PathBuf> {
    let direct = root.join("collection.json");
    if direct.is_file() {
        return Some(direct);
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let nested = entry.path().join("collection.json");
        if nested.is_file() {
            return Some(nested);
        }
    }
    None
}

/// Pull the slug and revision out of whatever the user pasted.
///
/// All three forms are things a user actually ends up holding: the `nxm://`
/// link the site's button produces, the page URL from the address bar, and the
/// bare slug from either of those.
fn parse_link(link: &str) -> Result<(String, u64), String> {
    if link.is_empty() {
        return Err("paste a collection link".into());
    }

    if link.to_ascii_lowercase().starts_with("nxm://") {
        return match sbmm_nexus::nxm::parse(link, sbmm_game::NEXUS_DOMAIN).map_err(fail)? {
            sbmm_nexus::NxmLink::Collection(c) => Ok((c.slug, c.revision)),
            sbmm_nexus::NxmLink::File(_) => {
                Err("that is a link to a single mod, not a collection".into())
            }
        };
    }

    if let Some(rest) = link.split("/collections/").nth(1) {
        let mut parts = rest.split('/').filter(|p| !p.is_empty());
        let slug = parts
            .next()
            .ok_or_else(|| "that link has no collection in it".to_string())?;
        // .../collections/<slug>/revisions/<n>
        let revision = match parts.next() {
            Some("revisions") => parts.next().and_then(|n| n.parse().ok()).unwrap_or(0),
            _ => 0,
        };
        return Ok((strip_query(slug).to_string(), revision));
    }

    // A bare slug. Anything with a slash in it is a URL we failed to read,
    // and guessing at it would produce a confusing "collection came back
    // empty" further down.
    if link.contains('/') || link.contains(' ') {
        return Err(format!("could not find a collection in {link}"));
    }
    Ok((strip_query(link).to_string(), 0))
}

fn strip_query(value: &str) -> &str {
    value
        .split_once('?')
        .map(|(before, _)| before)
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collection_link_is_read_in_any_of_the_forms_a_user_has() {
        assert_eq!(
            parse_link("nxm://stellarblade/collections/abc123/revisions/4").unwrap(),
            ("abc123".to_string(), 4)
        );
        assert_eq!(
            parse_link("https://next.nexusmods.com/stellarblade/collections/abc123").unwrap(),
            ("abc123".to_string(), 0)
        );
        assert_eq!(
            parse_link("https://next.nexusmods.com/stellarblade/collections/abc123?tab=mods")
                .unwrap(),
            ("abc123".to_string(), 0)
        );
        assert_eq!(
            parse_link("abc123").unwrap(),
            ("abc123".to_string(), 0),
            "the slug on its own is what people copy out of a URL"
        );
    }

    #[test]
    fn a_page_url_carrying_a_revision_keeps_it() {
        assert_eq!(
            parse_link("https://next.nexusmods.com/stellarblade/collections/abc123/revisions/12")
                .unwrap(),
            ("abc123".to_string(), 12)
        );
    }

    #[test]
    fn anything_that_is_not_a_collection_is_refused_with_a_reason() {
        assert!(parse_link("").is_err());
        assert!(parse_link("https://www.nexusmods.com/stellarblade/mods/440").is_err());
        assert!(
            parse_link("nxm://stellarblade/mods/440/files/1899").is_err(),
            "a single-mod link goes through the downloads screen instead"
        );
        assert!(
            parse_link("nxm://skyrimspecialedition/collections/abc").is_err(),
            "a collection for another game cannot be installed here"
        );
    }
}
