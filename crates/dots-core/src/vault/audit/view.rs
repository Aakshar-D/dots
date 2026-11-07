//! What an account's page shows of its audit records (env vault spec §10): the History list
//! and, on Overview, how sending stands. It never holds a value.

use serde::Serialize;

use super::sender::SendState;
use crate::store::HistoryRow;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditView {
    /// The endpoint the account's policy names; `None` when it sends no records.
    pub endpoint: Option<String>,
    /// How many records wait to be sent.
    pub queued: u64,
    pub send: SendState,
    /// This PC's records for the account, newest first: 30 days, at most 1,000, and no
    /// `fetched`.
    pub history: Vec<HistoryRow>,
}

impl AuditView {
    /// An account without an endpoint sends nothing, so it shows no send state (the sender's
    /// may still hold a failure from before its policy dropped `audit`).
    pub fn new(
        endpoint: Option<String>,
        queued: u64,
        send: SendState,
        history: Vec<HistoryRow>,
    ) -> AuditView {
        let send = if endpoint.is_some() {
            send
        } else {
            SendState::default()
        };
        AuditView {
            endpoint,
            queued,
            send,
            history,
        }
    }
}
