//! Accounts: the Dots ID of this install, and (later) the members and workspaces built on it
//! (accounts design §5).

pub mod edit;
mod identity;
mod membership;
mod permissions;
mod request;
pub mod roster;
mod scope;
mod workspaces;

pub use identity::{
    ensure_identity, identity, set_profile, sign_with_identity, Identity, IDENTITY_SECRET,
    MAX_EMAIL, MAX_PROFILE_NAME,
};
pub use membership::{Membership, NotMember};
pub use permissions::{
    access, access_dot, access_in, can, require, require_dot, require_in, require_team,
    workspace_name, Access, Permission,
};
pub use request::{make_request, parse_request, AccessRequest, RequestError, REQUEST_PREFIX};
pub use roster::{
    open_accepted_roster, parse_roster, roster_envelope, roster_hash, verify_roster, Accepted,
    Member, Role, Roster, RosterError, Workspace, WorkspaceRole, ROSTER_ENVELOPE_VERSION,
    ROSTER_VERSION,
};
pub use scope::{current_scope, set_current_scope, CurrentScope, DEFAULT_WORKSPACE, PERSONAL};
pub use workspaces::{visible_workspaces, workspace_id_for, WorkspaceView};
