//! Works out what an extracted archive contains and where each piece belongs.
//!
//! Rules are applied in order of decreasing specificity. Each rule *claims* the
//! files it recognises and the remainder falls through to the next one, so a
//! single archive that mixes (say) a logic mod with a cosmetic pak yields two
//! independently routed components rather than one wrong verdict.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::model::{ComponentFile, Confidence, DetectedComponent, ModType, PakSet};
use crate::paths;
use crate::tree::{FileTree, TreeEntry};

/// Extensions that are documentation or previews rather than mod payload.
const JUNK_EXTS: &[&str] = &[
    "txt", "md", "rtf", "pdf", "doc", "docx", "jpg", "jpeg", "png", "webp", "gif", "bmp", "url",
    "html", "htm", "nfo", "log",
];

const JUNK_NAMES: &[&str] = &["desktop.ini", "thumbs.db", ".ds_store"];

/// Extensions that mark a loose injector payload.
const ROOT_BINARY_EXTS: &[&str] = &["asi", "addon", "addon64"];

/// Classify every file in an extracted archive.
pub fn detect_components(tree: &FileTree) -> Vec<DetectedComponent> {
    let tree = tree.stripped();
    if tree.is_empty() {
        return Vec::new();
    }

    // An archive that already spells out `SB/...` tells us exactly where each
    // file goes, so trust it and skip the guesswork for that portion.
    let (sb_entries, other_entries): (Vec<_>, Vec<_>) = tree
        .entries()
        .iter()
        .cloned()
        .partition(|e| e.norm == "sb" || e.norm.starts_with("sb/"));

    let mut components = Vec::new();
    if !sb_entries.is_empty() {
        components.extend(detect_game_relative(&sb_entries));
    }
    if !other_entries.is_empty() {
        let rest = FileTree::new(other_entries.iter().map(|e| e.path.clone()));
        components.extend(detect_heuristic(&rest));
    }

    if components.is_empty() {
        components.push(unknown_component(tree.entries()));
    }
    components
}

// ---------------------------------------------------------------------------
// Explicit game-relative trees (CNS-style packages)
// ---------------------------------------------------------------------------

fn detect_game_relative(entries: &[TreeEntry]) -> Vec<DetectedComponent> {
    // Bucket by destination. UE4SS mods additionally key on their folder name
    // so two bundled mods do not get merged into one.
    let mut buckets: BTreeMap<(ModType, Option<String>), Vec<TreeEntry>> = BTreeMap::new();

    for entry in entries {
        let rel = match entry.norm.strip_prefix("sb/") {
            Some(r) => r,
            None => continue, // the bare `SB` directory entry itself
        };
        let (mod_type, ue4ss_name) = classify_game_relative(rel, entry);
        buckets
            .entry((mod_type, ue4ss_name))
            .or_default()
            .push(entry.clone());
    }

    buckets
        .into_iter()
        .map(|((mod_type, ue4ss_name), entries)| {
            // The archive path is already the game-relative path.
            let files: Vec<ComponentFile> = entries
                .iter()
                .map(|e| ComponentFile {
                    source: e.path.clone(),
                    target: e.path.clone(),
                })
                .collect();
            let (pak_sets, mut warnings) = build_pak_sets(&entries);
            if mod_type == ModType::GameRootOverlay {
                warnings.push(
                    "Ships explicit game paths and will be laid down directly onto the game folder."
                        .to_string(),
                );
            }
            DetectedComponent {
                mod_type,
                confidence: Confidence::High,
                files,
                target_subdir: PathBuf::from(mod_type.target_subdir()),
                ue4ss_mod_name: ue4ss_name,
                pak_sets,
                warnings,
            }
        })
        .collect()
}

/// Map a path already relative to `SB/` onto a mod type.
fn classify_game_relative(rel: &str, entry: &TreeEntry) -> (ModType, Option<String>) {
    if rel.starts_with("content/paks/logicmods/") {
        return (ModType::LogicMod, None);
    }
    if rel.starts_with("content/movies/") {
        return (ModType::Movie, None);
    }
    if rel.starts_with("content/paks/") {
        return (ModType::GenericPak, None);
    }
    if let Some(rest) = rel.strip_prefix("binaries/win64/ue4ss/mods/") {
        // `binaries/win64/ue4ss/mods/<Name>/...`
        if let Some((name, tail)) = rest.split_once('/') {
            let mod_type = if tail.starts_with("dlls/") {
                ModType::Ue4ssDll
            } else {
                ModType::Ue4ssLua
            };
            // Recover the original casing of the folder name for the filesystem.
            let original = nth_component(&entry.path, 5).unwrap_or_else(|| name.to_string());
            return (mod_type, Some(original));
        }
        // A bare `mods.txt` and similar belong to the framework.
        return (ModType::Ue4ssFramework, None);
    }
    if rel.starts_with("binaries/win64/ue4ss/") {
        return (ModType::Ue4ssFramework, None);
    }
    if rel.starts_with("binaries/win64/") {
        return (ModType::RootBinary, None);
    }
    (ModType::GameRootOverlay, None)
}

