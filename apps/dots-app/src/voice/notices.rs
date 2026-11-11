//! Spoken notices: Dot says one short line aloud when something needs the person (Open Dot
//! design spec, step 5). Off until "Tell me aloud when something needs me" (`voice_notices`)
//! is on, and only while the voice service is ready and the session rests.
//!
//! A line is built here from the dot's name and the kind of thing only: never a tool, an
//! input, a path or a question's words. A private approval names nothing. At most one line
//! every `NOTICE_GAP`; what arrives meanwhile is counted, from the Inbox, once the gap has
//! passed and the session rests. Nothing spoken is logged, only that a notice was and its
//! kind.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dots_core::events::RuntimeEvent;
use dots_core::model::MessageKind;
use dots_core::Runtime;
use tokio::sync::broadcast::error::RecvError;

use super::service::Service;

/// The least time between two notices.
pub const NOTICE_GAP: Duration = Duration::from_secs(30);

/// The longest a dot's name may be when spoken, in bytes.
const NAME_BYTES: usize = 80;
/// Said for a dot whose name is empty or unknown.
const A_DOT: &str = "A dot";
const PRIVATE: &str = "One private approval needs you.";
const ONE_THING: &str = "One thing needs you.";

/// Something that needs the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needs {
    Approval { dot_name: String, private: bool },
    Question { dot_name: String },
}

impl Needs {
    fn private(&self) -> bool {
        matches!(self, Needs::Approval { private: true, .. })
    }

    /// The line for this one thing alone.
    fn line(&self) -> String {
        match self {
            Needs::Approval { private: true, .. } => PRIVATE.to_string(),
            Needs::Approval { dot_name, .. } => {
                format!("{} needs your approval.", spoken_name(dot_name))
            }
            Needs::Question { dot_name } => {
                format!("{} has a question for you.", spoken_name(dot_name))
            }
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Needs::Approval { private: true, .. } => "private approval",
            Needs::Approval { .. } => "approval",
            Needs::Question { .. } => "question",
        }
    }
}

/// A dot's name for speech: on one line, without control or hidden characters, at most 80
/// bytes (cut on a character boundary); "A dot" when nothing is left.
pub fn spoken_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len().min(NAME_BYTES));
    let mut gap = false;
    for c in name.chars() {
        if c.is_whitespace() || c.is_control() {
            gap = !out.is_empty();
            continue;
        }
        if hidden(c) {
            continue;
        }
        let space = usize::from(gap);
        if out.len() + space + c.len_utf8() > NAME_BYTES {
            break;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(c);
    }
    if out.is_empty() {
        A_DOT.to_string()
    } else {
        out
    }
}

