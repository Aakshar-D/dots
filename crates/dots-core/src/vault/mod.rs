//! The env vault (env vault spec): the variables and secrets dots are granted, where they are
//! kept, and how a run gets them.

pub mod child_env;
pub mod entries;
pub mod grants;
pub mod literals;
pub mod redact;
pub mod resolve;
pub mod secrets;
pub mod service;
pub mod upgrade;

pub use entries::{EntryKind, EnvEntry};
pub use grants::{Consumer, Grants};
pub use resolve::{Missing, Resolved, Unresolved};
#[cfg(windows)]
pub use secrets::WindowsCredentials;
pub use secrets::{os_secrets, MemorySecrets, SecretStore, MAX_SECRET_BYTES};
pub use service::Vault;
