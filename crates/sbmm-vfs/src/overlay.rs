//! The virtual view of the game folder.
//!
//! This is the same idea as Mod Organizer 2's virtual file system: every
//! enabled mod is a *layer*, layers are stacked in load order, and the game
//! sees the union — the real game folder at the bottom, each mod over it, and
//! the overwrite folder on top. Where two layers provide the same path the
//! higher one wins and the lower ones are remembered as *shadowed*, which is
//! what a conflicts view is built from.
//!
//! Only the overlay is held here. The real game folder is not copied into the
//! tree: a path the overlay does not know is passed through to the disk, and a
//! directory listing is the real listing merged with the overlay's entries.
//! That keeps the tree as small as the mods are, however large the game is.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::path::{key, VPath};

/// Who provides a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "id")]
pub enum Origin {
    /// An installed mod, by its id in the manager's database.
    Mod(i64),
    /// A file the manager writes for the game, like UE4SS's `mods.txt`.
    Generated,
    /// Something the game itself created while running under the overlay.
    Overwrite,
}

/// One layer: a set of files and where in the game folder they appear.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub origin: Origin,
    /// Shown in conflict lists.
    pub name: String,
    pub files: Vec<LayerFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerFile {
    /// Relative to the game root, as the game will see it.
    pub target: PathBuf,
    /// The real file on disk, absolute.
    pub source: PathBuf,
}

/// A file the overlay provides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualFile {
    /// The name as the winning layer spells it.
    pub name: String,
    pub source: PathBuf,
    pub origin: Origin,
    /// Lower layers that also provide this path, highest first.
    pub shadowed: Vec<Origin>,
}

/// One entry of a merged directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub is_dir: bool,
    /// Where the bytes really are, for a file the overlay provides. `None` for
    /// the game's own entries and for directories.
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Child {
    name: String,
    is_dir: bool,
}

/// A path the overlay could not place, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub origin: Origin,
    pub target: PathBuf,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default)]
pub struct Overlay {
    files: BTreeMap<VPath, VirtualFile>,
    /// Every directory the overlay creates or adds to, with its children.
    /// Directories that exist only because a mod put a file there are real
    /// enough to the game: it lists and opens them like any other.
    dirs: BTreeMap<VPath, BTreeMap<String, Child>>,
    rejected: Vec<Rejected>,
}

impl Overlay {
    /// Stack `layers`, lowest first: a later layer wins every path it shares
    /// with an earlier one.
    pub fn build(layers: impl IntoIterator<Item = Layer>) -> Overlay {
        let mut overlay = Overlay::default();
        overlay.dirs.insert(VPath::root(), BTreeMap::new());
        for layer in layers {
            for file in &layer.files {
                overlay.add(layer.origin, file);
            }
        }
        overlay
    }

    /// Put one more file on top, for a file the game creates while running.
    pub fn add(&mut self, origin: Origin, file: &LayerFile) {
        let Some(path) = VPath::from_relative(&file.target) else {
            self.rejected.push(Rejected {
                origin,
                target: file.target.clone(),
                reason: "the path leaves the game folder",
            });
            return;
        };
        if path.is_root() {
            return;
        }
        // A file and a folder cannot share a name. The folder is kept, because
        // dropping it would hide everything underneath, and the file is
        // reported rather than lost without a word.
        if self.dirs.contains_key(&path) {
            self.rejected.push(Rejected {
                origin,
                target: file.target.clone(),
                reason: "another mod has a folder of that name",
            });
            return;
        }
        if path.ancestors().iter().any(|a| self.files.contains_key(a)) {
            self.rejected.push(Rejected {
                origin,
                target: file.target.clone(),
                reason: "another mod has a file where this needs a folder",
            });
            return;
        }

        let names: Vec<String> = path.components().map(str::to_string).collect();
        self.link_parents(&path, &names);

        let name = names.last().cloned().unwrap_or_default();
        let shadowed = match self.files.get(&path) {
            Some(previous) => {
                let mut shadowed = vec![previous.origin];
                shadowed.extend(previous.shadowed.iter().copied());
                shadowed.retain(|o| *o != origin);
                shadowed
            }
            None => Vec::new(),
        };
        self.files.insert(
            path,
            VirtualFile {
                name,
                source: file.source.clone(),
                origin,
                shadowed,
            },
        );
    }

    /// Add a directory with nothing in it yet, for one the game creates.
    pub fn add_dir(&mut self, path: &VPath) {
        if path.is_root()
            || self.files.contains_key(path)
            || path.ancestors().iter().any(|a| self.files.contains_key(a))
        {
            return;
        }
        let mut dir = VPath::root();
        for name in path.components() {
            self.dirs
                .entry(dir.clone())
                .or_default()
                .entry(key(name))
                .or_insert(Child {
                    name: name.to_string(),
                    is_dir: true,
                });
            dir = dir.join(name);
            self.dirs.entry(dir.clone()).or_default();
        }
    }

