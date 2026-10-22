//! The env vault (env vault spec): the variables and secrets dots are granted, where they are
//! kept, and how a run gets them.

pub mod secrets;

#[cfg(windows)]
pub use secrets::WindowsCredentials;
pub use secrets::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