/// Characters that show nothing (format and direction marks, joiners, tags, the soft hyphen).
fn hidden(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

/// "Two" to "Twenty"; digits past that.
fn count_word(n: usize) -> String {
    const WORDS: [&str; 19] = [
        "Two",
        "Three",
        "Four",
        "Five",
        "Six",
        "Seven",
        "Eight",
        "Nine",
        "Ten",
        "Eleven",
        "Twelve",
        "Thirteen",
        "Fourteen",
        "Fifteen",
        "Sixteen",
        "Seventeen",
        "Eighteen",
        "Nineteen",
        "Twenty",
    ];
    n.checked_sub(2)
        .and_then(|i| WORDS.get(i))
        .map_or_else(|| n.to_string(), |w| (*w).to_string())
}

/// What has waited since the last notice, and when that was.
#[derive(Debug, Default)]
pub struct Notices {
    last_spoken: Option<Instant>,
    waiting: Vec<Needs>,
    /// Events were missed (the stream lagged): something may wait that is not in `waiting`.
    missed: bool,
    /// What the last line spoke of, for the log.
    kind: &'static str,
}

impl Notices {
    pub fn new() -> Self {
        Self::default()
    }

    /// Something new needs the person. Returns the sentence to speak now, if any. Only a
    /// thing that is alone in waiting is said here; with more waiting, `rested` counts them
    /// from the Inbox once the session rests and the gap has passed.
    pub fn arrived(&mut self, need: Needs, resting: bool, now: Instant) -> Option<String> {
        self.waiting.push(need);
        if !resting || !self.may_speak(now) || self.missed {
            return None;
        }
        let [only] = self.waiting.as_slice() else {
            return None;
        };
        let (line, kind) = (only.line(), only.kind());
        self.spoke(now, kind);
        Some(line)
    }

    /// The session came to rest. Returns the sentence to speak now, if something is still
    /// waiting. `still_waiting` counts everything in the Inbox, `private_waiting` the private
    /// approvals among it.
    pub fn rested(
        &mut self,
        still_waiting: usize,
        private_waiting: usize,
        now: Instant,
    ) -> Option<String> {
        if !self.pending() || !self.may_speak(now) {
            return None;
        }
        let private = private_waiting.min(still_waiting);
        let (line, kind) = match still_waiting {
            // Answered meanwhile: nothing to say.
            0 => {
                self.clear();
                return None;
            }
            1 if private == 1 => (PRIVATE.to_string(), "private approval"),
            1 => match self.waiting.as_slice() {
                [only] if !self.missed && !only.private() => (only.line(), only.kind()),
                _ => (ONE_THING.to_string(), "one thing"),
            },
            n => (format!("{} things need you.", count_word(n)), "several"),
        };
        self.spoke(now, kind);
        Some(line)
    }

    /// Events were missed: what waits is recounted from the Inbox at the next rest.
    pub fn missed(&mut self) {
        self.missed = true;
    }

    /// Notices turned off, or voice off or not ready: nothing waits.
    pub fn clear(&mut self) {
        self.waiting.clear();
        self.missed = false;
    }

    /// Something waits to be said.
    pub fn pending(&self) -> bool {
        !self.waiting.is_empty() || self.missed
    }

    /// How long until what waits may be said: `None` when nothing waits, zero when now.
    pub fn wait_for(&self, now: Instant) -> Option<Duration> {
        self.pending().then(|| {
            self.last_spoken.map_or(Duration::ZERO, |at| {
                (at + NOTICE_GAP).saturating_duration_since(now)
            })
        })
    }

    /// What the last line spoke of: "approval", "question", "private approval", "one thing"
    /// or "several". For the log, which never carries the line.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    fn may_speak(&self, now: Instant) -> bool {
        self.last_spoken
            .is_none_or(|at| now.saturating_duration_since(at) >= NOTICE_GAP)
    }

    fn spoke(&mut self, now: Instant, kind: &'static str) {
        self.last_spoken = Some(now);
        self.kind = kind;
        self.clear();
    }
}

// ---- the listener ---------------------------------------------------------------------------

/// What a runtime event may ask of the person: an approval, or a question a dot posted. A
/// note does not need the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    Approval { approval_id: String, run_id: String },
    Question { dot_id: String, dot_name: String },
}

pub fn arrival(ev: &RuntimeEvent) -> Option<Arrival> {
    match ev {
        RuntimeEvent::ApprovalRequested { approval } => Some(Arrival::Approval {
            approval_id: approval.id.clone(),
            run_id: approval.run_id.clone(),
        }),
        RuntimeEvent::MessagePosted { message } if message.kind == MessageKind::Question => {
            Some(Arrival::Question {
                dot_id: message.dot_id.clone(),
                dot_name: message.dot_name.clone(),
            })
        }
        _ => None,
    }
}

/// `need`, unless it is about the dot whose conversation the person is reading (the rule
/// desktop notifications follow). A dot that is not known is announced.
pub fn announce(dot_id: Option<&str>, viewing: Option<&str>, need: Needs) -> Option<Needs> {
    (!crate::events::quiet(dot_id, viewing)).then_some(need)
}

/// What needs the person, found from the runtime. A private approval's dot name is not even
/// read: nothing about it is said.
async fn resolve(rt: &Runtime, arrival: Arrival) -> Option<Needs> {
    match arrival {
        Arrival::Approval {
            approval_id,
            run_id,
        } => {
            // An error counts as private.
            let private = rt
                .approval_touches_private(&approval_id)
                .await
                .unwrap_or(true);
            let dot_id = rt.get_run(&run_id).await.map(|r| r.dot_id).ok();
            if crate::events::quiet(dot_id.as_deref(), rt.viewing().as_deref()) {
                return None;
            }
            let dot_name = match (&dot_id, private) {
                (Some(id), false) => rt
                    .get_dot(id)
                    .await
                    .map(|d| d.spec.name)
                    .unwrap_or_default(),
                _ => String::new(),
            };
            Some(Needs::Approval { dot_name, private })
        }
        Arrival::Question { dot_id, dot_name } => announce(
            Some(&dot_id),
            rt.viewing().as_deref(),
            Needs::Question { dot_name },
        ),
    }
}

