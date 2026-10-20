//! A virtual file system for the game folder, in the manner of Mod Organizer 2.
//!
//! The hard-link backend puts mods *into* the game folder. This crate is the
//! other way round: the game folder is never touched, and the game is started
//! under a hook that answers its file requests from a merged view instead —
//! the real folder at the bottom, every enabled mod stacked over it in load
//! order, and an *overwrite* folder on top that catches whatever the game
//! writes. Mod Organizer 2 does this with USVFS; the moving parts are the same:
//!
//! - [`Overlay`] — the stacked view, which layer wins each path, and what each
//!   winner shadows.
//! - [`Vfs::redirect`] — the policy the hook applies to every file it is asked
//!   to open: read from the winning layer, write new files to overwrite,
//!   change existing ones where they really are.
//! - [`Mapping`] — the view written to disk by the manager and read by the hook
//!   in the game process, so the two never have to share memory.
//!
//! Everything here is platform-free and tested anywhere; only the hook that
//! intercepts the game's calls is Windows code.

pub mod launch;
pub mod mapping;
pub mod overlay;
pub mod path;
pub mod pattern;

use std::path::{Path, PathBuf};

pub use mapping::{Mapping, MappingError};
pub use overlay::{Layer, LayerFile, Listed, Origin, Overlay, Rejected, VirtualFile};
pub use path::{relative_to_root, VPath};

/// What the hook should do with a request for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// Not ours: let the call go to the real path untouched.
    Passthrough,
    /// Use this real file instead.
    To(PathBuf),
    /// A directory that exists only in the overlay. Opening it for listing
    /// has to succeed, but there is no real handle to give, so the hook
    /// answers from [`Vfs::list`].
    VirtualDir,
}

/// How the game wants to open a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reading, or querying attributes.
    Read,
    /// Writing to a file that must already exist.
    WriteExisting,
    /// Writing, creating the file when it is missing.
    Create,
}

/// A loaded overlay bound to a game folder and an overwrite folder.
#[derive(Debug, Clone)]
pub struct Vfs {
    overlay: Overlay,
    game_root: PathBuf,
    overwrite_root: PathBuf,
}

