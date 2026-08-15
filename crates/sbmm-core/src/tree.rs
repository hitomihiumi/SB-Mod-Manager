use std::path::{Path, PathBuf};

use crate::paths;

/// One file inside an extracted archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// Where the file actually lives, relative to the extraction root. This is
    /// what gets read during deployment, so wrapper stripping never touches it.
    pub path: PathBuf,
    /// The path after redundant wrapper folders have been removed, with the
    /// original casing kept. Destinations are computed from this.
    pub rel: PathBuf,
    /// Lowercased, forward-slash form of `rel`, used for all matching. Mod
    /// archives are wildly inconsistent about casing, so nothing compares
    /// against the raw paths directly.
    pub norm: String,
}

impl TreeEntry {
    pub fn segments(&self) -> Vec<&str> {
        self.norm.split('/').filter(|s| !s.is_empty()).collect()
    }

    pub fn file_name(&self) -> &str {
        self.norm.rsplit('/').next().unwrap_or(&self.norm)
    }

    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        name.rsplit_once('.').map(|(_, ext)| ext)
    }
}

/// A normalized view of every file in an extracted archive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTree {
    entries: Vec<TreeEntry>,
}

impl FileTree {
    /// Build a tree from relative file paths. Directories are implied by the
    /// files inside them and need not be listed.
    pub fn new<I, P>(paths: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut entries: Vec<TreeEntry> = paths
            .into_iter()
            .filter_map(|p| {
                let path = p.as_ref().to_path_buf();
                let norm = normalize(&path);
                if norm.is_empty() {
                    None
                } else {
                    Some(TreeEntry {
                        rel: path.clone(),
                        path,
                        norm,
                    })
                }
            })
            .collect();
        entries.sort_by(|a, b| a.norm.cmp(&b.norm));
        entries.dedup_by(|a, b| a.norm == b.norm);
        Self { entries }
    }

    /// Build a tree from entries that already carry both path forms.
    ///
    /// Used to split a tree without losing the distinction between where a
    /// file lives and where wrapper stripping decided it logically sits.
    pub fn from_entries(entries: Vec<TreeEntry>) -> Self {
        let mut entries = entries;
        entries.sort_by(|a, b| a.norm.cmp(&b.norm));
        entries.dedup_by(|a, b| a.norm == b.norm);
        Self { entries }
    }

