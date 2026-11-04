//! Snooze and Done on the Inbox (snooze and done spec section 4): the key that names each
//! Inbox item, the mark a person puts on it, and the SQL that leaves hidden items out of the
//! counts.

use serde::{Deserialize, Serialize};

str_enum!(InboxKind {
    Message => "message",
    Approval => "approval",
    Review => "review",
    NotDelivered => "not_delivered",
    Attention => "attention",
    Watcher => "watcher",
    Held => "held",
    Tools => "tools",
    Instructions => "instructions",
    AppReview => "app_review",
    TeamScope => "team_scope",
});

/// One Inbox item: its kind and an id that names the item as it is now (spec section 4.1), so
/// a mark never hides a later item with the same subject.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InboxKey {
    pub kind: InboxKind,
    pub id: String,
}

impl InboxKey {
    pub fn new(kind: InboxKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

/// A mark on an Inbox item, in exactly one state: snoozed until a time, done, or back from
/// snooze. `title` and `dot_id` are worked out from the item when the Inbox is read (the store
/// leaves them empty); `dot_id` is `None` for `tools`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxMark {
    pub kind: InboxKind,
    pub id: String,
    pub title: String,
    pub dot_id: Option<String>,
    pub snoozed_until: Option<String>,
    pub done_at: Option<String>,
    pub back_at: Option<String>,
}

impl InboxMark {
    pub fn key(&self) -> InboxKey {
        InboxKey::new(self.kind, self.id.clone())
    }
}

/// The state a mark is deleted in by Unsnooze (`Snoozed`) or Undo (`Done`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkState {
    Snoozed,
    Done,
}

/// The condition that the item of `kind` whose id is the SQL expression `id_expr` is not
/// hidden: no mark of it is snoozed or done. A back mark does not hide.
pub fn hidden_sql(kind: InboxKind, id_expr: &str) -> String {
    format!(
        "NOT EXISTS (SELECT 1 FROM inbox_marks k WHERE k.kind = '{}' AND k.item_id = {id_expr} \
         AND (k.snoozed_until IS NOT NULL OR k.done_at IS NOT NULL))",
        kind.as_str()
    )
}