// ---------------------------------------------------------------------------
// Loose archives
// ---------------------------------------------------------------------------

fn detect_heuristic(tree: &FileTree) -> Vec<DetectedComponent> {
    let mut claimed: HashSet<String> = HashSet::new();
    let mut components: Vec<DetectedComponent> = Vec::new();

    claim_ue4ss_framework(tree, &mut claimed, &mut components);
    claim_logic_mods(tree, &mut claimed, &mut components);
    claim_ue4ss_mods(tree, &mut claimed, &mut components);
    claim_movies(tree, &mut claimed, &mut components);
    claim_root_binaries(tree, &mut claimed, &mut components);
    claim_generic_paks(tree, &mut claimed, &mut components);

    // Anything left that is not documentation means we genuinely do not know.
    let leftovers: Vec<TreeEntry> = tree
        .entries()
        .iter()
        .filter(|e| !claimed.contains(&e.norm) && !is_junk(e))
        .cloned()
        .collect();
    if !leftovers.is_empty() {
        components.push(unknown_component(&leftovers));
    }

    components
}

/// The full UE4SS distribution: a DLL proxy plus the `ue4ss` runtime folder.
///
/// Distinguished from a *Lua mod that happens to be packaged with `ue4ss/Mods/`
/// paths* by the presence of `UE4SS.dll` itself.
fn claim_ue4ss_framework(
    tree: &FileTree,
    claimed: &mut HashSet<String>,
    out: &mut Vec<DetectedComponent>,
) {
    let has_core = tree
        .entries()
        .iter()
        .any(|e| e.file_name() == "ue4ss.dll");
    if !has_core {
        return;
    }

    let entries: Vec<TreeEntry> = tree
        .entries()
        .iter()
        .filter(|e| !claimed.contains(&e.norm) && !is_junk(e))
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }

    // The archive root corresponds to `Binaries/Win64`, so the whole structure
    // is preserved verbatim beneath it.
    let files = map_preserving(&entries, paths::BINARIES_WIN64, 0);
    mark_claimed(&entries, claimed);
    out.push(DetectedComponent {
        mod_type: ModType::Ue4ssFramework,
        confidence: Confidence::High,
        files,
        target_subdir: PathBuf::from(paths::BINARIES_WIN64),
        ue4ss_mod_name: None,
        pak_sets: Vec::new(),
        warnings: vec![
            "Installs the UE4SS runtime. Required by Lua, C++ and Blueprint logic mods."
                .to_string(),
        ],
    });
}

fn claim_logic_mods(
    tree: &FileTree,
    claimed: &mut HashSet<String>,
    out: &mut Vec<DetectedComponent>,
) {
    let entries: Vec<TreeEntry> = tree
        .entries_under_segment("logicmods")
        .into_iter()
        .filter(|e| !claimed.contains(&e.norm) && !is_junk(e))
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }

    let files = map_flat(&entries, paths::PAKS_LOGICMODS);
    let (pak_sets, warnings) = build_pak_sets(&entries);
    mark_claimed(&entries, claimed);
    out.push(DetectedComponent {
        mod_type: ModType::LogicMod,
        confidence: Confidence::High,
        files,
        target_subdir: PathBuf::from(paths::PAKS_LOGICMODS),
        ue4ss_mod_name: None,
        pak_sets,
        warnings,
    });
}

