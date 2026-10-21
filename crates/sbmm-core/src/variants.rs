//! Archives that ship several versions of one mod.
//!
//! Outfit mods in particular arrive as one archive holding `Black/`, `White/`
//! and `Red/`, each a complete mod on its own and each meant to be installed
//! *instead of* the others. Installing the whole archive puts every colour in
//! the game, all claiming the same files, and whichever loads last wins —
//! which looks like the mod ignoring the user. Mod Organizer 2 answers this
//! with an installer that asks; so does this.
//!
//! The test for "these are variants" is deliberately narrow: sibling folders
//! that each detect as a mod on their own *and* put files in the same places.
//! Sibling folders that do not clash — a bundle of separate Lua mods, say —
//! are meant to be installed together, and still are, without a question.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{Confidence, ModType};
use crate::paths;
use crate::tree::{FileTree, TreeEntry};
use crate::{detect_components, plan};

/// One version of the mod the user can pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantOption {
    /// The folder's name as the archive spells it, shown to the user and
    /// remembered as their choice.
    pub label: String,
    /// The folder inside the extracted archive, relative to its root, `/`
    /// separated — what is kept when this option is chosen.
    pub folder: String,
    pub mod_types: Vec<ModType>,
    pub file_count: usize,
}

/// The versions an archive offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantChoice {
    pub options: Vec<VariantOption>,
    /// Every option clashes with every other, so exactly one makes sense.
    /// Otherwise some may be combined, and the UI warns when chosen ones
    /// clash.
    pub exclusive: bool,
}

impl VariantChoice {
    /// The folders to drop for a choice of labels, or `None` when a label is
    /// not one of the options — a remembered choice from a different version
    /// of the archive must not be applied to this one.
    pub fn folders_to_drop(&self, chosen: &[String]) -> Option<Vec<String>> {
        if chosen.is_empty()
            || chosen
                .iter()
                .any(|c| !self.options.iter().any(|o| &o.label == c))
        {
            return None;
        }
        Some(
            self.options
                .iter()
                .filter(|o| !chosen.contains(&o.label))
                .map(|o| o.folder.clone())
                .collect(),
        )
    }
}

/// The versions `tree` offers, if it is an archive of alternatives.
pub fn variant_choice(tree: &FileTree) -> Option<VariantChoice> {
    let stripped = tree.stripped();
    let payload: Vec<&TreeEntry> = stripped
        .entries()
        .iter()
        .filter(|e| !is_documentation(e))
        .collect();
    // Payload at the top level means the archive is a mod, not a menu.
    if payload.is_empty() || payload.iter().any(|e| !e.norm.contains('/')) {
        return None;
    }

    let mut groups: BTreeMap<String, Vec<&TreeEntry>> = BTreeMap::new();
    for entry in &payload {
        let first = entry.norm.split('/').next().unwrap_or_default().to_string();
        groups.entry(first).or_default().push(entry);
    }
    if groups.len() < 2
        || groups
            .keys()
            .any(|g| paths::MEANINGFUL_DIRS.contains(&g.as_str()))
    {
        return None;
    }

    let mut options = Vec::new();
    let mut claims: Vec<BTreeSet<String>> = Vec::new();
    for (key, entries) in &groups {
        let sub = stripped.subtree(key);
        let components = detect_components(&sub);
        let understood = !components.is_empty()
            && components
                .iter()
                .all(|c| c.mod_type != ModType::Unknown && c.confidence != Confidence::AskUser);
        if !understood {
            return None;
        }
        // Where the files would land. Paks are compared by the name they are
        // deployed under, which carries no load-order prefix here, so two
        // colours of `Outfit_P.pak` count as the clash they are.
        let built = plan::build_plan(0, "", &components, 0, key);
        claims.push(
            built
                .files
                .iter()
                .map(|f| f.target.to_string_lossy().to_lowercase().replace('\\', "/"))
                .collect(),
        );

        let label = entries[0]
            .rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| key.clone());
        // The folder inside the extraction: however many wrapper folders were
        // stripped, plus this one.
        let wrappers = entries[0].path.components().count() - entries[0].rel.components().count();
        let folder = entries[0]
            .path
            .components()
            .take(wrappers + 1)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let mut mod_types: Vec<ModType> = components.iter().map(|c| c.mod_type).collect();
        mod_types.dedup();
        options.push(VariantOption {
            label,
            folder,
            mod_types,
            file_count: entries.len(),
        });
    }

    let clash = |a: &BTreeSet<String>, b: &BTreeSet<String>| !a.is_disjoint(b);
    let mut any = false;
    let mut all = true;
    for i in 0..claims.len() {
        for j in i + 1..claims.len() {
            if clash(&claims[i], &claims[j]) {
                any = true;
            } else {
                all = false;
            }
        }
    }
    any.then_some(VariantChoice {
        options,
        exclusive: all,
    })
}