    /// Take a file out of the view, for one the game moved or deleted.
    ///
    /// Whatever it shadowed stays hidden until the next launch rebuilds the
    /// view: the game asked for the path to be gone, and bringing a lower
    /// mod's copy back mid-session would be a surprise.
    pub fn remove(&mut self, path: &VPath) -> Option<VirtualFile> {
        let removed = self.files.remove(path)?;
        if let Some(parent) = path.parent() {
            if let Some(children) = self.dirs.get_mut(&parent) {
                children.remove(&key(path.file_name()));
            }
        }
        Some(removed)
    }

    /// Record every directory between the root and `path`, so each one lists
    /// the next.
    fn link_parents(&mut self, path: &VPath, names: &[String]) {
        let ancestors = path.ancestors();
        for (depth, dir) in ancestors.iter().enumerate() {
            let child_name = names.get(depth).cloned().unwrap_or_default();
            let is_dir = depth + 1 < ancestors.len();
            let children = self.dirs.entry(dir.clone()).or_default();
            children
                .entry(key(&child_name))
                .or_insert(Child {
                    name: child_name,
                    is_dir,
                })
                // A name first seen as a file's never becomes a folder here:
                // `add` refuses that case before linking.
                .is_dir |= is_dir;
            if is_dir {
                let next = dir.join(&names[depth]);
                self.dirs.entry(next).or_default();
            }
        }
    }

    /// The file the overlay provides at `path`, if it provides one.
    pub fn file(&self, path: &VPath) -> Option<&VirtualFile> {
        self.files.get(path)
    }

    /// Whether the overlay has a directory at `path` — one a mod created, or a
    /// real one it adds files to.
    pub fn is_dir(&self, path: &VPath) -> bool {
        self.dirs.contains_key(path)
    }

    /// Merge the real listing of `dir` with what the overlay adds there.
    ///
    /// A name the overlay provides replaces the real entry, so the listing
    /// never shows the same file twice and a pak a mod replaces is listed once,
    /// pointing at the mod's copy. Order is by name, case-insensitively, which
    /// is what NTFS returns and what Unreal's pak mount order relies on.
    pub fn list(&self, dir: &VPath, real: impl IntoIterator<Item = (String, bool)>) -> Vec<Listed> {
        let mut merged: BTreeMap<String, Listed> = BTreeMap::new();
        for (name, is_dir) in real {
            merged.insert(
                key(&name),
                Listed {
                    name,
                    is_dir,
                    source: None,
                },
            );
        }
        if let Some(children) = self.dirs.get(dir) {
            for (child_key, child) in children {
                let source = (!child.is_dir)
                    .then(|| self.files.get(&dir.join(child_key)))
                    .flatten()
                    .map(|f| f.source.clone());
                let entry = merged.entry(child_key.clone()).or_insert(Listed {
                    name: child.name.clone(),
                    is_dir: child.is_dir,
                    source: None,
                });
                // A real folder the overlay adds to stays the real folder.
                if !(entry.is_dir && child.is_dir) {
                    entry.name = child.name.clone();
                    entry.is_dir = child.is_dir;
                    entry.source = source;
                }
            }
        }
        merged.into_values().collect()
    }

    /// Every path more than one layer provides.
    pub fn conflicts(&self) -> impl Iterator<Item = (&VPath, &VirtualFile)> {
        self.files.iter().filter(|(_, f)| !f.shadowed.is_empty())
    }

    pub fn files(&self) -> impl Iterator<Item = (&VPath, &VirtualFile)> {
        self.files.iter()
    }

