use std::path::{Path, PathBuf};

use sbmm_core::plan::DeployPlan;

use crate::manifest::Manifest;

#[derive(Debug, thiserror::Error)]
pub enum DeployError {
    #[error("i/o error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("source file is missing from staging: {0}")]
    MissingSource(PathBuf),
    #[error("refusing to write outside the game folder: {0}")]
    EscapesGameRoot(PathBuf),
    #[error("{0}")]
    Other(String),
}

impl DeployError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        DeployError::Io {
            path: path.into(),
            source,
        }
    }
}

/// A file in the game folder that no longer matches what we deployed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub target: PathBuf,
    pub kind: DriftKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftKind {
    /// We deployed it; something removed it.
    Missing,
    /// Still there, but its size no longer matches the staged file.
    Modified,
}

/// Where the deployment engine is allowed to operate.
#[derive(Debug, Clone)]
pub struct DeployContext {
    /// The directory containing `SB/`.
    pub game_root: PathBuf,
    /// Where displaced original game files are parked.
    pub backup_root: PathBuf,
}

/// A strategy for getting staged files into the game folder.
///
/// The hardlink backend ships first; a ProjFS backend implements the same
/// contract later so that neither the domain nor the UI has to change.
pub trait DeployBackend {
    fn deploy(&self, ctx: &DeployContext, plan: &DeployPlan) -> Result<Manifest, DeployError>;
    fn undeploy(&self, ctx: &DeployContext, manifest: &Manifest) -> Result<(), DeployError>;
    fn verify(&self, ctx: &DeployContext, manifest: &Manifest) -> Result<Vec<Drift>, DeployError>;
    fn name(&self) -> &'static str;
}

/// Reject targets that would escape the game folder via `..` or an absolute
/// path. Archives are untrusted input, so this is enforced before any write.
pub fn resolve_target(game_root: &Path, relative: &Path) -> Result<PathBuf, DeployError> {
    if relative.is_absolute() {
        return Err(DeployError::EscapesGameRoot(relative.to_path_buf()));
    }
    let mut out = game_root.to_path_buf();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => return Err(DeployError::EscapesGameRoot(relative.to_path_buf())),
        }
    }
    if !out.starts_with(game_root) {
        return Err(DeployError::EscapesGameRoot(relative.to_path_buf()));
    }
    Ok(out)
}
