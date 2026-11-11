//! Spoken notices: Dot says one short line aloud when something needs the person (Open Dot
//! design spec, step 5). Off until "Tell me aloud when something needs me" (`voice_notices`)
//! is on, and only while the voice service is ready and the session rests.
//!
//! A line is built here from the dot's name and the kind of thing only: never a tool, an
//! input, a path or a question's words. A dot's name is said only when it is short and plain
//! (`plain_name`): names come from other people too (team dots), and a long one could be made
//! to read as a sentence of its own. A private approval names nothing. The fixed words never
//! say "dot": with the wake phrase on, the spotter listens while a notice plays.
//!
//! At most one line every `NOTICE_GAP`; what arrives meanwhile is counted, from the Inbox,
//! once the gap has passed and the session rests. Only what waits on an answer is counted:
//! pending approvals and unanswered questions, not those of the dot the person is viewing.
//! After missed events a recount is said only when more waits than at the last notice.
//! Nothing spoken is logged, only that a notice was and its kind.
//!
//! "The session rests" includes the stage being closed: a question or a card left showing
//! holds notices back, with no bound of its own. The stage clears on close (Esc, "Dot,
//! stop"), and the mind's own limits (a question's answer time, a turn's think limit) clear
//! what it showed, so the hold ends with them.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dots_core::events::RuntimeEvent;
use dots_core::model::MessageKind;
use dots_core::Runtime;
use tokio::sync::broadcast::error::RecvError;

use super::service::Service;

/// The least time between two notices.
pub const NOTICE_GAP: Duration = Duration::from_secs(30);

/// The longest a dot's name may be when spoken, in bytes, before the plainness check.
const NAME_BYTES: usize = 80;
/// A name said aloud has at most this many characters...
const PLAIN_CHARS: usize = 32;
/// ...and at most this many words.
const PLAIN_WORDS: usize = 4;
const PRIVATE: &str = "One private approval is waiting for you.";
const AN_APPROVAL: &str = "An approval is waiting for you.";
const A_QUESTION: &str = "A question is waiting for you.";

/// Something that needs the person; `id` is the approval's or the question message's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needs {
    Approval {
        id: String,
        dot_name: String,
        private: bool,
    },
    Question {
        id: String,
        dot_name: String,
    },
}

impl Needs {
    fn id(&self) -> &str {
        match self {
            Needs::Approval { id, .. } | Needs::Question { id, .. } => id,
        }
    }

    /// The line for this one thing alone.
    fn line(&self) -> String {
        match self {
            Needs::Approval { private: true, .. } => PRIVATE.to_string(),
            Needs::Approval { dot_name, .. } => match plain_name(dot_name) {
                Some(name) => format!("{name} needs your approval."),
                None => AN_APPROVAL.to_string(),
            },
            Needs::Question { dot_name, .. } => match plain_name(dot_name) {
                Some(name) => format!("{name} has a question for you."),
                None => A_QUESTION.to_string(),
            },
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Needs::Approval { private: true, .. } => WaitKind::PrivateApproval.name(),
            Needs::Approval { .. } => WaitKind::Approval.name(),
            Needs::Question { .. } => WaitKind::Question.name(),
        }
    }
}

/// One thing in the Inbox that waits on an answer from the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// The approval's id, or the question message's.
    pub id: String,
    pub kind: WaitKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    Approval,
    PrivateApproval,
    Question,
}

impl WaitKind {
    /// The line for one thing of this kind, naming no dot.
    fn nameless(self) -> &'static str {
        match self {
            WaitKind::Approval => AN_APPROVAL,
            WaitKind::PrivateApproval => PRIVATE,
            WaitKind::Question => A_QUESTION,
        }
    }

    fn name(self) -> &'static str {
        match self {
            WaitKind::Approval => "approval",
            WaitKind::PrivateApproval => "private approval",
            WaitKind::Question => "question",
        }
    }
}

/// A dot's name cleaned for speech: on one line, without control or hidden characters, at
/// most 80 bytes (cut on a character boundary); empty when nothing is left.
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
    out
}

