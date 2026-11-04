//! Snooze and Done on the Inbox (snooze and done spec section 4): the key that names each
//! Inbox item, the mark a person puts on it, and the SQL that leaves hidden items out of the
//! counts.

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};

use crate::model::{Inbox, Message, MessageKind};

/// How often the wake pass looks for snoozes that came due (spec section 3.9.8).
pub const WAKE_EVERY: TimeDelta = TimeDelta::seconds(30);
/// How long a done item stays in the Done list (spec section 3.9.4).
pub const DONE_LISTED_FOR: TimeDelta = TimeDelta::days(7);
/// How far ahead a snooze may reach (spec section 3.9.10).
pub const SNOOZE_REACH: TimeDelta = TimeDelta::days(365);
/// A title's length at most, in characters.
const TITLE_MAX: usize = 80;

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

/// One item of the Inbox: its key, its dot (`None` for `tools`) and its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxEntry {
    pub key: InboxKey,
    pub dot_id: Option<String>,
    pub title: String,
}

/// The first line of `text`, cut to 80 characters with `…`.
pub fn title_cut(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim_end();
    if line.chars().count() <= TITLE_MAX {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(TITLE_MAX - 1).collect();
    cut.push('…');
    cut
}

/// Every item of `inbox` and the waiting ones of `messages`, with their keys and titles (spec
/// section 4.1), in the order of the sections: messages, approvals, not delivered, runs to
/// review, schedules that need attention, tools changed, paused watchers, held runs,
/// instructions, apps to review, team scope problems. The UI's `inboxKeys` gives the same.
pub fn entries(inbox: &Inbox, messages: &[Message]) -> Vec<InboxEntry> {
    let mut out = Vec::new();
    let mut add = |kind: InboxKind, id: String, dot_id: Option<&str>, title: String| {
        out.push(InboxEntry {
            key: InboxKey::new(kind, id),
            dot_id: dot_id.map(str::to_string),
            title: title_cut(&title),
        });
    };
    for m in messages.iter().filter(|m| m.waits()) {
        let verb = match m.kind {
            MessageKind::Question => "asks",
            MessageKind::Note => "says",
        };
        let title = format!("{} {verb}: {}", m.dot_name, m.text);
        add(InboxKind::Message, m.id.clone(), Some(&m.dot_id), title);
    }
    for a in &inbox.approvals {
        let title = format!("{} asks to use {}", a.dot_name, a.approval.tool);
        add(
            InboxKind::Approval,
            a.approval.id.clone(),
            Some(&a.dot_id),
            title,
        );
    }
    for u in &inbox.not_delivered {
        let title = format!("{} didn't get your message", u.dot_name);
        add(
            InboxKind::NotDelivered,
            u.run.id.clone(),
            Some(&u.run.dot_id),
            title,
        );
    }
    for r in &inbox.review_runs {
        let title = format!("{}: run to review", r.dot_name);
        add(
            InboxKind::Review,
            r.run.id.clone(),
            Some(&r.run.dot_id),
            title,
        );
    }
    for a in &inbox.attention {
        let title = format!("{}'s schedule needs attention", a.dot_name);
        add(
            InboxKind::Attention,
            a.run_id.clone(),
            Some(&a.dot_id),
            title,
        );
    }
    for t in &inbox.tools_changed {
        let title = format!("{} tools changed", t.name);
        add(
            InboxKind::Tools,
            format!("{}/{}", t.server_id, t.scope),
            None,
            title,
        );
    }
    for w in &inbox.watchers {
        let title = format!("{}'s watcher {} paused", w.dot_name, w.name);
        let id = format!("{}@{}", w.watcher_id, w.at);
        add(InboxKind::Watcher, id, Some(&w.dot_id), title);
    }
    for h in &inbox.held {
        let title = format!("{}: run waiting to run", h.dot_name);
        add(InboxKind::Held, h.run_id.clone(), Some(&h.dot_id), title);
    }
    for i in &inbox.instructions {
        let title = format!("{} proposed new instructions", i.dot_name);
        add(
            InboxKind::Instructions,
            i.proposal.id.clone(),
            Some(&i.proposal.dot_id),
            title,
        );
    }
    for r in &inbox.app_reviews {
        let title = format!(
            "{} sent {} for review",
            r.dot_name, r.proposal.summary.title
        );
        add(
            InboxKind::AppReview,
            r.proposal.id.clone(),
            Some(&r.dot_id),
            title,
        );
    }
    for t in &inbox.team_scope {
        let title = format!("{} in team {} needs attention", t.dot_name, t.team_name);
        let id = format!("{}/{}", t.team_id, t.dot_id);
        add(InboxKind::TeamScope, id, Some(&t.dot_id), title);
    }
    out
}
