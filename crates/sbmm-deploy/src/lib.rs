//! Materialising mods into the game folder — reversibly.
//!
//! The guiding rule is that the game directory must always be returnable to
//! exactly the state it was in before the manager touched it. Every file we
//! create is recorded, every file we displace is backed up, and `undeploy`
//! only ever removes paths it can find in its own manifest. Files it does not
//! recognise are left alone, even if they sit in a mod folder.

pub mod backend;
pub mod hardlink;
pub mod manifest;
pub mod ue4ss;

pub use backend::{DeployBackend, DeployError, Drift, DriftKind};
pub use hardlink::HardlinkBackend;
pub use manifest::{DeployedFile, Manifest};
