//! Accounts: the Dots ID of this install, and (later) the members and workspaces built on it
//! (accounts design §5).

mod identity;
mod scope;

pub use identity::{
    ensure_identity, identity, set_profile, Identity, IDENTITY_SECRET, MAX_EMAIL, MAX_PROFILE_NAME,
};
pub use scope::{current_scope, set_current_scope, CurrentScope, DEFAULT_WORKSPACE, PERSONAL};
