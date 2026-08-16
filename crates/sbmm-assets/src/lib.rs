//! Reading what a mod actually replaces.
//!
//! Two mods can both look installed and both be enabled while only one of them
//! is having any effect, because they replace the same asset and the game
//! loads whichever mounts last. Nothing on the outside of the files says so —
//! the only way to know is to read the index and compare.
//!
//! Stellar Blade is UE 4.26 with IoStore, so a pak mod is a `.pak`/`.utoc`
//! /`.ucas` set. Almost all of the content is in the `.ucas`, indexed by the
//! `.utoc`; the `.pak` beside it is usually near-empty. Both index formats are
//! read here, and only the index — the `.ucas` is never opened, so a scan
//! costs a few kilobytes of reads no matter how large the mod is.
//!
//! Nothing is decompressed and nothing is decrypted. This answers "which
//! assets are in here", not "what are they".

pub mod pak;
pub mod utoc;

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not an IoStore container")]
    NotAToc(std::path::PathBuf),
    #[error("{0} is not an Unreal pak")]
    NotAPak(std::path::PathBuf),
    #[error("{path} is malformed: {reason}")]
    Malformed {
        path: std::path::PathBuf,
        reason: String,
    },
    /// The container's index is encrypted, which needs a game-specific AES key
    /// the manager does not have and has no business guessing at.
    #[error("{0} has an encrypted index, so its contents cannot be listed")]
    Encrypted(std::path::PathBuf),
    #[error("{0} uses an index format older than this reader supports")]
    UnsupportedVersion(std::path::PathBuf),
}

impl AssetError {
    pub(crate) fn io(path: impl Into<std::path::PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn malformed(
        path: impl Into<std::path::PathBuf>,
        reason: impl Into<String>,
    ) -> Self {
        Self::Malformed {
            path: path.into(),
            reason: reason.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, AssetError>;

/// One thing a mod provides, in a form two mods can be compared on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum Asset {
    /// An asset path, lowercased with forward slashes.
    ///
    /// Case is dropped because Unreal resolves these case-insensitively, so
    /// `Eve/Body.uasset` and `eve/body.uasset` are the same asset and two mods
    /// shipping them under different casing do conflict.
    Path(String),
    /// A chunk with no name.
    ///
    /// Containers can be built without a directory index. The chunk id is
    /// still a stable identity, so a clash is still detectable — the UI just
    /// cannot say what the file is called.
    Chunk(String),
}

impl Asset {
    pub fn path(value: impl AsRef<str>) -> Asset {
        Asset::Path(normalise(value.as_ref()))
    }

    /// How it reads in a list.
    pub fn label(&self) -> &str {
        match self {
            Asset::Path(path) => path,
            Asset::Chunk(id) => id,
        }
    }

    pub fn is_named(&self) -> bool {
        matches!(self, Asset::Path(_))
    }
}

/// Lowercase, forward slashes, no leading separator.
pub fn normalise(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches('/')
        .to_ascii_lowercase()
}

/// Everything one container provides.
///
/// Dispatches on the extension rather than sniffing, because the two formats
/// live side by side under the same base name and the caller already knows
/// which file it is holding.
pub fn read(path: &Path) -> Result<BTreeSet<Asset>> {
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("utoc") => utoc::read(path),
        Some("pak") => pak::read(path),
        _ => Ok(BTreeSet::new()),
    }
}

/// Everything a whole mod provides, across every container it installs.
///
/// A failure on one container does not lose the others: a mod with one
/// unreadable pak still reports what the rest of it replaces, which is more
/// useful than reporting nothing. What could not be read comes back separately
/// so the UI can say the answer is incomplete rather than implying it is not.
pub fn read_all<'a>(
    paths: impl IntoIterator<Item = &'a Path>,
) -> (BTreeSet<Asset>, Vec<AssetError>) {
    let mut assets = BTreeSet::new();
    let mut problems = Vec::new();

    for path in paths {
        match read(path) {
            Ok(found) => assets.extend(found),
            Err(error) => problems.push(error),
        }
    }
    (assets, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_are_compared_without_case_or_separator_differences() {
        assert_eq!(
            Asset::path("SB\\Content\\Eve\\Body.uasset"),
            Asset::path("sb/content/eve/body.uasset")
        );
        assert_eq!(
            Asset::path("/SB/Content/Thing.uasset"),
            Asset::path("SB/Content/Thing.uasset"),
            "a leading separator is a mount-point artefact, not part of the name"
        );
    }

    #[test]
    fn a_file_that_is_neither_format_yields_nothing_rather_than_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let stray = dir.path().join("readme.txt");
        std::fs::write(&stray, b"hello").unwrap();
        assert!(read(&stray).unwrap().is_empty());
    }

    #[test]
    fn one_unreadable_container_does_not_lose_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("broken.utoc");
        std::fs::write(&broken, b"not a toc at all").unwrap();
        let ignored = dir.path().join("notes.txt");
        std::fs::write(&ignored, b"hello").unwrap();

        let (assets, problems) = read_all([broken.as_path(), ignored.as_path()]);
        assert!(assets.is_empty());
        assert_eq!(
            problems.len(),
            1,
            "the unreadable one is reported, not hidden"
        );
    }
}
