//! Updating the game's upscaler DLLs.
//!
//! Stellar Blade ships DLSS and FSR as ordinary DLLs next to the executable,
//! and both vendors keep a stable ABI within a line, so a newer DLL of the
//! same line can be dropped in without the game knowing. That is the whole
//! trick — there is no patching involved, only file replacement, which is why
//! it can be undone exactly.
//!
//! Two constraints shape everything here:
//!
//! - **The line matters more than the number.** FSR 3.1 and FSR 4 do not share
//!   a DLL name or an ABI, so a game on 3.1 can only be moved along the 3.1
//!   line. Offering the newest release regardless would produce a file the
//!   game silently ignores.
//! - **The hardware matters.** DLSS Frame Generation needs an RTX 40 or newer;
//!   installing it on a 3060 puts a DLL on disk that can never load.
//!
//! Everything except the actual adapter enumeration is platform-free, so the
//! catalogue, the version parsing and the capability rules are tested on any
//! machine.

pub mod catalog;
pub mod gpu;
pub mod pe;
pub mod scan;

use serde::{Deserialize, Serialize};

pub use catalog::{catalog, Release, Source};
pub use gpu::{Adapter, Capability, Vendor};
pub use scan::{scan, InstalledFile};

#[derive(Debug, thiserror::Error)]
pub enum UpscalerError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0} is not a Windows DLL")]
    NotAPe(std::path::PathBuf),
    #[error("{0} carries no version information")]
    NoVersion(std::path::PathBuf),
}

impl UpscalerError {
    pub(crate) fn io(path: impl Into<std::path::PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

pub type Result<T> = std::result::Result<T, UpscalerError>;

/// A replaceable upscaler DLL.
///
/// Named after what the file does rather than the marketing name, because the
/// three DLSS DLLs are independent: a machine can take a newer Super
/// Resolution without being able to run Frame Generation at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    /// `nvngx_dlss.dll` — DLSS Super Resolution.
    DlssSuperResolution,
    /// `nvngx_dlssg.dll` — DLSS Frame Generation.
    DlssFrameGeneration,
    /// `nvngx_dlssd.dll` — DLSS Ray Reconstruction.
    DlssRayReconstruction,
    /// `amd_fidelityfx_dx12.dll` — FSR on Direct3D 12.
    FsrDx12,
    /// `amd_fidelityfx_vk.dll` — FSR on Vulkan.
    FsrVulkan,
}

impl Component {
    /// The file name the game loads. Matching is case-insensitive because
    /// what is on disk depends on whoever packaged the game.
    pub fn file_name(self) -> &'static str {
        match self {
            Component::DlssSuperResolution => "nvngx_dlss.dll",
            Component::DlssFrameGeneration => "nvngx_dlssg.dll",
            Component::DlssRayReconstruction => "nvngx_dlssd.dll",
            Component::FsrDx12 => "amd_fidelityfx_dx12.dll",
            Component::FsrVulkan => "amd_fidelityfx_vk.dll",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Component::DlssSuperResolution => "DLSS Super Resolution",
            Component::DlssFrameGeneration => "DLSS Frame Generation",
            Component::DlssRayReconstruction => "DLSS Ray Reconstruction",
            Component::FsrDx12 => "FSR (DirectX 12)",
            Component::FsrVulkan => "FSR (Vulkan)",
        }
    }

    pub fn family(self) -> Family {
        match self {
            Component::DlssSuperResolution
            | Component::DlssFrameGeneration
            | Component::DlssRayReconstruction => Family::Dlss,
            Component::FsrDx12 | Component::FsrVulkan => Family::Fsr,
        }
    }

    /// What the hardware has to be able to do for this file to be worth having.
    pub fn requires(self) -> Capability {
        match self {
            Component::DlssSuperResolution | Component::DlssRayReconstruction => {
                Capability::DlssSuperResolution
            }
            Component::DlssFrameGeneration => Capability::DlssFrameGeneration,
            // FSR is vendor-agnostic and runs on anything that runs the game.
            Component::FsrDx12 | Component::FsrVulkan => Capability::Universal,
        }
    }

    pub fn all() -> &'static [Component] {
        &[
            Component::DlssSuperResolution,
            Component::DlssFrameGeneration,
            Component::DlssRayReconstruction,
            Component::FsrDx12,
            Component::FsrVulkan,
        ]
    }

    pub fn from_file_name(name: &str) -> Option<Component> {
        let lower = name.to_ascii_lowercase();
        Component::all()
            .iter()
            .copied()
            .find(|c| c.file_name() == lower)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Family {
    Dlss,
    Fsr,
}

/// A four-part Windows file version, compared numerically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version(pub u16, pub u16, pub u16, pub u16);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The fourth part is a build counter nobody recognises, so it only
        // shows when it carries information.
        if self.3 == 0 {
            write!(f, "{}.{}.{}", self.0, self.1, self.2)
        } else {
            write!(f, "{}.{}.{}.{}", self.0, self.1, self.2, self.3)
        }
    }
}

impl Version {
    /// Parse a `1.2.3` or `v1.2.3.4` style string.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let mut parts = text.split('.').map(|p| p.parse::<u16>());
        let major = parts.next()?.ok()?;
        let minor = parts.next().transpose().ok()?.unwrap_or(0);
        let patch = parts.next().transpose().ok()?.unwrap_or(0);
        let build = parts.next().transpose().ok()?.unwrap_or(0);
        if parts.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch, build))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_are_recognised_from_their_file_name_whatever_the_case() {
        assert_eq!(
            Component::from_file_name("nvngx_dlss.dll"),
            Some(Component::DlssSuperResolution)
        );
        assert_eq!(
            Component::from_file_name("NVNGX_DLSSG.DLL"),
            Some(Component::DlssFrameGeneration)
        );
        assert_eq!(
            Component::from_file_name("amd_fidelityfx_dx12.dll"),
            Some(Component::FsrDx12)
        );
        // Similar names that are not upscalers must not be touched.
        assert_eq!(Component::from_file_name("nvngx.dll"), None);
        assert_eq!(Component::from_file_name("amd_ags_x64.dll"), None);
    }

    #[test]
    fn versions_compare_numerically_not_as_text() {
        let older = Version::parse("3.7.10").unwrap();
        let newer = Version::parse("310.7.0").unwrap();
        assert!(
            newer > older,
            "310 is later than 3.7, despite sorting first"
        );
        assert!(Version::parse("v1.1.4").unwrap() > Version::parse("1.1.3").unwrap());
        assert_eq!(Version::parse("2").unwrap(), Version(2, 0, 0, 0));
    }

    #[test]
    fn version_display_hides_an_empty_build_number() {
        assert_eq!(Version(310, 7, 0, 0).to_string(), "310.7.0");
        assert_eq!(Version(1, 0, 1, 41314).to_string(), "1.0.1.41314");
    }

    #[test]
    fn rubbish_versions_are_rejected_rather_than_guessed_at() {
        assert_eq!(Version::parse(""), None);
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse("1.2.3.4.5"), None);
        assert_eq!(Version::parse("3.1.x"), None);
    }
}