/// UE4SS Lua and C++ mods, identified by their `scripts/main.lua` or
/// `dlls/main.dll` marker.
fn claim_ue4ss_mods(
    tree: &FileTree,
    claimed: &mut HashSet<String>,
    out: &mut Vec<DetectedComponent>,
) {
    let mut dirs: Vec<String> = tree.dirs_containing("scripts/main.lua");
    for d in tree.dirs_containing("dlls/main.dll") {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs.sort();

    for dir in dirs {
        let entries: Vec<TreeEntry> = tree
            .entries_in_dir(&dir)
            .into_iter()
            .filter(|e| !claimed.contains(&e.norm))
            .cloned()
            .collect();
        if entries.is_empty() {
            continue;
        }

        // A folder carrying both markers is still one mod; Lua takes the label.
        let is_lua = entries
            .iter()
            .any(|e| e.norm.ends_with("scripts/main.lua"));
        let mod_type = if is_lua {
            ModType::Ue4ssLua
        } else {
            ModType::Ue4ssDll
        };

        let depth = if dir.is_empty() {
            0
        } else {
            dir.matches('/').count() + 1
        };
        let name = if dir.is_empty() {
            None
        } else {
            nth_component(&entries[0].path, depth - 1)
        };

        let target_root = match &name {
            Some(n) => format!("{}/{}", paths::UE4SS_MODS, n),
            None => paths::UE4SS_MODS.to_string(),
        };
        let files = map_preserving(&entries, &target_root, depth);
        mark_claimed(&entries, claimed);

        let mut warnings = vec![
            "Will be registered in UE4SS mods.txt so the loader picks it up.".to_string(),
        ];
        if name.is_none() {
            warnings.push(
                "The archive has no mod folder; the mod name will be used as the folder name."
                    .to_string(),
            );
        }

        out.push(DetectedComponent {
            mod_type,
            confidence: Confidence::High,
            files,
            target_subdir: PathBuf::from(&target_root),
            ue4ss_mod_name: name,
            pak_sets: Vec::new(),
            warnings,
        });
    }
}

fn claim_movies(tree: &FileTree, claimed: &mut HashSet<String>, out: &mut Vec<DetectedComponent>) {
    let entries: Vec<TreeEntry> = tree
        .entries()
        .iter()
        .filter(|e| !claimed.contains(&e.norm))
        .filter(|e| {
            e.segments().contains(&"movies")
                || e.extension()
                    .is_some_and(|ext| paths::MOVIE_EXTS.contains(&ext))
        })
        .filter(|e| !is_junk(e))
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }

    let files = map_flat(&entries, paths::CONTENT_MOVIES);
    mark_claimed(&entries, claimed);
    out.push(DetectedComponent {
        mod_type: ModType::Movie,
        confidence: Confidence::Medium,
        files,
        target_subdir: PathBuf::from(paths::CONTENT_MOVIES),
        ue4ss_mod_name: None,
        pak_sets: Vec::new(),
        warnings: vec!["Replaces a game movie file of the same name.".to_string()],
    });
}

/// ReShade and similar: a proxy DLL next to the executable, plus its assets.
fn claim_root_binaries(
    tree: &FileTree,
    claimed: &mut HashSet<String>,
    out: &mut Vec<DetectedComponent>,
) {
    let top_level: HashSet<String> = tree
        .top_level_files()
        .iter()
        .map(|e| e.norm.clone())
        .collect();

    let has_marker = tree.entries().iter().any(|e| {
        (top_level.contains(&e.norm) && paths::PROXY_DLLS.contains(&e.file_name()))
            || e.file_name() == "reshade.ini"
            || e.extension()
                .is_some_and(|ext| ROOT_BINARY_EXTS.contains(&ext))
            || e.segments().contains(&"reshade-shaders")
            || e.segments().contains(&"reshade-presets")
    });
    if !has_marker {
        return;
    }

    let entries: Vec<TreeEntry> = tree
        .entries()
        .iter()
        .filter(|e| !claimed.contains(&e.norm) && !is_junk(e))
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }

    // Shader and preset folders must keep their structure, so preserve paths.
    let files = map_preserving(&entries, paths::BINARIES_WIN64, 0);
    mark_claimed(&entries, claimed);
    out.push(DetectedComponent {
        mod_type: ModType::RootBinary,
        confidence: Confidence::Medium,
        files,
        target_subdir: PathBuf::from(paths::BINARIES_WIN64),
        ue4ss_mod_name: None,
        pak_sets: Vec::new(),
        warnings: vec![
            "Installs next to the game executable. Only one proxy DLL of a given name can be active."
                .to_string(),
        ],
    });
}

fn claim_generic_paks(
    tree: &FileTree,
    claimed: &mut HashSet<String>,
    out: &mut Vec<DetectedComponent>,
) {
    let entries: Vec<TreeEntry> = tree
        .entries()
        .iter()
        .filter(|e| !claimed.contains(&e.norm))
        .filter(|e| {
            e.extension()
                .is_some_and(|ext| matches!(ext, "pak" | "utoc" | "ucas" | "sig"))
        })
        .cloned()
        .collect();
    if entries.is_empty() {
        return;
    }

    let files = map_flat(&entries, paths::PAKS_MODS);
    let (pak_sets, warnings) = build_pak_sets(&entries);
    mark_claimed(&entries, claimed);
    out.push(DetectedComponent {
        mod_type: ModType::GenericPak,
        confidence: Confidence::High,
        files,
        target_subdir: PathBuf::from(paths::PAKS_MODS),
        ue4ss_mod_name: None,
        pak_sets,
        warnings,
    });
}

