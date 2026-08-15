//! Turns detected components into a concrete list of files to lay down.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::loadorder;
use crate::model::{ComponentFile, Confidence, DetectedComponent, ModType};
use crate::paths;

/// One file to materialise in the game folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFile {
    /// Relative to the mod's staging directory.
    pub source: PathBuf,
    /// Relative to the game root.
    pub target: PathBuf,
}

/// Everything the deployment backend needs to install one mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployPlan {
    pub mod_id: i64,
    /// Absolute path of the mod's staging directory.
    pub staging_root: PathBuf,
    pub files: Vec<PlannedFile>,
    /// UE4SS mod folder names that must appear in `mods.txt`.
    pub ue4ss_registrations: Vec<String>,
}

impl DeployPlan {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Build the deployment plan for one mod.
///
/// `priority` drives the numeric prefix applied to load-ordered paks;
/// `mod_name` is the fallback folder name for UE4SS mods whose archive did not
/// carry one.
pub fn build_plan(
    mod_id: i64,
    staging_root: impl Into<PathBuf>,
    components: &[DetectedComponent],
    priority: i64,
    mod_name: &str,
) -> DeployPlan {
    let mut files = Vec::new();
    let mut ue4ss_registrations = Vec::new();

    for component in components {
        let mut component_files = component.files.clone();

        // A UE4SS mod with no folder of its own is nested under the mod name.
        if component.mod_type.needs_ue4ss_registration() {
            let folder = match &component.ue4ss_mod_name {
                Some(name) => name.clone(),
                None => {
                    let folder = sanitize_folder_name(mod_name);
                    rebase_under(&mut component_files, paths::UE4SS_MODS, &folder);
                    folder
                }
            };
            if !ue4ss_registrations.contains(&folder) {
                ue4ss_registrations.push(folder);
            }
        }

        if component.mod_type.is_load_ordered() {
            loadorder::apply_load_order(&mut component_files, &component.pak_sets, priority);
        }

        files.extend(component_files.into_iter().map(|f| PlannedFile {
            source: f.source,
            target: f.target,
        }));
    }

    DeployPlan {
        mod_id,
        staging_root: staging_root.into(),
        files,
        ue4ss_registrations,
    }
}

/// Re-route a component after the user corrects its detected type.
///
/// Detection is a guess for `Unknown` payloads; this recomputes destinations
/// from scratch for the chosen type rather than patching the old ones.
pub fn retarget(component: &DetectedComponent, new_type: ModType, mod_name: &str) -> DetectedComponent {
    let target_root = match new_type {
        ModType::Ue4ssLua | ModType::Ue4ssDll => {
            let folder = component
                .ue4ss_mod_name
                .clone()
                .unwrap_or_else(|| sanitize_folder_name(mod_name));
            format!("{}/{}", paths::UE4SS_MODS, folder)
        }
        other => other.target_subdir().to_string(),
    };

    let files = component
        .files
        .iter()
        .map(|f| {
            let target = if new_type.is_flat_target() {
                let name = f
                    .source
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| f.source.clone());
                Path::new(&target_root).join(name)
            } else {
                Path::new(&target_root).join(&f.source)
            };
            ComponentFile {
                source: f.source.clone(),
                target,
            }
        })
        .collect();

    let entries_for_paks: Vec<crate::tree::TreeEntry> = component
        .files
        .iter()
        .map(|f| crate::tree::TreeEntry {
            norm: f.source.to_string_lossy().to_ascii_lowercase().replace('\\', "/"),
            rel: f.source.clone(),
            path: f.source.clone(),
        })
        .collect();
    let (pak_sets, warnings) = crate::detect::build_pak_sets(&entries_for_paks);

    DetectedComponent {
        mod_type: new_type,
        confidence: Confidence::High,
        files,
        target_subdir: PathBuf::from(&target_root),
        ue4ss_mod_name: component.ue4ss_mod_name.clone(),
        pak_sets: if new_type.is_load_ordered() {
            pak_sets
        } else {
            Vec::new()
        },
        warnings,
    }
}

/// Move every target from `<parent>/...` to `<parent>/<folder>/...`.
fn rebase_under(files: &mut [ComponentFile], parent: &str, folder: &str) {
    let parent_path = Path::new(parent);
    let new_parent = parent_path.join(folder);
    for file in files.iter_mut() {
        if let Ok(rest) = file.target.strip_prefix(parent_path) {
            file.target = new_parent.join(rest);
        }
    }
}

/// Strip characters that are not valid in a Windows directory name.
pub fn sanitize_folder_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "Mod".to_string()
    } else {
        trimmed.to_string()
    }
}
