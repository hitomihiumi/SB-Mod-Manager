//! The overlay as written to disk for the hook to read.
//!
//! The manager and the game are two processes. Rather than share memory, the
//! manager writes the whole description to one file before launching, and the
//! hook reads it once at start-up — the same split USVFS makes between Mod
//! Organizer and the game. The overwrite folder is not listed: it is scanned
//! when the file is loaded, so it is always current.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::overlay::{Layer, Overlay};
use crate::{overwrite_layer, Vfs};

/// Bumped whenever the shape changes, so a hook from another build refuses a
/// mapping it would misread instead of serving the wrong files.
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum MappingError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the mapping file is damaged: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("the mapping was written by format {found}, this build reads {FORMAT_VERSION}")]
    Version { found: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub version: u32,
    pub game_root: PathBuf,
    pub overwrite_root: PathBuf,
    /// Lowest first.
    pub layers: Vec<Layer>,
}

impl Mapping {
    pub fn new(
        game_root: impl Into<PathBuf>,
        overwrite_root: impl Into<PathBuf>,
        layers: Vec<Layer>,
    ) -> Mapping {
        Mapping {
            version: FORMAT_VERSION,
            game_root: game_root.into(),
            overwrite_root: overwrite_root.into(),
            layers,
        }
    }

    /// Write atomically: the game may be starting while the manager rewrites
    /// the file, and a half-written mapping would be read as damaged.
    pub fn save(&self, path: &Path) -> Result<(), MappingError> {
        let io = |p: &Path| {
            let p = p.to_path_buf();
            move |e| MappingError::Io { path: p, source: e }
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(self)?).map_err(io(&temporary))?;
        std::fs::rename(&temporary, path).map_err(io(path))?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Mapping, MappingError> {
        let bytes = std::fs::read(path).map_err(|e| MappingError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        let mapping: Mapping = serde_json::from_slice(&bytes)?;
        if mapping.version != FORMAT_VERSION {
            return Err(MappingError::Version {
                found: mapping.version,
            });
        }
        Ok(mapping)
    }

    /// Build the live view: every layer, then whatever is in overwrite now.
    pub fn into_vfs(self) -> Vfs {
        let overwrite = overwrite_layer(&self.overwrite_root);
        let overlay = Overlay::build(self.layers.into_iter().chain(std::iter::once(overwrite)));
        Vfs::new(overlay, self.game_root, self.overwrite_root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{LayerFile, Origin};
    use crate::{Access, Redirect, VPath};

    #[test]
    fn a_mapping_survives_the_trip_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let overwrite = dir.path().join("overwrite");
        std::fs::create_dir_all(overwrite.join("SB")).unwrap();
        std::fs::write(overwrite.join("SB/created.txt"), "x").unwrap();

        let mapping = Mapping::new(
            dir.path().join("game"),
            &overwrite,
            vec![Layer {
                origin: Origin::Mod(3),
                name: "Outfit".into(),
                files: vec![LayerFile {
                    target: "SB/Content/Paks/~mods/X_P.pak".into(),
                    source: dir.path().join("mods/x.pak"),
                }],
            }],
        );
        let path = dir.path().join("vfs/mapping.json");
        mapping.save(&path).unwrap();
        let loaded = Mapping::load(&path).unwrap();
        assert_eq!(loaded, mapping);

        let vfs = loaded.into_vfs();
        assert_eq!(
            vfs.redirect(
                &VPath::from_relative("SB/created.txt").unwrap(),
                Access::Read,
                false
            ),
            Redirect::To(overwrite.join("SB/created.txt")),
            "overwrite is read when the mapping is loaded"
        );
        assert_eq!(vfs.overlay().len(), 2);
    }

    #[test]
    fn a_mapping_from_another_format_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mapping.json");
        let mut mapping = Mapping::new("/game", "/overwrite", Vec::new());
        mapping.version = FORMAT_VERSION + 1;
        mapping.save(&path).unwrap();

        assert!(matches!(
            Mapping::load(&path),
            Err(MappingError::Version { .. })
        ));
    }
}
