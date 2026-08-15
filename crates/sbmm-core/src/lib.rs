//! Domain layer for SB Mod Manager.
//!
//! This crate is deliberately free of I/O and platform APIs so the interesting
//! logic — figuring out what a downloaded archive actually *is*, and where its
//! files need to end up — can be unit tested anywhere.

pub mod detect;
pub mod loadorder;
pub mod model;
pub mod paths;
pub mod plan;
pub mod tree;

pub use detect::detect_components;
pub use model::{Confidence, DetectedComponent, ModType, PakSet};
pub use plan::{DeployPlan, PlannedFile};
pub use tree::FileTree;