fn unknown_component(entries: &[TreeEntry]) -> DetectedComponent {
    DetectedComponent {
        mod_type: ModType::Unknown,
        confidence: Confidence::AskUser,
        files: entries
            .iter()
            .map(|e| ComponentFile {
                source: e.path.clone(),
                target: e.path.clone(),
            })
            .collect(),
        target_subdir: PathBuf::new(),
        ue4ss_mod_name: None,
        pak_sets: Vec::new(),
        warnings: vec!["Could not determine where these files belong.".to_string()],
    }
}

// ---------------------------------------------------------------------------
// Pak sets
// ---------------------------------------------------------------------------

/// Group `.pak` files with their IoStore siblings.
///
/// Stellar Blade is UE 4.26 with IoStore, so a mod is usually a
/// `.pak` + `.utoc` + `.ucas` triplet that must keep one shared base name.
pub fn build_pak_sets(entries: &[TreeEntry]) -> (Vec<PakSet>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut by_stem: BTreeMap<String, Vec<&TreeEntry>> = BTreeMap::new();

    for entry in entries {
        let Some(ext) = entry.extension() else { continue };
        if !matches!(ext, "pak" | "utoc" | "ucas") {
            continue;
        }
        // Key on directory + stem so identically named paks in different
        // folders are not merged.
        let stem = entry
            .norm
            .rsplit_once('.')
            .map(|(s, _)| s.to_string())
            .unwrap_or_else(|| entry.norm.clone());
        by_stem.entry(stem).or_default().push(entry);
    }

    let mut sets = Vec::new();
    for (stem, group) in by_stem {
        let find = |ext: &str| {
            group
                .iter()
                .find(|e| e.extension() == Some(ext))
                .map(|e| e.path.clone())
        };

        let Some(pak) = find("pak") else {
            let short_stem = stem.rsplit('/').next().unwrap_or(&stem);
            warnings.push(format!(
                "`{short_stem}` has IoStore files but no .pak; the game will not mount it."
            ));
            continue;
        };

        // Derive the base name from the on-disk name, not the lowercased
        // matching key, so the author's capitalisation survives deployment.
        let file_name = pak
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let short_stem = file_name
            .rsplit_once('.')
            .map_or(file_name.as_str(), |(s, _)| s)
            .to_string();
        let had_p_suffix = short_stem.to_ascii_lowercase().ends_with("_p");
        let base = if had_p_suffix {
            short_stem[..short_stem.len() - 2].to_string()
        } else {
            short_stem.clone()
        };

        let set = PakSet {
            base,
            pak,
            utoc: find("utoc"),
            ucas: find("ucas"),
            had_p_suffix,
        };
        if !set.is_complete() {
            warnings.push(format!(
                "`{short_stem}` is missing one of its .utoc/.ucas pair and may fail to load."
            ));
        }
        if !had_p_suffix {
            warnings.push(format!(
                "`{short_stem}` has no `_P` suffix; it will be renamed so the game mounts it."
            ));
        }
        sets.push(set);
    }

    (sets, warnings)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Route every file into a flat destination directory, discarding structure.
fn map_flat(entries: &[TreeEntry], target_dir: &str) -> Vec<ComponentFile> {
    entries
        .iter()
        .map(|e| {
            let name = e
                .path
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| e.path.clone());
            ComponentFile {
                source: e.path.clone(),
                target: Path::new(target_dir).join(name),
            }
        })
        .collect()
}

/// Route files under a destination directory, keeping their structure below
/// `strip_depth` leading components.
fn map_preserving(entries: &[TreeEntry], target_dir: &str, strip_depth: usize) -> Vec<ComponentFile> {
    entries
        .iter()
        .map(|e| {
            let rel = strip_leading(&e.path, strip_depth).unwrap_or_else(|| {
                PathBuf::from(e.path.file_name().unwrap_or(e.path.as_os_str()))
            });
            ComponentFile {
                source: e.path.clone(),
                target: Path::new(target_dir).join(rel),
            }
        })
        .collect()
}

fn mark_claimed(entries: &[TreeEntry], claimed: &mut HashSet<String>) {
    for e in entries {
        claimed.insert(e.norm.clone());
    }
}

fn is_junk(entry: &TreeEntry) -> bool {
    if entry.segments().contains(&"__macosx") {
        return true;
    }
    if JUNK_NAMES.contains(&entry.file_name()) {
        return true;
    }
    entry
        .extension()
        .is_some_and(|ext| JUNK_EXTS.contains(&ext))
}

fn strip_leading(path: &Path, count: usize) -> Option<PathBuf> {
    let comps: Vec<_> = path
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    if comps.len() <= count {
        return None;
    }
    Some(comps[count..].iter().collect())
}

fn nth_component(path: &Path, index: usize) -> Option<String> {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .nth(index)
}