/// The name to say, when it is short and plain: at most 32 characters and 4 words once
/// cleaned, no line break, no sentence punctuation (`. ! ? : ;`), and nothing that sounds
/// like the wake or stop phrase. `None` says the nameless line instead.
pub fn plain_name(name: &str) -> Option<String> {
    if name.contains(['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}']) {
        return None;
    }
    let name = spoken_name(name);
    if name.is_empty()
        || name.chars().count() > PLAIN_CHARS
        || name.split(' ').count() > PLAIN_WORDS
        || name.contains(['.', '!', '?', ':', ';'])
    {
        return None;
    }
    // Compared as spoken: letters and digits only, words one space apart.
    let heard = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let heard = format!(" {heard} ");
    if heard.contains(" hey dot ") || heard.contains(" dot stop ") {
        return None;
    }
    Some(name)
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
    /// How many things the last notice spoke of; lowered when a recount finds fewer, so
    /// only something new makes a recount worth saying.
    last_spoken_count: usize,
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
        self.spoke(now, kind, 1);
        Some(line)
    }

    /// The session came to rest. Returns the sentence to speak now, if something is still
    /// waiting. `inbox` is what waits on an answer in the Inbox now.
    pub fn rested(&mut self, inbox: &[Waiting], now: Instant) -> Option<String> {
        if !self.pending() || !self.may_speak(now) {
            return None;
        }
        let count = inbox.len();
        // Nothing new arrived (only events were missed): said only when more waits than the
        // last notice spoke of.
        if self.waiting.is_empty() && count <= self.last_spoken_count {
            self.last_spoken_count = count;
            self.clear();
            return None;
        }
        let (line, kind) = match inbox {
            // Answered meanwhile: nothing to say.
            [] => {
                self.last_spoken_count = 0;
                self.clear();
                return None;
            }
            // Named only when it is the very thing a notice was about.
            [only] => match self.waiting.iter().find(|need| need.id() == only.id) {
                Some(need) if only.kind != WaitKind::PrivateApproval => (need.line(), need.kind()),
                _ => (only.kind.nameless().to_string(), only.kind.name()),
            },
            _ => (
                format!("{} things are waiting for you.", count_word(count)),
                "several",
            ),
        };
        self.spoke(now, kind, count);
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

    /// What the last line spoke of: "approval", "question", "private approval" or
    /// "several". For the log, which never carries the line.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Moves the last notice back by the gap, as if it had been said long ago.
    #[cfg(test)]
    pub fn spoke_long_ago(&mut self) {
        self.last_spoken = self.last_spoken.and_then(|at| at.checked_sub(NOTICE_GAP));
    }

    fn may_speak(&self, now: Instant) -> bool {
        self.last_spoken
            .is_none_or(|at| now.saturating_duration_since(at) >= NOTICE_GAP)
    }

    fn spoke(&mut self, now: Instant, kind: &'static str, count: usize) {
        self.last_spoken = Some(now);
        self.last_spoken_count = count;
        self.kind = kind;
        self.clear();
    }
}

// ---- the listener ---------------------------------------------------------------------------

/// What a runtime event may ask of the person: an approval, or a question a dot posted. A
/// note does not need the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrival {
    Approval {
        approval_id: String,
        run_id: String,
    },
    Question {
        message_id: String,
        dot_id: String,
        dot_name: String,
    },
}

