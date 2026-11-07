//! Audit records (env vault spec §10): one record per resolution attempt and per fetch
//! outcome of an org, as one line of canonical JSON. A record never holds a value. Each org's
//! records on this install are numbered by `seq` and chained by `prev`, the SHA-256 of the
//! line before, so a receiver sees a gap or a changed record. The format is
//! `docs/vault-audit-format.md`.

pub mod record;
pub mod sender;
pub mod verify;
pub mod writer;

pub use record::{
    canonical, line_hash, stamp, AuditDot, AuditRecord, AuditUser, Dropped, Event, RECORD_VERSION,
};
pub use sender::{AuditSender, AuditTransport, HttpTransport, SendState};
pub use verify::{verify_lines, ChainReport, ChainSummary, Finding};
pub use writer::{Audit, Happened, OrgAudit, Usage};
