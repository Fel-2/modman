//! Nexus Mods integration: `nxm://` link parsing, REST client, and file
//! downloads. Network-facing; kept out of `modeman-core` so the engine stays
//! dependency-light and offline-testable.
//!
//! Auth uses a personal API key (Settings → API on nexusmods.com) sent in the
//! `apikey` header. Free accounts can download via `nxm://` links (the "Mod
//! Manager Download" button); premium accounts can also fetch direct links.

mod client;
mod error;
mod nxm;
mod protocol;

pub use client::{DownloadLink, ModFile, ModInfo, ModList, NexusClient, User};
pub use error::{Error, Result};
pub use nxm::NxmLink;
pub use protocol::{desktop_entry, install_protocol_handler};
