//! Orgs (env vault spec §6): entries an admin's service holds for a team, under a policy signed
//! with the org's minisign key, joined with a join code.

pub mod join;
pub mod offline;
pub mod policy;
pub mod row;
pub mod service;
pub mod signed;
pub mod sources;

pub use join::{
    check_org_id, key_fingerprint, parse_join_code, BaoAuth, InfisicalSettings, JoinCode,
    OpenBaoSettings, SopsSettings, SourceSettings,
};
#[cfg(windows)]
pub use offline::Dpapi;
pub use offline::{
    delete_offline, offline_path, os_protector, read_offline, write_offline, NoProtector,
    OfflineCopy, Protector, TestProtector, OFFLINE_DIR,
};
pub use policy::{parse_policy, AuditTarget, OrgPolicy, PersonalMode, PolicyError, UseRules};
pub use row::OrgRow;
pub use service::{
    JoinPreview, OrgEntry, OrgLookup, OrgStatus, OrgSummary, OrgView, Orgs, SourceFactory,
    RETRY_AFTER,
};
pub use signed::{envelope, verify_policy};
pub use sources::{
    http_client, http_client_with_timeout, source_for, Fetched, Infisical, OpenBao, Sops, Source,
    SourceCreds, SourceError, POLICY_ENTRY,
};
