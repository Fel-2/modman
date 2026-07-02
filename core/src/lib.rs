//! modeman-core: Linux-first game mod manager engine.
//!
//! Platform/UI-agnostic. Owns game detection, mod storage, profiles,
//! load order, and deployment into the live game directory.

pub mod archive;
pub mod conflict;
pub mod deploy;
pub mod error;
pub mod fomod;
pub mod game;
pub mod launchers;
pub mod loadorder;
pub mod manager;
pub mod paradoxdb;
pub mod plugins;
pub mod profile;
pub mod redmod;
pub mod store;
pub mod vdf;
pub mod vfs;

pub use conflict::FileConflict;
pub use error::{Error, Result};
pub use fomod::{FomodConfig, FomodSession, Selections};
pub use game::{GameSpec, InstalledGame, CATALOG};
pub use manager::Manager;
pub use profile::{ModEntry, Profile};
pub use store::{ModRecord, NexusRef};