    /// Files that could not be placed.
    pub fn rejected(&self) -> &[Rejected] {
        &self.rejected
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(origin: Origin, files: &[(&str, &str)]) -> Layer {
        Layer {
            origin,
            name: format!("{origin:?}"),
            files: files
                .iter()
                .map(|(target, source)| LayerFile {
                    target: PathBuf::from(target),
                    source: PathBuf::from(source),
                })
                .collect(),
        }
    }

    fn vpath(text: &str) -> VPath {
        VPath::from_relative(text).unwrap()
    }

    #[test]
    fn the_higher_layer_wins_and_the_lower_one_is_remembered() {
        let overlay = Overlay::build([
            layer(
                Origin::Mod(1),
                &[("SB/Content/Paks/~mods/Outfit_P.pak", "/lib/a/x.pak")],
            ),
            layer(
                Origin::Mod(2),
                &[("sb/content/paks/~MODS/outfit_p.pak", "/lib/b/x.pak")],
            ),
        ]);

        let file = overlay
            .file(&vpath("SB/Content/Paks/~mods/Outfit_P.pak"))
            .unwrap();
        assert_eq!(file.origin, Origin::Mod(2));
        assert_eq!(file.source, PathBuf::from("/lib/b/x.pak"));
        assert_eq!(file.shadowed, vec![Origin::Mod(1)]);
        assert_eq!(file.name, "outfit_p.pak", "the winner's spelling is shown");
        assert_eq!(overlay.conflicts().count(), 1);
    }

    #[test]
    fn three_layers_keep_the_whole_stack_of_losers() {
        let overlay = Overlay::build([
            layer(Origin::Mod(1), &[("a.txt", "/1")]),
            layer(Origin::Mod(2), &[("a.txt", "/2")]),
            layer(Origin::Overwrite, &[("a.txt", "/o")]),
        ]);
        let file = overlay.file(&vpath("a.txt")).unwrap();
        assert_eq!(file.origin, Origin::Overwrite);
        assert_eq!(file.shadowed, vec![Origin::Mod(2), Origin::Mod(1)]);
    }

    #[test]
    fn folders_a_mod_creates_are_listed_from_the_root_down() {
        let overlay = Overlay::build([layer(
            Origin::Mod(1),
            &[(
                "SB/Binaries/Win64/ue4ss/Mods/Skip/scripts/main.lua",
                "/lib/main.lua",
            )],
        )]);

        assert!(overlay.is_dir(&vpath("SB/Binaries/Win64/ue4ss/Mods/Skip")));
        let listing = overlay.list(&vpath("SB/Binaries/Win64/ue4ss/Mods"), []);
        assert_eq!(
            listing,
            vec![Listed {
                name: "Skip".into(),
                is_dir: true,
                source: None
            }]
        );
    }

    #[test]
    fn a_listing_merges_the_real_folder_with_the_overlay() {
        let overlay = Overlay::build([layer(
            Origin::Mod(1),
            &[
                ("SB/Content/Paks/pakchunk0.pak", "/lib/replacement.pak"),
                ("SB/Content/Paks/~mods/0010_X_P.pak", "/lib/x.pak"),
            ],
        )]);

        let listing = overlay.list(
            &vpath("SB/Content/Paks"),
            [
                ("PakChunk0.pak".to_string(), false),
                ("~mods".to_string(), true),
                ("global.ucas".to_string(), false),
            ],
        );
        let names: Vec<(&str, bool, bool)> = listing
            .iter()
            .map(|l| (l.name.as_str(), l.is_dir, l.source.is_some()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("global.ucas", false, false),
                ("pakchunk0.pak", false, true),
                ("~mods", true, false),
            ],
            "the replaced pak is listed once, from the mod; the real folder stays real"
        );
    }

    #[test]
    fn a_file_where_another_mod_has_a_folder_is_reported_not_placed() {
        let overlay = Overlay::build([
            layer(Origin::Mod(1), &[("SB/Thing/inner.txt", "/1")]),
            layer(
                Origin::Mod(2),
                &[("SB/Thing", "/2"), ("SB/Thing/inner.txt/deeper", "/3")],
            ),
        ]);
        assert!(overlay.is_dir(&vpath("SB/Thing")));
        assert_eq!(
            overlay.file(&vpath("SB/Thing/inner.txt")).unwrap().origin,
            Origin::Mod(1)
        );
        assert_eq!(overlay.rejected().len(), 2);
    }

    #[test]
    fn an_empty_folder_the_game_creates_is_listed() {
        let mut overlay = Overlay::build([]);
        overlay.add_dir(&vpath("SB/Saved/Logs"));
        assert!(overlay.is_dir(&vpath("sb/saved/logs")));
        assert!(overlay.list(&vpath("SB/Saved/Logs"), []).is_empty());
        assert_eq!(overlay.list(&vpath("SB/Saved"), [])[0].name, "Logs");
    }

    #[test]
    fn a_removed_file_is_gone_from_its_folder() {
        let mut overlay = Overlay::build([layer(Origin::Mod(1), &[("SB/a.txt", "/1")])]);
        assert!(overlay.remove(&vpath("sb/A.TXT")).is_some());
        assert!(overlay.file(&vpath("SB/a.txt")).is_none());
        assert!(overlay.list(&vpath("SB"), []).is_empty());
    }

    #[test]
    fn a_path_that_escapes_the_game_is_rejected() {
        let overlay = Overlay::build([layer(Origin::Mod(1), &[("../../evil.dll", "/x")])]);
        assert!(overlay.is_empty());
        assert_eq!(
            overlay.rejected()[0].reason,
            "the path leaves the game folder"
        );
    }
}