    pub fn entries(&self) -> &[TreeEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drop redundant top-level folders such as `MyMod v1.2/`.
    ///
    /// A wrapper is only stripped when it is the sole top-level entry and its
    /// name is not itself a routing signal — `SB/`, `LogicMods/` and friends
    /// are load-bearing and must survive.
    pub fn stripped(&self) -> FileTree {
        let mut current = self.entries.clone();

        loop {
            if current.is_empty() {
                break;
            }
            // Every entry must be nested at least one level deep.
            if current.iter().any(|e| !e.norm.contains('/')) {
                break;
            }
            let first = match current[0].norm.split('/').next() {
                Some(s) => s.to_string(),
                None => break,
            };
            if paths::MEANINGFUL_DIRS.contains(&first.as_str()) {
                break;
            }
            // A lone top-level folder that directly holds a UE4SS marker *is*
            // the mod folder — its name is the identity UE4SS registers, so it
            // must not be mistaken for packaging cruft.
            let is_ue4ss_mod_folder = current.iter().any(|e| {
                e.norm == format!("{first}/scripts/main.lua")
                    || e.norm == format!("{first}/dlls/main.dll")
            });
            if is_ue4ss_mod_folder {
                break;
            }
            let all_share = current
                .iter()
                .all(|e| e.norm.split('/').next() == Some(first.as_str()));
            if !all_share {
                break;
            }

            current = current
                .iter()
                .filter_map(|e| {
                    let rest_norm = e.norm.split_once('/').map(|(_, r)| r.to_string())?;
                    let rest_rel = strip_first_component(&e.rel)?;
                    Some(TreeEntry {
                        path: e.path.clone(),
                        rel: rest_rel,
                        norm: rest_norm,
                    })
                })
                .collect();
        }

        FileTree { entries: current }
    }

    /// True when any path contains the given directory name as a full segment.
    pub fn has_segment(&self, segment: &str) -> bool {
        let seg = segment.to_ascii_lowercase();
        self.entries
            .iter()
            .any(|e| e.segments().contains(&seg.as_str()))
    }

    /// Entries whose normalized path contains `segment` as a full component.
    pub fn entries_under_segment(&self, segment: &str) -> Vec<&TreeEntry> {
        let seg = segment.to_ascii_lowercase();
        self.entries
            .iter()
            .filter(|e| e.segments().contains(&seg.as_str()))
            .collect()
    }

    /// Find directories `D` such that `D/<relative>` exists in the tree.
    ///
    /// Used to spot UE4SS mod folders via their `scripts/main.lua` or
    /// `dlls/main.dll` marker. Returns normalized directory paths; an empty
    /// string means the marker sits at the tree root.
    pub fn dirs_containing(&self, relative: &str) -> Vec<String> {
        let needle = relative.to_ascii_lowercase();
        let suffix = format!("/{needle}");
        let mut out: Vec<String> = self
            .entries
            .iter()
            .filter_map(|e| {
                if e.norm == needle {
                    Some(String::new())
                } else {
                    e.norm.strip_suffix(&suffix).map(|dir| dir.to_string())
                }
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Entries that live directly at the root of the tree.
    pub fn top_level_files(&self) -> Vec<&TreeEntry> {
        self.entries
            .iter()
            .filter(|e| !e.norm.contains('/'))
            .collect()
    }

    /// Entries sitting inside the given normalized directory (at any depth).
    pub fn entries_in_dir(&self, dir: &str) -> Vec<&TreeEntry> {
        if dir.is_empty() {
            return self.entries.iter().collect();
        }
        let prefix = format!("{}/", dir.to_ascii_lowercase());
        self.entries
            .iter()
            .filter(|e| e.norm.starts_with(&prefix))
            .collect()
    }

    /// A subtree rooted at `dir`, with the prefix removed from every path.
    pub fn subtree(&self, dir: &str) -> FileTree {
        if dir.is_empty() {
            return self.clone();
        }
        let prefix = format!("{}/", dir.to_ascii_lowercase());
        let depth = prefix.matches('/').count();
        let entries = self
            .entries
            .iter()
            .filter(|e| e.norm.starts_with(&prefix))
            .filter_map(|e| {
                let norm = e.norm[prefix.len()..].to_string();
                let rel = strip_components(&e.rel, depth)?;
                Some(TreeEntry {
                    path: e.path.clone(),
                    rel,
                    norm,
                })
            })
            .collect();
        FileTree { entries }
    }
}

fn normalize(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn strip_first_component(path: &Path) -> Option<PathBuf> {
    strip_components(path, 1)
}

fn strip_components(path: &Path, count: usize) -> Option<PathBuf> {
    let mut comps: Vec<_> = path
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    if comps.len() <= count {
        return None;
    }
    comps.drain(..count);
    Some(comps.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_redundant_wrapper_directory() {
        let tree = FileTree::new(["Cool Mod v1.2/foo_P.pak", "Cool Mod v1.2/foo_P.utoc"]);
        let stripped = tree.stripped();
        let names: Vec<_> = stripped.entries().iter().map(|e| e.norm.as_str()).collect();
        assert_eq!(names, vec!["foo_p.pak", "foo_p.utoc"]);
        assert_eq!(
            stripped.entries()[0].path,
            PathBuf::from("Cool Mod v1.2/foo_P.pak"),
            "stripping is a matching concern; the file is still where it was"
        );
    }

    #[test]
    fn strips_nested_wrappers_but_keeps_meaningful_dirs() {
        let tree = FileTree::new(["Release/Build/LogicMods/thing.pak"]);
        let stripped = tree.stripped();
        assert_eq!(stripped.entries()[0].norm, "logicmods/thing.pak");
    }

    #[test]
    fn never_strips_the_sb_game_root() {
        let tree = FileTree::new(["SB/Content/Paks/~mods/a_P.pak"]);
        let stripped = tree.stripped();
        assert_eq!(stripped.entries()[0].norm, "sb/content/paks/~mods/a_p.pak");
    }

    #[test]
    fn keeps_wrapper_when_root_also_has_loose_files() {
        let tree = FileTree::new(["Wrapper/a.pak", "readme.txt"]);
        let stripped = tree.stripped();
        assert_eq!(stripped.entries().len(), 2);
        assert!(stripped.entries().iter().any(|e| e.norm == "wrapper/a.pak"));
    }

    #[test]
    fn finds_marker_directories() {
        let tree = FileTree::new(["MyLuaMod/Scripts/main.lua", "MyLuaMod/Scripts/util.lua"]);
        assert_eq!(tree.dirs_containing("scripts/main.lua"), vec!["myluamod"]);
    }

    #[test]
    fn subtree_rebases_paths() {
        let tree = FileTree::new(["SB/Content/Movies/intro.mp4"]);
        let sub = tree.subtree("sb");
        assert_eq!(sub.entries()[0].norm, "content/movies/intro.mp4");
        assert_eq!(
            sub.entries()[0].rel,
            PathBuf::from("Content/Movies/intro.mp4")
        );
        assert_eq!(
            sub.entries()[0].path,
            PathBuf::from("SB/Content/Movies/intro.mp4"),
            "the on-disk path must survive rebasing"
        );
    }
}