pub fn arrival(ev: &RuntimeEvent) -> Option<Arrival> {
    match ev {
        RuntimeEvent::ApprovalRequested { approval } => Some(Arrival::Approval {
            approval_id: approval.id.clone(),
            run_id: approval.run_id.clone(),
        }),
        RuntimeEvent::MessagePosted { message } if message.kind == MessageKind::Question => {
            Some(Arrival::Question {
                message_id: message.id.clone(),
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

/// What `resolve` reads: the runtime in the app, a fake in tests.
#[async_trait]
pub trait Lookup: Send + Sync {
    /// Whether the approval touches something private; `None` when that could not be read.
    async fn touches_private(&self, approval_id: &str) -> Option<bool>;
    /// The dot a run belongs to; `None` when the run cannot be found.
    async fn run_dot(&self, run_id: &str) -> Option<String>;
    async fn dot_name(&self, dot_id: &str) -> Option<String>;
    /// The dot whose conversation the person is reading.
    fn viewing(&self) -> Option<String>;
}

#[async_trait]
impl Lookup for Runtime {
    async fn touches_private(&self, approval_id: &str) -> Option<bool> {
        self.approval_touches_private(approval_id).await.ok()
    }
    async fn run_dot(&self, run_id: &str) -> Option<String> {
        self.get_run(run_id).await.map(|r| r.dot_id).ok()
    }
    async fn dot_name(&self, dot_id: &str) -> Option<String> {
        self.get_dot(dot_id).await.map(|d| d.spec.name).ok()
    }
    fn viewing(&self) -> Option<String> {
        Runtime::viewing(self)
    }
}

/// What needs the person, found from the runtime. A private approval's dot name is not even
/// read: nothing about it is said. A check that fails counts as private; a dot that cannot
/// be found leaves the name empty, so the line names none.
pub async fn resolve(rt: &dyn Lookup, arrival: Arrival) -> Option<Needs> {
    match arrival {
        Arrival::Approval {
            approval_id,
            run_id,
        } => {
            let private = rt.touches_private(&approval_id).await.unwrap_or(true);
            let dot_id = rt.run_dot(&run_id).await;
            if crate::events::quiet(dot_id.as_deref(), rt.viewing().as_deref()) {
                return None;
            }
            let dot_name = match (&dot_id, private) {
                (Some(id), false) => rt.dot_name(id).await.unwrap_or_default(),
                _ => String::new(),
            };
            Some(Needs::Approval {
                id: approval_id,
                dot_name,
                private,
            })
        }
        Arrival::Question {
            message_id,
            dot_id,
            dot_name,
        } => announce(
            Some(&dot_id),
            rt.viewing().as_deref(),
            Needs::Question {
                id: message_id,
                dot_name,
            },
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
                    if let Some(need) = resolve(rt.as_ref(), arrival).await {
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
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn approval(name: &str) -> Needs {
        Needs::Approval {
            id: format!("a:{name}"),
            dot_name: name.into(),
            private: false,
        }
    }

    fn private(name: &str) -> Needs {
        Needs::Approval {
            id: format!("p:{name}"),
            dot_name: name.into(),
            private: true,
        }
    }

    fn question(name: &str) -> Needs {
        Needs::Question {
            id: format!("q:{name}"),
            dot_name: name.into(),
        }
    }

    /// The Inbox's waiting item for `need`.
    fn item(need: &Needs) -> Waiting {
        let kind = match need {
            Needs::Approval { private: true, .. } => WaitKind::PrivateApproval,
            Needs::Approval { .. } => WaitKind::Approval,
            Needs::Question { .. } => WaitKind::Question,
        };
        Waiting {
            id: need.id().to_string(),
            kind,
        }
    }

    /// `n` approvals waiting in the Inbox, none of them one a notice was about.
    fn others(n: usize) -> Vec<Waiting> {
        (0..n)
            .map(|i| Waiting {
                id: format!("other{i}"),
                kind: WaitKind::Approval,
            })
            .collect()
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
            assert_eq!(n.rested(&others(10), t + at), None, "{at:?}");
        }
        // Thirty seconds on: one line that counts everything waiting, from the Inbox.
        assert_eq!(
            n.rested(&others(10), t + secs(30)).as_deref(),
            Some("Ten things are waiting for you.")
        );
        // Nothing new since: silence.
        assert_eq!(n.rested(&others(10), t + secs(90)), None);
    }

    #[test]
    fn a_private_approval_names_nothing() {
        let t = Instant::now();
        let mut n = Notices::new();
        let secret = private("Secret Payroll");
        let line = n.arrived(secret.clone(), true, t).unwrap();
        assert_eq!(line, "One private approval is waiting for you.");
        assert!(!line.contains("Secret") && !line.contains("Payroll"));
        // Counted later, alone: the same line.
        let mut n = Notices::new();
        assert_eq!(n.arrived(secret.clone(), false, t), None);
        let line = n.rested(&[item(&secret)], t).unwrap();
        assert_eq!(line, "One private approval is waiting for you.");
        assert_eq!(n.kind(), "private approval");
        // With others, it is only counted.
        let mut n = Notices::new();
        n.arrived(secret.clone(), false, t);
        n.arrived(approval("Research"), false, t);
        assert_eq!(
            n.rested(&[item(&secret), item(&approval("Research"))], t)
                .as_deref(),
            Some("Two things are waiting for you.")
        );
    }

    #[test]
    fn what_arrives_mid_turn_is_said_once_when_the_session_next_rests() {
        let t = Instant::now();
        let mut n = Notices::new();
        let research = question("Research");
        // The person is speaking, or Dot is mid-turn.
        assert_eq!(n.arrived(research.clone(), false, t), None);
        // At rest, still waiting: said once.
        assert_eq!(
            n.rested(&[item(&research)], t + secs(5)).as_deref(),
            Some("Research has a question for you.")
        );
        assert_eq!(n.rested(&[item(&research)], t + secs(60)), None);
        // Something that arrives mid-turn within the gap waits for the gap, then is said.
        let mail = approval("Mail");
        assert_eq!(n.arrived(mail.clone(), false, t + secs(10)), None);
        let both = [item(&research), item(&mail)];
        assert_eq!(n.rested(&both, t + secs(20)), None, "within the gap");
        assert_eq!(
            n.rested(&both, t + secs(35)).as_deref(),
            Some("Two things are waiting for you.")
        );
        // Answered before the session rested: nothing is said.
        let mut n = Notices::new();
        assert_eq!(n.arrived(research.clone(), false, t), None);
        assert_eq!(n.rested(&[], t + secs(5)), None);
        assert_eq!(n.rested(&[item(&research)], t + secs(6)), None, "forgotten");
    }

    #[test]
    fn cleared_nothing_is_said_and_nothing_waits() {
        let t = Instant::now();
        let mut n = Notices::new();
        assert_eq!(n.arrived(approval("Research"), false, t), None);
        n.clear();
        assert_eq!(n.rested(&others(1), t + secs(1)), None);
        assert_eq!(n.rested(&others(1), t + secs(60)), None);
    }

    #[test]
    fn a_question_and_an_approval_together_are_counted() {
        let t = Instant::now();
        let mut n = Notices::new();
        assert_eq!(n.arrived(question("Research"), false, t), None);
        assert_eq!(n.arrived(approval("Mail"), false, t), None);
        assert_eq!(
            n.rested(&others(2), t).as_deref(),
            Some("Two things are waiting for you.")
        );
        assert_eq!(n.kind(), "several");
        // Arriving together while resting, the second waits for the gap and is counted.
        let mut n = Notices::new();
        assert!(n.arrived(question("Research"), true, t).is_some());
        assert_eq!(n.arrived(approval("Mail"), true, t), None);
        assert_eq!(
            n.rested(&others(3), t + NOTICE_GAP).as_deref(),
            Some("Three things are waiting for you.")
        );
    }

    #[test]
    fn one_waiting_item_is_named_only_when_it_is_the_one_the_notice_is_about() {
        let t = Instant::now();
        let research = question("Research");
        // The one question waiting is another one: it is not given this dot's name.
        let mut n = Notices::new();
        n.arrived(research.clone(), false, t);
        let other = Waiting {
            id: "q:other".into(),
            kind: WaitKind::Question,
        };
        assert_eq!(
            n.rested(&[other], t).as_deref(),
            Some("A question is waiting for you.")
        );
        // Nor is an approval that waits instead.
        let mut n = Notices::new();
        n.arrived(research.clone(), false, t);
        assert_eq!(
            n.rested(&others(1), t).as_deref(),
            Some("An approval is waiting for you.")
        );
        assert_eq!(n.kind(), "approval");
        // The very one: named.
        let mut n = Notices::new();
        n.arrived(research.clone(), false, t);
        assert_eq!(
            n.rested(&[item(&research)], t).as_deref(),
            Some("Research has a question for you.")
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
        assert_eq!(
            n.rested(&others(4), t).as_deref(),
            Some("Four things are waiting for you.")
        );
        // Missed with one known thing waiting: the Inbox's one thing is named only when it
        // is that thing.
        let research = approval("Research");
        let mut n = Notices::new();
        n.arrived(research.clone(), false, t);
        n.missed();
        assert_eq!(
            n.rested(&others(1), t).as_deref(),
            Some("An approval is waiting for you.")
        );
        let mut n = Notices::new();
        n.arrived(research.clone(), false, t);
        n.missed();
        assert_eq!(
            n.rested(&[item(&research)], t).as_deref(),
            Some("Research needs your approval.")
        );
        // Something arriving after a miss is counted too, not said alone.
        let mut n = Notices::new();
        n.missed();
        assert_eq!(n.arrived(research.clone(), true, t), None);
        assert_eq!(
            n.rested(&others(2), t).as_deref(),
            Some("Two things are waiting for you.")
        );
        // Nothing waits in the Inbox after all: nothing is said, and nothing waits.
        let mut n = Notices::new();
        n.missed();
        assert_eq!(n.rested(&[], t), None);
        assert_eq!(n.wait_for(t), None);
    }

    #[test]
    fn a_lag_with_the_same_count_says_nothing_and_a_larger_one_speaks_once() {
        let t = Instant::now();
        let mut n = Notices::new();
        n.missed();
        assert_eq!(
            n.rested(&others(3), t).as_deref(),
            Some("Three things are waiting for you.")
        );
        // Missed again, as many waiting as were said: nothing, and nothing waits.
        n.missed();
        assert_eq!(n.rested(&others(3), t + secs(40)), None);
        assert!(!n.pending());
        // Fewer: nothing either.
        n.missed();
        assert_eq!(n.rested(&others(2), t + secs(50)), None);
        // More than at the last notice... (counted from the lowered count: two)
        n.missed();
        assert_eq!(
            n.rested(&others(3), t + secs(60)).as_deref(),
            Some("Three things are waiting for you.")
        );
        // ...said once.
        n.missed();
        assert_eq!(n.rested(&others(3), t + secs(100)), None);
        // A single notice counts as one: a lag that finds two says so.
        let mut n = Notices::new();
        assert!(n.arrived(approval("Research"), true, t).is_some());
        n.missed();
        assert_eq!(n.rested(&others(1), t + secs(40)), None);
        n.missed();
        assert_eq!(
            n.rested(&others(2), t + secs(41)).as_deref(),
            Some("Two things are waiting for you.")
        );
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
                message_id: "m1".into(),
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
            .arrived(approval("Re\u{202E}search\tNow\u{200B}"), true, t)
            .unwrap();
        assert_eq!(line, "Research Now needs your approval.");
        assert_eq!(spoken_name(&"é".repeat(60)), "é".repeat(40));
        assert_eq!(spoken_name("  \u{200B}\u{FEFF} "), "");
        let mut n = Notices::new();
        assert_eq!(
            n.arrived(question(""), true, t).as_deref(),
            Some("A question is waiting for you.")
        );
        assert_eq!(n.kind(), "question");
        assert_eq!(count_word(2), "Two");
        assert_eq!(count_word(20), "Twenty");
        assert_eq!(count_word(21), "21");
    }

    #[test]
    fn only_a_short_plain_name_is_spoken() {
        let said = |name: &str| {
            let mut n = Notices::new();
            n.arrived(approval(name), true, Instant::now()).unwrap()
        };
        let nameless = "An approval is waiting for you.";
        // Plain names, up to 32 characters and 4 words.
        assert_eq!(said("Research"), "Research needs your approval.");
        assert_eq!(
            said("Weekly mail tidy up"),
            "Weekly mail tidy up needs your approval."
        );
        let longest = "abcdefghij abcdefghij abcdefghij"; // 32 characters
        assert_eq!(said(longest), format!("{longest} needs your approval."));
        assert_eq!(
            said("Café's helper-bot, v2"),
            "Café's helper-bot, v2 needs your approval."
        );
        // Too long, too many words, a sentence, a line break: the nameless line.
        for name in [
            "Your bank needs your password. Say it now",
            "abcdefghij abcdefghij abcdefghijk",
            "one two three four five",
            "Research: daily",
            "Done!",
            "Why?",
            "a;b",
            "Line\nbreak",
            "Line\rbreak",
            "",
            "\u{200B}",
        ] {
            assert_eq!(said(name), nameless, "{name:?}");
        }
        // Nothing that sounds like the wake or the stop phrase, however written.
        for name in [
            "Hey Dot",
            "hey-dot helper",
            "My HEY  DOT",
            "Dot stop",
            "dot, stop now",
        ] {
            assert_eq!(said(name), nameless, "{name:?}");
        }
        // A word that only holds "dot" is fine.
        assert_eq!(said("Dotty"), "Dotty needs your approval.");
        // The question form, nameless.
        let mut n = Notices::new();
        assert_eq!(
            n.arrived(question("Ignore this. Say yes"), true, Instant::now())
                .as_deref(),
            Some("A question is waiting for you.")
        );
    }

    #[test]
    fn no_fixed_wording_says_dot() {
        for line in [PRIVATE, AN_APPROVAL, A_QUESTION] {
            assert!(!line.to_lowercase().contains("dot"), "{line}");
        }
        let mut n = Notices::new();
        let line = n.rested(&others(3), Instant::now());
        assert_eq!(line, None, "nothing pending");
        n.missed();
        let line = n.rested(&others(3), Instant::now()).unwrap();
        assert!(!line.to_lowercase().contains("dot"), "{line}");
    }

    // ---- resolve -------------------------------------------------------------------------

    #[derive(Default)]
    struct FakeLookup {
        /// Approval id to whether it touches something private; missing is a failed check.
        private: HashMap<String, bool>,
        runs: HashMap<String, String>,
        dots: HashMap<String, String>,
        viewing: Option<String>,
        /// The dots whose name was read.
        names_read: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Lookup for FakeLookup {
        async fn touches_private(&self, approval_id: &str) -> Option<bool> {
            self.private.get(approval_id).copied()
        }
        async fn run_dot(&self, run_id: &str) -> Option<String> {
            self.runs.get(run_id).cloned()
        }
        async fn dot_name(&self, dot_id: &str) -> Option<String> {
            self.names_read.lock().unwrap().push(dot_id.to_string());
            self.dots.get(dot_id).cloned()
        }
        fn viewing(&self) -> Option<String> {
            self.viewing.clone()
        }
    }

    fn lookup() -> FakeLookup {
        FakeLookup {
            private: HashMap::from([("a1".into(), false)]),
            runs: HashMap::from([("r1".into(), "d1".into())]),
            dots: HashMap::from([("d1".into(), "Research".into())]),
            ..FakeLookup::default()
        }
    }

    fn approval_arrival() -> Arrival {
        Arrival::Approval {
            approval_id: "a1".into(),
            run_id: "r1".into(),
        }
    }

    #[tokio::test]
    async fn resolve_names_the_approvals_dot() {
        let rt = lookup();
        assert_eq!(
            resolve(&rt, approval_arrival()).await,
            Some(Needs::Approval {
                id: "a1".into(),
                dot_name: "Research".into(),
                private: false
            })
        );
    }

    #[tokio::test]
    async fn resolve_does_not_announce_an_approval_in_the_viewed_dot() {
        let rt = FakeLookup {
            viewing: Some("d1".into()),
            ..lookup()
        };
        assert_eq!(resolve(&rt, approval_arrival()).await, None);
        // Nor a question there.
        let question = Arrival::Question {
            message_id: "m1".into(),
            dot_id: "d1".into(),
            dot_name: "Research".into(),
        };
        assert_eq!(resolve(&rt, question).await, None);
    }

    #[tokio::test]
    async fn resolve_counts_a_failed_private_check_as_private() {
        let rt = FakeLookup {
            private: HashMap::new(),
            ..lookup()
        };
        let need = resolve(&rt, approval_arrival()).await.unwrap();
        assert_eq!(
            need,
            Needs::Approval {
                id: "a1".into(),
                dot_name: String::new(),
                private: true
            }
        );
        assert!(
            rt.names_read.lock().unwrap().is_empty(),
            "the name was read"
        );
        assert_eq!(need.line(), "One private approval is waiting for you.");
    }

    #[tokio::test]
    async fn resolve_gives_the_nameless_form_for_a_run_that_cannot_be_found() {
        let rt = FakeLookup {
            runs: HashMap::new(),
            ..lookup()
        };
        let need = resolve(&rt, approval_arrival()).await.unwrap();
        assert_eq!(
            need,
            Needs::Approval {
                id: "a1".into(),
                dot_name: String::new(),
                private: false
            }
        );
        assert_eq!(need.line(), "An approval is waiting for you.");
    }
}
