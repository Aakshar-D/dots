//! Accounts: the Dots ID of this install, and (later) the members and workspaces built on it
//! (accounts design §5).

mod identity;

pub use identity::{
    ensure_identity, identity, set_profile, Identity, IDENTITY_SECRET, MAX_EMAIL, MAX_PROFILE_NAME,
};