impl Vfs {
    pub fn new(
        overlay: Overlay,
        game_root: impl Into<PathBuf>,
        overwrite_root: impl Into<PathBuf>,
    ) -> Vfs {
        Vfs {
            overlay,
            game_root: game_root.into(),
            overwrite_root: overwrite_root.into(),
        }
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn game_root(&self) -> &Path {
        &self.game_root
    }

    pub fn overwrite_root(&self) -> &Path {
        &self.overwrite_root
    }

    /// Where a request for `path` should really go.
    ///
    /// The rules are Mod Organizer 2's, because they are the ones that keep
    /// both the game folder and the mods honest:
    ///
    /// - A path the overlay provides is served from the winning layer, for
    ///   reading *and* writing: a mod's config edited in game stays with that
    ///   mod.
    /// - A path that is only real is passed through, so the game's own files
    ///   behave exactly as they would without the manager.
    /// - A new file — one that exists nowhere — goes to the overwrite folder,
    ///   so logs, caches and configs the game or a mod creates never land in
    ///   the game folder. The caller records it with [`Vfs::created`].
    pub fn redirect(&self, path: &VPath, access: Access, real_exists: bool) -> Redirect {
        if let Some(file) = self.overlay.file(path) {
            return Redirect::To(file.source.clone());
        }
        if self.overlay.is_dir(path) && !real_exists {
            return Redirect::VirtualDir;
        }
        if real_exists || access != Access::Create {
            return Redirect::Passthrough;
        }
        // A file created where the game folder has no such directory, but a
        // mod does, still has to go somewhere the game can find again.
        Redirect::To(self.overwrite_path(path))
    }

    /// Where a directory the game creates should really be made.
    ///
    /// The same rule as files: a directory that already exists, really or
    /// virtually, needs nothing; a new one goes to overwrite.
    pub fn redirect_mkdir(&self, path: &VPath, real_exists: bool) -> Redirect {
        if real_exists || self.overlay.is_dir(path) {
            Redirect::Passthrough
        } else {
            Redirect::To(self.overwrite_path(path))
        }
    }

    /// Record a file the game just created in the overwrite folder, so later
    /// requests in the same session find it.
    pub fn created(&mut self, path: &VPath, real: PathBuf) {
        self.overlay.add(
            Origin::Overwrite,
            &LayerFile {
                target: PathBuf::from(path.display()),
                source: real,
            },
        );
    }

    /// Forget a file the game moved away or deleted.
    pub fn removed(&mut self, path: &VPath) {
        self.overlay.remove(path);
    }

    /// Record a directory the game just created in the overwrite folder.
    pub fn created_dir(&mut self, path: &VPath) {
        self.overlay.add_dir(path);
    }

    /// The merged listing of `dir`, given the real folder's own entries.
    pub fn list(&self, dir: &VPath, real: impl IntoIterator<Item = (String, bool)>) -> Vec<Listed> {
        self.overlay.list(dir, real)
    }

    /// Place a game-relative path inside the overwrite folder.
    pub fn overwrite_path(&self, path: &VPath) -> PathBuf {
        let mut out = self.overwrite_root.clone();
        for part in path.components() {
            out.push(part);
        }
        out
    }

    /// Place an absolute path the game asked for, if it is inside the game
    /// folder.
    pub fn locate(&self, absolute: &str) -> Option<VPath> {
        relative_to_root(&self.game_root.to_string_lossy(), absolute)
    }
}

/// Every file under `overwrite_root`, as a layer, so what the game created in
/// an earlier session is still there in the next one — as it is in Mod
/// Organizer 2, where overwrite is simply the topmost mod.
pub fn overwrite_layer(overwrite_root: &Path) -> Layer {
    let mut files = Vec::new();
    collect(overwrite_root, overwrite_root, &mut files);
    files.sort_by(|a: &LayerFile, b| a.target.cmp(&b.target));
    Layer {
        origin: Origin::Overwrite,
        name: "Overwrite".into(),
        files,
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<LayerFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect(root, &path, out),
            Ok(kind) if kind.is_file() => {
                if let Ok(relative) = path.strip_prefix(root) {
                    out.push(LayerFile {
                        target: relative.to_path_buf(),
                        source: path.clone(),
                    });
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vfs() -> Vfs {
        let overlay = Overlay::build([Layer {
            origin: Origin::Mod(7),
            name: "Outfit".into(),
            files: vec![
                LayerFile {
                    target: "SB/Content/Paks/~mods/0010_Outfit_P.pak".into(),
                    source: "/lib/mods/Outfit/Outfit_P.pak".into(),
                },
                LayerFile {
                    target: "SB/Binaries/Win64/ue4ss/Mods/Skip/scripts/main.lua".into(),
                    source: "/lib/mods/Skip/main.lua".into(),
                },
            ],
        }]);
        Vfs::new(overlay, "/game", "/lib/overwrite")
    }

    fn vpath(text: &str) -> VPath {
        VPath::from_relative(text).unwrap()
    }

    #[test]
    fn a_mod_file_is_read_from_the_mod() {
        assert_eq!(
            vfs().redirect(
                &vpath("sb/content/paks/~mods/0010_outfit_p.pak"),
                Access::Read,
                false
            ),
            Redirect::To("/lib/mods/Outfit/Outfit_P.pak".into())
        );
    }

    #[test]
    fn the_games_own_files_pass_straight_through() {
        let vfs = vfs();
        let pak = vpath("SB/Content/Paks/pakchunk0-WindowsNoEditor.pak");
        assert_eq!(
            vfs.redirect(&pak, Access::Read, true),
            Redirect::Passthrough
        );
        assert_eq!(
            vfs.redirect(&pak, Access::WriteExisting, true),
            Redirect::Passthrough
        );
        assert_eq!(
            vfs.redirect(&vpath("SB/missing.txt"), Access::Read, false),
            Redirect::Passthrough,
            "a missing file stays missing so the game gets its usual error"
        );
    }

    #[test]
    fn a_new_file_goes_to_overwrite_and_is_found_afterwards() {
        let mut vfs = vfs();
        let log = vpath("SB/Binaries/Win64/ue4ss/UE4SS.log");
        let Redirect::To(real) = vfs.redirect(&log, Access::Create, false) else {
            panic!("a new file must be redirected");
        };
        assert_eq!(
            real,
            PathBuf::from("/lib/overwrite/SB/Binaries/Win64/ue4ss/UE4SS.log"),
            "the game's own spelling is kept"
        );

        vfs.created(&log, real.clone());
        assert_eq!(vfs.redirect(&log, Access::Read, false), Redirect::To(real));
    }

    #[test]
    fn a_folder_only_a_mod_has_is_virtual() {
        let vfs = vfs();
        assert_eq!(
            vfs.redirect(
                &vpath("SB/Binaries/Win64/ue4ss/Mods/Skip"),
                Access::Read,
                false
            ),
            Redirect::VirtualDir
        );
        assert_eq!(
            vfs.redirect_mkdir(&vpath("SB/Binaries/Win64/ue4ss/Mods/Skip"), false),
            Redirect::Passthrough,
            "creating a folder that is already there virtually is a no-op"
        );
        assert!(matches!(
            vfs.redirect_mkdir(&vpath("SB/Saved/Logs"), false),
            Redirect::To(_)
        ));
    }

    #[test]
    fn absolute_paths_are_located_inside_the_game_only() {
        let vfs = Vfs::new(Overlay::default(), r"C:\Games\SB", r"C:\Lib\overwrite");
        assert_eq!(
            vfs.locate(r"C:\games\sb\SB\Content").unwrap().as_str(),
            "sb/content"
        );
        assert_eq!(vfs.locate(r"C:\Windows\System32\kernel32.dll"), None);
    }

    #[test]
    fn what_the_game_created_last_time_is_still_there() {
        let dir = tempfile::tempdir().unwrap();
        let overwrite = dir.path();
        std::fs::create_dir_all(overwrite.join("SB/Saved")).unwrap();
        std::fs::write(overwrite.join("SB/Saved/settings.ini"), "x").unwrap();

        let layer = overwrite_layer(overwrite);
        assert_eq!(layer.origin, Origin::Overwrite);
        assert_eq!(layer.files.len(), 1);
        assert_eq!(
            layer.files[0].target,
            PathBuf::from("SB/Saved/settings.ini")
        );
    }
}