/// Readmes and previews, which every variant folder may carry and which say
/// nothing about whether the folders are alternatives.
fn is_documentation(entry: &TreeEntry) -> bool {
    matches!(
        entry.extension(),
        Some(
            "txt" | "md" | "pdf" | "jpg" | "jpeg" | "png" | "webp" | "gif" | "url" | "html" | "htm"
        )
    ) || matches!(entry.file_name(), "desktop.ini" | "thumbs.db" | ".ds_store")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(files: &[&str]) -> Option<VariantChoice> {
        variant_choice(&FileTree::new(files))
    }

    #[test]
    fn colours_of_one_outfit_are_alternatives() {
        let c = choice(&[
            "Planet Diving Suit/Black/Outfit_P.pak",
            "Planet Diving Suit/Black/Outfit_P.utoc",
            "Planet Diving Suit/Black/Outfit_P.ucas",
            "Planet Diving Suit/White/Outfit_P.pak",
            "Planet Diving Suit/White/Outfit_P.utoc",
            "Planet Diving Suit/White/Outfit_P.ucas",
            "Planet Diving Suit/readme.txt",
        ])
        .expect("two complete colours of one outfit");
        assert!(c.exclusive);
        let labels: Vec<&str> = c.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec!["Black", "White"]);
        assert_eq!(c.options[0].folder, "Planet Diving Suit/Black");
        assert_eq!(c.options[0].mod_types, vec![ModType::GenericPak]);
        assert_eq!(c.options[0].file_count, 3);
    }

    #[test]
    fn separate_lua_mods_in_one_archive_are_installed_together() {
        assert_eq!(
            choice(&[
                "Photo Mode/scripts/main.lua",
                "Quick Sprint/scripts/main.lua",
            ]),
            None,
            "different folders, different files: a bundle, not a menu"
        );
    }

    #[test]
    fn three_options_where_only_some_clash_may_be_combined() {
        let c = choice(&[
            "Black/Outfit_P.pak",
            "White/Outfit_P.pak",
            "Hair/Hair_P.pak",
        ])
        .unwrap();
        assert!(!c.exclusive, "the hair goes with either colour");
        assert_eq!(c.options.len(), 3);
    }

    #[test]
    fn a_mod_with_files_at_the_top_is_not_a_menu() {
        assert_eq!(choice(&["Outfit_P.pak", "Extras/Outfit_P.pak"]), None);
        assert_eq!(choice(&["Outfit_P.pak"]), None);
    }

    #[test]
    fn a_structured_mod_is_not_mistaken_for_variants() {
        assert_eq!(
            choice(&[
                "SB/Content/Paks/~mods/Outfit_P.pak",
                "LogicMods/Outfit_P.pak",
            ]),
            None
        );
    }

    #[test]
    fn an_option_detection_cannot_place_stops_the_offer() {
        assert_eq!(choice(&["A/something.bin", "B/something.bin"]), None);
    }

    #[test]
    fn a_remembered_choice_applies_only_to_options_that_exist() {
        let c = choice(&["Black/Outfit_P.pak", "White/Outfit_P.pak"]).unwrap();
        assert_eq!(
            c.folders_to_drop(&["White".into()]),
            Some(vec!["Black".to_string()])
        );
        assert_eq!(c.folders_to_drop(&["Red".into()]), None);
        assert_eq!(c.folders_to_drop(&[]), None);
    }
}