/// Listens to the runtime for what needs the person and hands it to the voice service, which
/// says it when notices are on, voice is ready and the session rests. While notices are off or
/// voice is not ready nothing is looked up and nothing waits.
pub fn spawn(rt: Arc<Runtime>, service: Arc<Service>) {
    let mut rx = rt.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let Some(arrival) = arrival(&ev) else {
                        continue;
                    };
                    if !service.notices_wanted() {
                        service.clear_notices();
                        continue;
                    }
                    if let Some(need) = resolve(&rt, arrival).await {
                        service.need(need).await;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("voice: the notices listener skipped {n} events");
                    service.missed_notices().await;
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approval(name: &str) -> Needs {
        Needs::Approval {
            dot_name: name.into(),
            private: false,
        }
    }

    fn private(name: &str) -> Needs {
        Needs::Approval {
            dot_name: name.into(),
            private: true,
        }
    }

    fn question(name: &str) -> Needs {
        Needs::Question {
            dot_name: name.into(),
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn one_approval_and_one_question_are_said_with_the_dots_name() {
        let t = Instant::now();
        let mut n = Notices::new();
        assert_eq!(
            n.arrived(approval("Research"), true, t).as_deref(),
            Some("Research needs your approval.")
        );
        let mut n = Notices::new();
        assert_eq!(
            n.arrived(question("Research"), true, t).as_deref(),
            Some("Research has a question for you.")
        );
    }

    #[test]
    fn a_burst_of_ten_approvals_gives_one_notice_then_silence_then_one_count() {
        let t = Instant::now();
        let mut n = Notices::new();
        let mut said = Vec::new();
        for i in 0..10u64 {
            let at = t + Duration::from_millis(i * 100);
            said.extend(n.arrived(approval("Research"), true, at));
        }
        assert_eq!(said, ["Research needs your approval."]);
        // The first notice was spoken and the session rests again: still within the gap.
        for at in [secs(3), secs(10), secs(29)] {
            assert_eq!(n.rested(10, 0, t + at), None, "{at:?}");
        }
        // Thirty seconds on: one line that counts everything waiting, from the Inbox.
        assert_eq!(
            n.rested(10, 0, t + secs(30)).as_deref(),
            Some("Ten things need you.")
        );
        // Nothing new since: silence.
        assert_eq!(n.rested(10, 0, t + secs(90)), None);
    }

    #[test]
    fn a_private_approval_names_nothing() {
        let t = Instant::now();
        let mut n = Notices::new();
        let line = n.arrived(private("Secret Payroll"), true, t).unwrap();
        assert_eq!(line, "One private approval needs you.");
        assert!(!line.contains("Secret") && !line.contains("Payroll"));
        // Counted later, alone: the same line.
        let mut n = Notices::new();
        assert_eq!(n.arrived(private("Secret Payroll"), false, t), None);
        let line = n.rested(1, 1, t).unwrap();
        assert_eq!(line, "One private approval needs you.");
        // With others, it is only counted.
        let mut n = Notices::new();
        n.arrived(private("Secret Payroll"), false, t);
        n.arrived(approval("Research"), false, t);
        assert_eq!(n.rested(2, 1, t).as_deref(), Some("Two things need you."));
    }

    #[test]
    fn what_arrives_mid_turn_is_said_once_when_the_session_next_rests() {
        let t = Instant::now();
        let mut n = Notices::new();
        // The person is speaking, or Dot is mid-turn.
        assert_eq!(n.arrived(question("Research"), false, t), None);
        // At rest, still waiting: said once.
        assert_eq!(
            n.rested(1, 0, t + secs(5)).as_deref(),
            Some("Research has a question for you.")
        );
        assert_eq!(n.rested(1, 0, t + secs(60)), None);
        // Something that arrives mid-turn within the gap waits for the gap, then is said.
        assert_eq!(n.arrived(approval("Mail"), false, t + secs(10)), None);
        assert_eq!(n.rested(2, 0, t + secs(20)), None, "within the gap");
        assert_eq!(
            n.rested(2, 0, t + secs(35)).as_deref(),
            Some("Two things need you.")
        );
        // Answered before the session rested: nothing is said.
        let mut n = Notices::new();
        assert_eq!(n.arrived(question("Research"), false, t), None);
        assert_eq!(n.rested(0, 0, t + secs(5)), None);
        assert_eq!(n.rested(1, 0, t + secs(6)), None, "forgotten");
    }

    #[test]
    fn cleared_nothing_is_said_and_nothing_waits() {
        let t = Instant::now();
        let mut n = Notices::new();
        assert_eq!(n.arrived(approval("Research"), false, t), None);
        n.clear();
        assert_eq!(n.rested(1, 0, t + secs(1)), None);
        assert_eq!(n.rested(1, 0, t + secs(60)), None);
    }

    #[test]
    fn a_question_and_an_approval_together_are_counted() {
        let t = Instant::now();
        let mut n = Notices::new();
        assert_eq!(n.arrived(question("Research"), false, t), None);
        assert_eq!(n.arrived(approval("Mail"), false, t), None);
        assert_eq!(n.rested(2, 0, t).as_deref(), Some("Two things need you."));
        // Arriving together while resting, the second waits for the gap and is counted.
        let mut n = Notices::new();
        assert!(n.arrived(question("Research"), true, t).is_some());
        assert_eq!(n.arrived(approval("Mail"), true, t), None);
        assert_eq!(
            n.rested(3, 0, t + NOTICE_GAP).as_deref(),
            Some("Three things need you.")
        );
    }

    #[test]
    fn the_viewed_dot_is_not_announced() {
        let need = approval("Research");
        assert_eq!(announce(Some("d1"), Some("d1"), need.clone()), None);
        assert_eq!(
            announce(Some("d1"), Some("d2"), need.clone()),
            Some(need.clone())
        );
        assert_eq!(announce(Some("d1"), None, need.clone()), Some(need.clone()));
        // A dot that is not known is announced.
        assert_eq!(announce(None, Some("d1"), need.clone()), Some(need));
    }

    #[test]
    fn lagged_events_recount_from_the_inbox() {
        let t = Instant::now();
        let mut n = Notices::new();
        n.missed();
        assert_eq!(n.wait_for(t), Some(Duration::ZERO));
        assert_eq!(n.rested(4, 1, t).as_deref(), Some("Four things need you."));
        // Missed with one known thing waiting: the Inbox's one thing is not assumed to be it.
        let mut n = Notices::new();
        n.arrived(approval("Research"), false, t);
        n.missed();
        assert_eq!(n.rested(1, 0, t).as_deref(), Some("One thing needs you."));
        // Something arriving after a miss is counted too, not said alone.
        let mut n = Notices::new();
        n.missed();
        assert_eq!(n.arrived(approval("Research"), true, t), None);
        assert_eq!(n.rested(2, 0, t).as_deref(), Some("Two things need you."));
        // Nothing waits in the Inbox after all: nothing is said, and nothing waits.
        let mut n = Notices::new();
        n.missed();
        assert_eq!(n.rested(0, 0, t), None);
        assert_eq!(n.wait_for(t), None);
    }

    #[test]
    fn only_approvals_and_questions_need_the_person() {
        use dots_core::model::{Approval, ApprovalStatus, Message};
        let approval = Approval {
            id: "a1".into(),
            run_id: "r1".into(),
            tool: "Bash".into(),
            input: serde_json::json!({"command": "rm -rf /secret"}),
            input_hash: String::new(),
            status: ApprovalStatus::Pending,
            note: None,
            parked: false,
            resolved: false,
            created_at: String::new(),
            decided_at: None,
            self_apply: false,
            apply_state: None,
            applied_at: None,
            mcp_fingerprint: None,
            decided_via: None,
        };
        assert_eq!(
            arrival(&RuntimeEvent::ApprovalRequested { approval }),
            Some(Arrival::Approval {
                approval_id: "a1".into(),
                run_id: "r1".into()
            })
        );
        let message = |kind| Message {
            id: "m1".into(),
            dot_id: "d1".into(),
            dot_name: "Research".into(),
            run_id: None,
            kind,
            text: "Which folder should I clean?".into(),
            choices: None,
            created_at: String::new(),
            read_at: None,
            answered_at: None,
            answer_run_id: None,
            dismissed_at: None,
        };
        assert_eq!(
            arrival(&RuntimeEvent::MessagePosted {
                message: message(MessageKind::Question)
            }),
            Some(Arrival::Question {
                dot_id: "d1".into(),
                dot_name: "Research".into()
            })
        );
        assert_eq!(
            arrival(&RuntimeEvent::MessagePosted {
                message: message(MessageKind::Note)
            }),
            None
        );
    }

    #[test]
    fn a_line_carries_only_the_dots_name_and_the_kind() {
        let t = Instant::now();
        // Control and hidden characters go; the name is cut at 80 bytes.
        let mut n = Notices::new();
        let line = n
            .arrived(approval("Re\u{202E}search\n\tNow\u{200B}"), true, t)
            .unwrap();
        assert_eq!(line, "Research Now needs your approval.");
        assert_eq!(spoken_name(&"é".repeat(60)), "é".repeat(40));
        assert_eq!(spoken_name("  \u{200B}\u{FEFF} "), "A dot");
        let mut n = Notices::new();
        assert_eq!(
            n.arrived(question(""), true, t).as_deref(),
            Some("A dot has a question for you.")
        );
        assert_eq!(n.kind(), "question");
        assert_eq!(count_word(2), "Two");
        assert_eq!(count_word(20), "Twenty");
        assert_eq!(count_word(21), "21");
    }
}
