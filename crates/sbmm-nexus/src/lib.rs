//! Talking to Nexus Mods.
//!
//! The API host is not reachable from CI, so everything here is built around
//! traits that tests fill with canned responses: the REST client over
//! [`client::Transport`], and file downloads over [`download::Fetcher`].
//!
//! Nexus gates direct download links behind Premium. That gate is respected,
//! not worked around — the free path is the `nxm://` link the website's "Mod
//! Manager Download" button produces, which is what [`nxm`] parses.

pub mod client;
pub mod collection;
pub mod http;
pub mod nxm;
pub mod ratelimit;

pub use client::{Account, ModFile, ModInfo, NexusClient, NexusError, Transport, UpdatedMod};
pub use collection::{Collection, CollectionMod, ManualMod};
pub use http::{Fetcher, Progress, ReqwestTransport};
pub use nxm::{NxmCollection, NxmError, NxmFile, NxmLink};
pub use ratelimit::RateLimit;
