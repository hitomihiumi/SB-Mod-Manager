//! Finding the Stellar Blade install and describing its layout.

pub mod discover;
pub mod vdf;

use std::path::{Path, PathBuf};

/// Steam application id for Stellar Blade.
pub const STEAM_APP_ID: &str = "3489700";

/// Nexus Mods game domain, used when building API and `nxm://` URLs.
pub const NEXUS_DOMAIN: &str = "stellarblade";

pub use discover::{discover, validate, DiscoveryError};

/// How an install was found, shown in the setup wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallSource {
    Steam,
    Epic,
    Manual,
}

/// A validated Stellar Blade installation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInstall {
    /// The directory that contains `SB/`.
    pub root: PathBuf,
    pub source: InstallSource,
    /// Name of the shipping executable, e.g. `SB-Win64-Shipping.exe`.
    pub executable: Option<String>,
}

impl GameInstall {
    pub fn paks_dir(&self) -> PathBuf {
        self.root.join("SB/Content/Paks")
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.root.join(sbmm_paths::PAKS_MODS)
    }

    pub fn logic_mods_dir(&self) -> PathBuf {
        self.root.join(sbmm_paths::PAKS_LOGICMODS)
    }

    pub fn binaries_dir(&self) -> PathBuf {
        self.root.join(sbmm_paths::BINARIES_WIN64)
    }

    pub fn executable_path(&self) -> Option<PathBuf> {
        self.executable
            .as_ref()
            .map(|exe| self.binaries_dir().join(exe))
    }
}

/// Paths mirrored from `sbmm-core` so this crate stays dependency-light.
mod sbmm_paths {
    pub const PAKS_MODS: &str = "SB/Content/Paks/~mods";
    pub const PAKS_LOGICMODS: &str = "SB/Content/Paks/LogicMods";
    pub const BINARIES_WIN64: &str = "SB/Binaries/Win64";
}

/// Does this directory look like a Stellar Blade install?
pub fn looks_like_game_root(path: &Path) -> bool {
    path.join("SB/Content/Paks").is_dir()
}

/// Find `SB/Binaries/Win64/*-Win64-Shipping.exe`.
///
/// Matched by pattern rather than a hard-coded name so a renamed or re-badged
/// build still validates.
pub fn find_shipping_exe(root: &Path) -> Option<String> {
    let binaries = root.join(sbmm_paths::BINARIES_WIN64);
    let entries = std::fs::read_dir(binaries).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("-win64-shipping.exe") {
            return Some(name);
        }
    }
    None
}
