//! Orgs (env vault spec §6): entries an admin's service holds for a team, under a policy signed
//! with the org's minisign key, joined with a join code.

pub mod join;
pub mod policy;
pub mod row;
pub mod signed;

pub use join::{
    check_org_id, key_fingerprint, parse_join_code, BaoAuth, InfisicalSettings, JoinCode,
    OpenBaoSettings, SopsSettings, SourceSettings,
};
pub use policy::{parse_policy, AuditTarget, OrgPolicy, PersonalMode, PolicyError, UseRules};
pub use row::OrgRow;
pub use signed::{envelope, verify_policy};
