//! The env vault (env vault spec): the variables and secrets dots are granted, where they are
//! kept, and how a run gets them.

pub mod entries;
pub mod secrets;
pub mod service;

pub use entries::{EntryKind, EnvEntry};
#[cfg(windows)]
pub use secrets::WindowsCredentials;
pub use secrets::{os_secrets, MemorySecrets, SecretStore, MAX_SECRET_BYTES};
pub use service::Vault;
