//! Open Dot, the voice assistant: its quick matcher, its reply reader, the picture of now, the
//! prompt, its thinking through the CLI, acting on checked records, and the mind that runs one
//! spoken turn.
//!
//! The mind holds what waits on the person between turns: a confirmation, a question with
//! options, or the setup of a new dot. Approving, denying, running, creating, and sending or
//! answering where the person did not name the target, happen only after the person says yes
//! (or clicks Yes) to a question Dots built from the record. Nothing the model writes can pass
//! one: a model's own question is answered back to the model, never to a deed.
//!
//! Nothing heard, said or pictured is logged; only timings and the kind of step.

pub mod act;
pub mod picture;
pub mod prompt;
pub mod quick;
pub mod reply;
pub mod think;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::engine::claude::once::OnceFailure;
use crate::schedule::{Rule, Schedule};
use crate::setup::{SetupAnswer, SetupProposal, SetupRequest, SetupRound};
use crate::{Error, Result, Runtime};
use act::{
    needs_confirmation, outcome, plain_error, plan, AskFor, Deed, OpenTarget, Outcome, Step,
    DOT_GONE, HANDLED,
};
use picture::{clean, Picture, View, NAME_BYTES};
use prompt::{prompt, Turn, INSTRUCTIONS};
use quick::{firm_yes, quick, words_of, Quick};
use reply::{read_reply, SpeechGate, Wish};
use think::{cap_hit, Thinker};

/// What the mind tells the stage, in order.
// Sent once each, by value: not boxed, so the stage matches on it plainly.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MindEvent {
    Thinking,
    /// "One moment." once, when nothing was said within `FILLER_AFTER`.
    Filler {
        text: String,
    },
    Say {
        delta: String,
    },
    SayEnd {
        then: Then,
    },
    /// Spoken by the app; the options are shown.
    Ask {
        question: String,
        options: Vec<String>,
    },
    Card {
        card: Card,
    },
    Open {
        target: OpenTarget,
    },
    /// The step that waited was dropped: the stage clears its options and card.
    Cleared,
}

/// What the stage does after a turn's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// Stop listening.
    #[default]
    Rest,
    /// Listen a little while for a follow-up.
    FollowUp,
    /// Listen for the answer to the question asked.
    Ask,
}

/// What the stage shows beside the words.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Card {
    Proposal { proposal: SetupProposal },
    Confirm { deed: Deed },
    List { title: String, lines: Vec<String> },
}

/// Turns and what waits are forgotten after this long without a turn.
pub const IDLE_FORGET: Duration = Duration::from_secs(600);
/// A step that waits on the person is dropped this long after it was asked: silence is no,
/// whether or not the app calls `lapse`.
pub const PENDING_LIMIT: Duration = Duration::from_secs(60);
/// "One moment." is said when nothing was said this long after the request started.
pub const FILLER_AFTER: Duration = Duration::from_millis(1500);
/// Most turns kept, and the longest record of what was said in one.
const KEPT_TURNS: usize = 12;
const SAID_BYTES: usize = 400;

const FILLER: &str = "One moment.";
const OKAY: &str = "Okay.";
const WONT: &str = "Okay, I won't.";
const NOT_SENT: &str = "Okay, not sent.";
const NOT_CREATED: &str = "Okay, not created.";
const NOTHING_WAITING: &str = "There's nothing waiting on a yes or no.";
const CLEAR_YES: &str = "I need a clear yes for that. Say yes, or no.";
const NOTHING_SAID: &str = "I haven't said anything yet.";
const NO_CLI: &str =
    "I can't think right now: the Claude CLI isn't set up. Open Settings to fix it.";
const TIMED_OUT: &str = "That took too long. Try again.";
const FAILED: &str = "Something went wrong reaching Claude.";
const WENT_WRONG: &str = "Something went wrong. Try again.";
const DIDNT_CATCH: &str = "I didn't catch what to do with that. Say it another way?";
const SETTING_UP: &str = "Let me set that up.";
const SETUP_FAILED: &str = "Setup could not finish.";
const NEEDS_TITLE: &str = "What needs you";
/// Longest first message said in the "Create it?" question.
const FIRST_MESSAGE_BYTES: usize = 160;

fn yes_no() -> Vec<String> {
    vec!["Yes".into(), "No".into()]
}

/// One spoken turn in, events out. See the module documentation.
pub struct Mind {
    thinker: Arc<dyn Thinker>,
    state: Mutex<State>,
    /// `FILLER_AFTER`; shorter in tests.
    filler_after: Duration,
    /// `IDLE_FORGET`; shorter in tests.
    idle_forget: Duration,
    /// `PENDING_LIMIT`; shorter in tests.
    pending_limit: Duration,
    /// Held while a deed is checked, carried out and its outcome said: a newer turn waits for
    /// it before it begins, so a deed that started is never cut off, and its words never land
    /// after a newer turn's. A turn answering a step that waits holds it from its start.
    deeds: tokio::sync::Mutex<()>,
}

/// What a turn holds while it may carry out a deed (`Mind::deeds`).
type Gate<'a> = tokio::sync::MutexGuard<'a, ()>;

#[derive(Default)]
struct State {
    turns: Vec<Turn>,
    pending: Option<Pending>,
    /// The words of the last reply, for "say that again", and how it ended.
    last_said: String,
    last_then: Then,
    /// Questions read aloud in the last turn.
    read_aloud: Vec<String>,
    last_heard: Option<Instant>,
    /// The turn running now; a newer one makes an older one's events go nowhere.
    turn: u64,
    cancel: Option<CancellationToken>,
    /// How many steps that waited were taken by a yes, and whether one's deed is being done:
    /// a second yes for the same step is that same answer, and does nothing.
    decided: u64,
    deciding: bool,
}

impl State {
    /// Forgets turns, replies and what waits; returns whether a step that waited was dropped.
    fn forget(&mut self) -> bool {
        self.turns.clear();
        self.last_said.clear();
        self.last_then = Then::Rest;
        self.read_aloud.clear();
        self.pending.take().is_some()
    }
}

/// A step that waits on the person, with the account and workspace it was made in.
#[derive(Debug, Clone)]
struct Pending {
    kind: Waiting,
    account: String,
    workspace: String,
    /// When it was asked; it is dropped after `PENDING_LIMIT`.
    asked_at: Instant,
    /// An unclear yes ("okay", "sure") was heard once and the person was asked for a clear one.
    retried: bool,
}

/// How the person answered a step Dots asked a yes on.
#[derive(Debug, PartialEq)]
enum Answer {
    Yes,
    No,
    /// "Okay", "sure", "right": asked once more for a clear yes.
    Unclear,
    /// A second unclear yes: dropped.
    GiveUp,
    /// Anything else: dropped, and the words are a new turn.
    Other,
}

#[derive(Debug, Clone)]
enum Waiting {
    /// A deed Dots asked about; it runs only on yes.
    Confirm { deed: Deed, question: String },
    /// A question with options (or an open one, from the model).
    Ask {
        question: String,
        options: Vec<String>,
        then: AskFor,
        /// For `AskFor::Dot`: the ids of the dots offered, in order.
        dot_ids: Vec<String>,
    },
    /// A setup agent's question, one of a round.
    SetupQuestion {
        id: String,
        description: String,
        rounds: Vec<SetupRound>,
        questions: Vec<String>,
        answers: Vec<String>,
    },
    /// A proposed dot: "Create it?".
    Create {
        proposal: SetupProposal,
        question: String,
    },
}

impl Waiting {
    /// The question being asked and its options.
    fn asked(&self) -> (String, Vec<String>) {
        match self {
            Waiting::Confirm { question, .. } | Waiting::Create { question, .. } => {
                (question.clone(), yes_no())
            }
            Waiting::Ask {
                question, options, ..
            } => (question.clone(), options.clone()),
            Waiting::SetupQuestion {
                questions, answers, ..
            } => (
                questions.get(answers.len()).cloned().unwrap_or_default(),
                Vec::new(),
            ),
        }
    }

    /// Whether a deed waits on this step's yes: a confirmation, a proposed dot, or an answer
    /// read back.
    fn needs_yes(&self) -> bool {
        matches!(
            self,
            Waiting::Confirm { .. }
                | Waiting::Create { .. }
                | Waiting::Ask {
                    then: AskFor::AnswerCheck { .. },
                    ..
                }
        )
    }

    fn setup_id(&self) -> Option<&str> {
        match self {
            Waiting::SetupQuestion { id, .. } => Some(id),
            _ => None,
        }
    }
}

/// What came in: words, or a click on an option.
enum Input<'a> {
    Heard(&'a str),
    Pick(usize),
}

/// One turn's output: events go out only while the turn is the current one.
struct Out<'a> {
    mind: &'a Mind,
    tx: &'a mpsc::UnboundedSender<MindEvent>,
    turn: u64,
    /// Everything said this turn, questions included: the turn's record.
    said: Mutex<String>,
    /// The words said this turn, for "say that again".
    words: Mutex<String>,
    spoke: AtomicBool,
    read_aloud: Mutex<Vec<String>>,
    /// A deed was carried out: its outcome is said even if the turn was ended meanwhile.
    done: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl<'a> Out<'a> {
    fn live(&self) -> bool {
        lock(&self.mind.state).turn == self.turn
    }

    /// Whether a deed may still be carried out: the turn is the current one and not cancelled.
    fn may_act(&self, cancel: &CancellationToken) -> bool {
        self.live() && !cancel.is_cancelled()
    }

    /// From now on, the turn's events go out whatever ends it: a deed was carried out. The
    /// deed gate keeps any newer turn from beginning until they are out.
    fn deed_done(&self) {
        self.done.store(true, Ordering::SeqCst);
    }

    /// Sends while the turn is the current one (or its deed is done). The check and the send
    /// are made under the state lock, so a newer turn's first event cannot come before this.
    fn send(&self, event: MindEvent) {
        let s = lock(&self.mind.state);
        if s.turn == self.turn || self.done.load(Ordering::SeqCst) {
            let _ = self.tx.send(event);
        }
    }

    /// Streamed words, as they come.
    fn stream(&self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        lock(&self.said).push_str(delta);
        lock(&self.words).push_str(delta);
        self.spoke.store(true, Ordering::SeqCst);
        self.send(MindEvent::Say {
            delta: delta.to_string(),
        });
    }

    /// A sentence of Dots, after what was said before it.
    fn say(&self, sentence: &str) {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            return;
        }
        let gap = {
            let words = lock(&self.words);
            !words.is_empty() && !words.ends_with(char::is_whitespace)
        };
        self.stream(&if gap {
            format!(" {sentence}")
        } else {
            sentence.to_string()
        });
    }

    fn ask(&self, question: &str, options: Vec<String>) {
        {
            let mut said = lock(&self.said);
            if !said.is_empty() && !said.ends_with(char::is_whitespace) {
                said.push(' ');
            }
            said.push_str(question);
        }
        self.send(MindEvent::Ask {
            question: question.to_string(),
            options,
        });
    }

    fn end(&self, then: Then) {
        self.send(MindEvent::SayEnd { then });
    }

    fn spoke(&self) -> bool {
        self.spoke.load(Ordering::SeqCst)
    }
}

impl Mind {
    pub fn new(thinker: Arc<dyn Thinker>) -> Mind {
        Mind {
            thinker,
            state: Mutex::new(State::default()),
            filler_after: FILLER_AFTER,
            idle_forget: IDLE_FORGET,
            pending_limit: PENDING_LIMIT,
            deeds: tokio::sync::Mutex::new(()),
        }
    }

    /// One spoken turn. Events go to `out` in order; returns when the turn is over. A newer
    /// `hear` or `pick` ends this one: its remaining events are not sent.
    pub async fn hear(
        &self,
        rt: &Runtime,
        heard: &str,
        view: &View,
        out: &mpsc::UnboundedSender<MindEvent>,
    ) -> Result<()> {
        self.turn(rt, Input::Heard(heard), view, out).await
    }

    /// Click or key on an option (0-based). Same effect as saying it.
    pub async fn pick(
        &self,
        rt: &Runtime,
        index: usize,
        view: &View,
        out: &mpsc::UnboundedSender<MindEvent>,
    ) -> Result<()> {
        self.turn(rt, Input::Pick(index), view, out).await
    }

    /// The asking time ran out or the stage closed: the step that waits is dropped ("no"), and
    /// a turn in flight is ended, so a reply that lands later asks and installs nothing. A deed
    /// already being carried out still finishes and says its outcome.
    pub fn lapse(&self, out: &mpsc::UnboundedSender<MindEvent>) {
        let dropped = {
            let mut s = lock(&self.state);
            s.turn += 1;
            if let Some(c) = s.cancel.take() {
                c.cancel();
            }
            s.pending.take().is_some()
        };
        if dropped {
            let _ = out.send(MindEvent::Cleared);
        }
    }

    /// Stops a request in flight; keeps turns and what waits.
    pub fn cancel(&self) {
        if let Some(c) = &lock(&self.state).cancel {
            c.cancel();
        }
    }

    /// Forgets everything, and ends a turn in flight. Returns whether a step that waited on the
    /// person was dropped: the caller then clears its options and card, as on `Cleared`.
    pub fn reset(&self) -> bool {
        let mut s = lock(&self.state);
        let dropped = s.forget();
        s.turn += 1;
        if let Some(c) = s.cancel.take() {
            c.cancel();
        }
        dropped
    }

    /// Whether a step waits on the person.
    pub fn pending(&self) -> bool {
        lock(&self.state).pending.is_some()
    }

    /// Starts a turn: forgets after the idle period, ends the turn before, and returns this
    /// turn's number, what cancels its requests (a child of the runtime's shutdown), and
    /// whether forgetting dropped a step that waited.
    fn begin(&self, rt: &Runtime) -> (u64, CancellationToken, bool) {
        let mut s = lock(&self.state);
        let now = Instant::now();
        let forgot = s
            .last_heard
            .is_some_and(|at| now.duration_since(at) > self.idle_forget)
            && s.forget();
        s.last_heard = Some(now);
        s.turn += 1;
        if let Some(c) = s.cancel.take() {
            c.cancel();
        }
        let cancel = rt.shutdown_token().child_token();
        s.cancel = Some(cancel.clone());
        (s.turn, cancel, forgot)
    }

    /// Whether the input is no turn at all: a click on an option that is not offered (it ends
    /// nothing and keeps what waits), or a second yes or click for a step a yes already took
    /// while its deed is being done or since this input arrived (`arrived`).
    fn no_turn(&self, input: &Input<'_>, arrived: u64, busy: bool) -> bool {
        let s = lock(&self.state);
        let options = s
            .pending
            .as_ref()
            .map(|p| p.kind.asked().1)
            .unwrap_or_default();
        let repeated = (busy || s.decided != arrived) && s.pending.is_none();
        match input {
            Input::Pick(i) => *i >= options.len(),
            Input::Heard(h) => repeated && (firm_yes(h) || quick(h, &[]) == Some(Quick::Yes)),
        }
    }

    async fn turn(
        &self,
        rt: &Runtime,
        input: Input<'_>,
        view: &View,
        tx: &mpsc::UnboundedSender<MindEvent>,
    ) -> Result<()> {
        let started = std::time::Instant::now();
        let (arrived, busy) = {
            let s = lock(&self.state);
            (s.decided, s.deciding)
        };
        if busy && self.no_turn(&input, arrived, busy) {
            return Ok(());
        }
        // A deed in flight finishes and says its outcome before this turn begins.
        let gate = self.deeds.lock().await;
        if self.no_turn(&input, arrived, false) {
            return Ok(());
        }
        let (turn, cancel, forgot) = self.begin(rt);
        let o = Out {
            mind: self,
            tx,
            turn,
            said: Mutex::new(String::new()),
            words: Mutex::new(String::new()),
            spoke: AtomicBool::new(false),
            read_aloud: Mutex::new(Vec::new()),
            done: AtomicBool::new(false),
        };
        if forgot {
            o.send(MindEvent::Cleared);
        }
        let kind = self.step(rt, input, view, &o, &cancel, gate).await;
        tracing::debug!(
            ms = started.elapsed().as_millis() as u64,
            step = kind,
            "open dot turn"
        );
        Ok(())
    }

    /// The pending step, dropped first when it is older than the pending limit, or when the
    /// view moved to another account or workspace.
    fn waiting(&self, view: &View, o: &Out<'_>) -> Option<Pending> {
        let (kind, dropped) = {
            let mut s = lock(&self.state);
            let gone = s.pending.as_ref().is_some_and(|p| {
                p.account != view.account
                    || p.workspace != view.workspace
                    || p.asked_at.elapsed() > self.pending_limit
            });
            if gone {
                s.pending = None;
            }
            (s.pending.clone(), gone)
        };
        if dropped {
            o.send(MindEvent::Cleared);
        }
        kind
    }

    /// Takes the pending step away (this turn answers it), with `Cleared`.
    fn drop_pending(&self, o: &Out<'_>) {
        if !o.live() {
            return;
        }
        if lock(&self.state).pending.take().is_some() {
            o.send(MindEvent::Cleared);
        }
    }

    fn set_pending(&self, o: &Out<'_>, kind: Waiting, view: &View) {
        let mut s = lock(&self.state);
        if s.turn == o.turn {
            s.pending = Some(Pending {
                kind,
                account: view.account.clone(),
                workspace: view.workspace.clone(),
                asked_at: Instant::now(),
                retried: false,
            });
        }
    }

    /// Marks the step that waits as asked once more for a clear yes.
    fn retry(&self, o: &Out<'_>) {
        let mut s = lock(&self.state);
        if s.turn == o.turn {
            if let Some(p) = s.pending.as_mut() {
                p.retried = true;
                p.asked_at = Instant::now();
            }
        }
    }

    /// Takes the step that waits for a yes: it is gone, and until `Deciding` is dropped a second
    /// yes is that same answer. `None` when it is gone already (lapsed or reset meanwhile).
    fn take_for_yes(&self, o: &Out<'_>) -> Option<Deciding<'_>> {
        let taken = {
            let mut s = lock(&self.state);
            if s.turn != o.turn || s.pending.take().is_none() {
                return None;
            }
            s.decided += 1;
            s.deciding = true;
            Deciding(self)
        };
        o.send(MindEvent::Cleared);
        Some(taken)
    }

    /// The person said yes to a step that waited on it (already taken away), with the deed
    /// gate held: its deed is done once a fresh picture shows its record still there, and only
    /// while the turn is still live and not cancelled. Once done, its outcome is said.
    #[allow(clippy::too_many_arguments)]
    async fn yes(
        &self,
        rt: &Runtime,
        kind: Waiting,
        heard: &str,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
        _deciding: Deciding<'_>,
    ) -> &'static str {
        let may_act = || o.may_act(cancel);
        match kind {
            Waiting::Confirm { deed, .. }
            | Waiting::Ask {
                then: AskFor::AnswerCheck { deed },
                ..
            } => {
                let Some(done) = confirmed(rt, &deed, view, &may_act).await else {
                    o.end(Then::Rest);
                    return "ended before the deed";
                };
                o.deed_done();
                let then = if done.done {
                    Then::FollowUp
                } else {
                    Then::Rest
                };
                self.finish(o, heard, &done.said, then);
                "confirmed"
            }
            Waiting::Create { proposal, .. } => {
                if !may_act() {
                    o.end(Then::Rest);
                    return "ended before the deed";
                }
                let (said, opened, then) = create(rt, proposal, view).await;
                o.deed_done();
                o.say(&said);
                if let Some(target) = opened {
                    o.send(MindEvent::Open { target });
                }
                o.end(then);
                self.record(o, heard, then);
                "created"
            }
            // `needs_yes` holds only the kinds above
            Waiting::Ask { .. } | Waiting::SetupQuestion { .. } => {
                o.end(Then::Rest);
                "nothing to do"
            }
        }
    }

    /// Records the turn: what was heard and said, the reply for "say that again".
    fn record(&self, o: &Out<'_>, heard: &str, then: Then) {
        let mut s = lock(&self.state);
        if s.turn != o.turn {
            return;
        }
        let said = lock(&o.said).trim().to_string();
        let words = lock(&o.words).trim().to_string();
        s.turns.push(Turn {
            heard: heard.to_string(),
            said: picture::cut(&said, SAID_BYTES).to_string(),
        });
        let extra = s.turns.len().saturating_sub(KEPT_TURNS);
        s.turns.drain(..extra);
        if !words.is_empty() {
            s.last_said = words;
            s.last_then = then;
        }
        s.read_aloud = std::mem::take(&mut *lock(&o.read_aloud));
    }

    /// Says `sentence`, ends the turn with `then` and records it.
    fn finish(&self, o: &Out<'_>, heard: &str, sentence: &str, then: Then) {
        o.say(sentence);
        o.end(then);
        self.record(o, heard, then);
    }

    /// The turn's work; returns the kind of step, for the log.
    /// The deed gate is held from the turn's start; it is let go unless this turn says yes to
    /// a step that waited.
    async fn step(
        &self,
        rt: &Runtime,
        input: Input<'_>,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
        gate: Gate<'_>,
    ) -> &'static str {
        let waiting = self.waiting(view, o);
        let options = waiting
            .as_ref()
            .map(|w| w.kind.asked().1)
            .unwrap_or_default();
        let clicked = matches!(input, Input::Pick(_));
        let (heard, quick) = match input {
            Input::Heard(h) => (h.trim().to_string(), quick(h, &options)),
            Input::Pick(i) => match options.get(i) {
                Some(option) => (option.clone(), Some(Quick::Pick(i))),
                None => {
                    o.end(Then::Rest);
                    return "no such option";
                }
            },
        };

        match quick {
            Some(Quick::Stop) | Some(Quick::Cancel) => {
                let setup = waiting
                    .as_ref()
                    .and_then(|w| w.kind.setup_id().map(str::to_string));
                if let Some(id) = setup {
                    rt.setup_cancel(&id);
                }
                {
                    let mut s = lock(&self.state);
                    if s.turn == o.turn {
                        s.pending = None;
                    }
                }
                o.send(MindEvent::Cleared);
                if quick == Some(Quick::Cancel) {
                    self.finish(o, &heard, OKAY, Then::Rest);
                    return "cancel";
                }
                o.end(Then::Rest);
                return "stop";
            }
            Some(Quick::Repeat) => {
                let (said, then) = {
                    let s = lock(&self.state);
                    (s.last_said.clone(), s.last_then)
                };
                o.say(if said.is_empty() { NOTHING_SAID } else { &said });
                match &waiting {
                    Some(w) => {
                        let (question, options) = w.kind.asked();
                        o.ask(&question, options);
                        o.end(Then::Ask);
                    }
                    None => o.end(then),
                }
                return "repeat";
            }
            _ => {}
        }

        // A step Dots asked a yes on passes only on a click on Yes or a firm spoken yes. A
        // spoken position word ("one", "first", "last") is neither yes nor no there.
        if let Some(w) = waiting.as_ref().filter(|w| w.kind.needs_yes()) {
            let answer = match (clicked, quick.clone()) {
                (true, Some(Quick::Pick(0))) => Answer::Yes,
                (true, Some(Quick::Pick(_))) => Answer::No,
                (false, _) if firm_yes(&heard) => Answer::Yes,
                (false, Some(Quick::No)) => Answer::No,
                (false, Some(Quick::Yes)) if !w.retried => Answer::Unclear,
                (false, Some(Quick::Yes)) => Answer::GiveUp,
                _ => Answer::Other,
            };
            match answer {
                Answer::Yes => {
                    let Some(deciding) = self.take_for_yes(o) else {
                        o.end(Then::Rest);
                        return "gone before the yes";
                    };
                    let said = self
                        .yes(rt, w.kind.clone(), &heard, view, o, cancel, deciding)
                        .await;
                    drop(gate);
                    return said;
                }
                Answer::No | Answer::GiveUp => {
                    self.drop_pending(o);
                    let said = match (&answer, &w.kind) {
                        (Answer::GiveUp, _) => WONT,
                        (_, Waiting::Create { .. }) => NOT_CREATED,
                        (_, Waiting::Ask { .. }) => NOT_SENT,
                        _ => WONT,
                    };
                    self.finish(o, &heard, said, Then::Rest);
                    return "declined";
                }
                // Asked once more; a second unclear yes drops the step.
                Answer::Unclear => {
                    self.retry(o);
                    self.finish(o, &heard, CLEAR_YES, Then::Ask);
                    return "unclear yes";
                }
                // Anything else drops the step; the words go on as a new turn.
                Answer::Other => self.drop_pending(o),
            }
        }
        // No deed of a step that waited: a deed of this turn takes the gate again.
        drop(gate);

        // What goes to the model, and the person's own words, when it does.
        let mut to_model = heard.clone();
        let own = heard.clone();
        match waiting.map(|w| w.kind) {
            // answered above
            Some(Waiting::Confirm { .. }) | Some(Waiting::Create { .. }) => {}
            Some(Waiting::Ask {
                options,
                then,
                dot_ids,
                ..
            }) => {
                let index = match quick {
                    Some(Quick::Pick(i)) => Some(i),
                    Some(Quick::Yes) => options.iter().position(|x| is_yes(x)),
                    Some(Quick::No) => options.iter().position(|x| is_no(x)),
                    _ => None,
                };
                self.drop_pending(o);
                if let Some(i) = index {
                    match then {
                        AskFor::Dot { wish, .. } => {
                            return self
                                .chosen_dot(rt, &wish, &dot_ids[i], &heard, view, o, cancel)
                                .await;
                        }
                        // answered above
                        AskFor::AnswerCheck { .. } => {}
                        // The model asked: its next turn reads the answer.
                        AskFor::Model => to_model = format!("The person chose: {}", options[i]),
                    }
                }
            }
            Some(Waiting::SetupQuestion {
                id,
                description,
                rounds,
                questions,
                mut answers,
            }) => {
                answers.push(heard.clone());
                if answers.len() < questions.len() {
                    let next = questions[answers.len()].clone();
                    self.set_pending(
                        o,
                        Waiting::SetupQuestion {
                            id,
                            description,
                            rounds,
                            questions,
                            answers,
                        },
                        view,
                    );
                    o.ask(&next, Vec::new());
                    o.end(Then::Ask);
                    self.record(o, &heard, Then::Ask);
                    return "setup question";
                }
                self.drop_pending(o);
                let mut rounds = rounds;
                rounds.push(SetupRound { questions, answers });
                o.send(MindEvent::Thinking);
                let req = SetupRequest {
                    description,
                    rounds,
                    current: None,
                    change: None,
                    skip: false,
                };
                return self.setup(rt, id, req, &heard, view, o, cancel).await;
            }
            None => {
                if matches!(quick, Some(Quick::Yes) | Some(Quick::No)) {
                    self.finish(o, &heard, NOTHING_WAITING, Then::Rest);
                    return "nothing waiting";
                }
            }
        }
        self.think(rt, &to_model, &own, view, o, cancel).await
    }

    /// A model turn: the caps, the picture, the request, then the wish.
    async fn think(
        &self,
        rt: &Runtime,
        heard: &str,
        own: &str,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
    ) -> &'static str {
        let settings = rt.settings().await;
        if let Ok(Some(said)) = cap_hit(rt.store(), &settings, chrono::Utc::now()).await {
            self.finish(o, heard, &said, Then::Rest);
            return "cap";
        }
        o.send(MindEvent::Thinking);
        let picture = match Picture::take(rt, view).await {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!("open dot could not take the picture");
                self.finish(o, heard, WENT_WRONG, Then::Rest);
                return "picture failed";
            }
        };
        let turns = lock(&self.state).turns.clone();
        let asking = prompt(&picture, &turns, heard);
        let model = settings.open_dot_model.clone();
        let mut gate = SpeechGate::new();
        let asked = {
            let mut on_text = |delta: &str| {
                let words = gate.push(delta);
                o.stream(&words);
            };
            let request = self
                .thinker
                .think(INSTRUCTIONS, &asking, &model, cancel, &mut on_text);
            tokio::pin!(request);
            let filler = tokio::time::sleep(self.filler_after);
            tokio::pin!(filler);
            let mut waited = false;
            loop {
                tokio::select! {
                    asked = &mut request => break asked,
                    _ = &mut filler, if !waited => {
                        waited = true;
                        if !o.spoke() {
                            o.send(MindEvent::Filler { text: FILLER.into() });
                        }
                    }
                }
            }
        };
        let answer = match asked {
            Ok(answer) => answer,
            Err(failure) => {
                // The CLI's own text is never said, sent or logged.
                let said = match failure {
                    OnceFailure::NoCli => NO_CLI,
                    OnceFailure::TimedOut => TIMED_OUT,
                    OnceFailure::Failed(_) => FAILED,
                    OnceFailure::Cancelled => "",
                };
                self.finish(o, heard, said, Then::Rest);
                return "failed";
            }
        };
        if let Some(usage) = &answer.usage {
            if rt.store().add_open_dot_usage(&model, usage).await.is_err() {
                tracing::warn!("open dot usage could not be recorded");
            }
        }
        // What the stream held back and turned out to be speech; never the words twice.
        o.stream(&gate.finish());
        let (reply, bad) = read_reply(&answer.text);
        let Some(wish) = reply.wish else {
            if bad && reply.words.is_empty() {
                self.finish(o, heard, DIDNT_CATCH, Then::Rest);
                return "unreadable";
            }
            let then = if o.spoke() {
                Then::FollowUp
            } else {
                Then::Rest
            };
            o.end(then);
            self.record(o, heard, then);
            return "words";
        };
        if wish == Wish::Needs {
            return self.needs(&picture, heard, o);
        }
        if let Wish::ReadQuestion { q } = &wish {
            if let Some(line) = picture.question(q) {
                lock(&o.read_aloud).push(line.id.clone());
            }
        }
        let step = plan(&wish, &picture, view);
        self.act(rt, step, &picture, heard, own, view, o, cancel)
            .await
    }

    /// What needs the person: said, listed on a card, and the Inbox opened for private ones.
    fn needs(&self, picture: &Picture, heard: &str, o: &Out<'_>) -> &'static str {
        o.say(&act::needs_report(picture));
        let mut lines: Vec<String> = picture
            .questions
            .iter()
            .map(|q| format!("{} asks: {}", spoken_name(&q.dot_name), q.text))
            .chain(picture.approvals.iter().map(|a| {
                let mut line = format!("{} wants to use {}", spoken_name(&a.dot_name), a.tool);
                if !a.target.is_empty() {
                    line.push_str(&format!(": {}", a.target));
                }
                line
            }))
            .collect();
        let more = picture.more_questions + picture.more_approvals;
        if more > 0 {
            lines.push(format!("And {more} more."));
        }
        if picture.private_approvals > 0 {
            lines.push(format!(
                "{} private, in the Inbox",
                picture.private_approvals
            ));
        }
        o.send(MindEvent::Card {
            card: Card::List {
                title: NEEDS_TITLE.into(),
                lines,
            },
        });
        if picture.private_approvals > 0 {
            o.send(MindEvent::Open {
                target: OpenTarget::Inbox,
            });
        }
        o.end(Then::FollowUp);
        self.record(o, heard, Then::FollowUp);
        "needs"
    }

    /// Carries out or asks about the planned step.
    #[allow(clippy::too_many_arguments)]
    async fn act(
        &self,
        rt: &Runtime,
        step: Step,
        picture: &Picture,
        heard: &str,
        own: &str,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
    ) -> &'static str {
        match step {
            Step::Say(said) => {
                self.finish(o, heard, &said, Then::FollowUp);
                "say"
            }
            Step::Do(deed) => {
                if needs_confirmation(&deed) {
                    // `plan` never does this; a deed that must be confirmed never runs here.
                    self.finish(o, heard, DIDNT_CATCH, Then::Rest);
                    return "refused";
                }
                if !points_at(&deed, own, view, picture, &self.read_aloud_before()) {
                    let question = check_question(&deed);
                    self.confirm(o, deed, question, false, heard, view);
                    return "confirm";
                }
                // Never after the person moved on: a newer turn or a cancel ends this one first.
                let _gate = self.deeds.lock().await;
                if !o.may_act(cancel) {
                    o.end(Then::Rest);
                    return "ended before the deed";
                }
                let done = outcome(rt, &deed).await;
                o.deed_done();
                let then = if done.done {
                    Then::FollowUp
                } else {
                    Then::Rest
                };
                self.finish(o, heard, &done.said, then);
                "do"
            }
            Step::Confirm {
                deed,
                question,
                on_screen,
            } => {
                self.confirm(o, deed, question, on_screen, heard, view);
                "confirm"
            }
            Step::Ask {
                question,
                options,
                then,
            } => {
                let dot_ids = match &then {
                    AskFor::Dot { handles, .. } => handles
                        .iter()
                        .map(|h| picture.dot(h).map(|d| d.id.clone()).unwrap_or_default())
                        .collect(),
                    _ => Vec::new(),
                };
                o.ask(&question, options.clone());
                self.set_pending(
                    o,
                    Waiting::Ask {
                        question,
                        options,
                        then,
                        dot_ids,
                    },
                    view,
                );
                o.end(Then::Ask);
                self.record(o, heard, Then::Ask);
                "ask"
            }
            Step::Open(target, said) => {
                o.send(MindEvent::Open { target });
                self.finish(o, heard, &said, Then::FollowUp);
                "open"
            }
            Step::Create { description } => {
                o.say(SETTING_UP);
                let req = SetupRequest {
                    description,
                    rounds: Vec::new(),
                    current: None,
                    change: None,
                    skip: false,
                };
                let id = crate::util::new_id();
                self.setup(rt, id, req, heard, view, o, cancel).await
            }
        }
    }

    fn read_aloud_before(&self) -> Vec<String> {
        lock(&self.state).read_aloud.clone()
    }

    /// Shows the deed's card and asks Dots' own question; the deed waits for a yes.
    fn confirm(
        &self,
        o: &Out<'_>,
        deed: Deed,
        question: String,
        on_screen: bool,
        heard: &str,
        view: &View,
    ) {
        o.send(MindEvent::Card {
            card: Card::Confirm { deed: deed.clone() },
        });
        if on_screen {
            if let Deed::Approve { approval_id, .. } | Deed::Deny { approval_id, .. } = &deed {
                o.send(MindEvent::Open {
                    target: OpenTarget::Approval {
                        approval_id: approval_id.clone(),
                    },
                });
            }
        }
        o.ask(&question, yes_no());
        self.set_pending(o, Waiting::Confirm { deed, question }, view);
        o.end(Then::Ask);
        self.record(o, heard, Then::Ask);
    }

    /// The person chose a dot for a wish: planned again on a fresh picture, where the confirm
    /// tier still applies.
    #[allow(clippy::too_many_arguments)]
    async fn chosen_dot(
        &self,
        rt: &Runtime,
        wish: &Wish,
        dot_id: &str,
        heard: &str,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
    ) -> &'static str {
        let picture = match Picture::take(rt, view).await {
            Ok(p) => p,
            Err(_) => {
                self.finish(o, heard, WENT_WRONG, Then::Rest);
                return "picture failed";
            }
        };
        let Some(handle) = picture
            .dots
            .iter()
            .find(|d| d.id == dot_id)
            .map(|d| d.handle.clone())
        else {
            self.finish(o, heard, DOT_GONE, Then::Rest);
            return "dot gone";
        };
        let wish = match wish.clone() {
            Wish::Run { .. } => Wish::Run { d: handle },
            Wish::Send { text, here, .. } => Wish::Send {
                d: handle,
                text,
                here,
            },
            other => other,
        };
        let step = plan(&wish, &picture, view);
        // Choosing the dot by name is pointing at it.
        let own = picture
            .dot(&match &wish {
                Wish::Run { d } | Wish::Send { d, .. } => d.clone(),
                _ => String::new(),
            })
            .map(|d| d.name.clone())
            .unwrap_or_default();
        self.act(rt, step, &picture, heard, &own, view, o, cancel)
            .await
    }

    /// Asks the setup agent and goes on with its questions or its proposal.
    #[allow(clippy::too_many_arguments)]
    async fn setup(
        &self,
        rt: &Runtime,
        id: String,
        req: SetupRequest,
        heard: &str,
        view: &View,
        o: &Out<'_>,
        cancel: &CancellationToken,
    ) -> &'static str {
        let asked = tokio::select! {
            asked = rt.setup_ask(&id, req.clone()) => asked,
            _ = cancel.cancelled() => {
                rt.setup_cancel(&id);
                o.end(Then::Rest);
                return "setup cancelled";
            }
        };
        match asked {
            Ok(SetupAnswer::Questions { questions }) if !questions.is_empty() => {
                let first = questions[0].clone();
                self.set_pending(
                    o,
                    Waiting::SetupQuestion {
                        id,
                        description: req.description,
                        rounds: req.rounds,
                        questions,
                        answers: Vec::new(),
                    },
                    view,
                );
                o.ask(&first, Vec::new());
                o.end(Then::Ask);
                self.record(o, heard, Then::Ask);
                "setup question"
            }
            Ok(SetupAnswer::Questions { .. }) => {
                self.finish(o, heard, SETUP_FAILED, Then::Rest);
                "setup failed"
            }
            Ok(SetupAnswer::Proposal { proposal, .. }) => {
                o.send(MindEvent::Card {
                    card: Card::Proposal {
                        proposal: proposal.clone(),
                    },
                });
                let question = create_question(&proposal);
                o.ask(&question, yes_no());
                self.set_pending(o, Waiting::Create { proposal, question }, view);
                o.end(Then::Ask);
                self.record(o, heard, Then::Ask);
                "proposal"
            }
            Err(e) => {
                self.finish(o, heard, &error_said(e), Then::Rest);
                "setup failed"
            }
        }
    }
}

/// Carries out a deed the person said yes to, once a fresh picture shows its record is still
/// there: the approval still waits and is not private, the dot is still in this workspace, the
/// question still waits. `None`, and nothing done, when `may_act` says the turn ended first.
async fn confirmed(
    rt: &Runtime,
    deed: &Deed,
    view: &View,
    may_act: &dyn Fn() -> bool,
) -> Option<Outcome> {
    let Ok(picture) = Picture::take(rt, view).await else {
        return Some(Outcome::refused(WENT_WRONG));
    };
    if !may_act() {
        return None;
    }
    let still = match deed {
        Deed::Approve { approval_id, .. } | Deed::Deny { approval_id, .. } => {
            picture.approvals.iter().any(|a| &a.id == approval_id)
        }
        Deed::Run { dot_id, .. } | Deed::Send { dot_id, .. } => {
            picture.dots.iter().any(|d| &d.id == dot_id)
        }
        Deed::Answer { message_id, .. } => picture.questions.iter().any(|q| &q.id == message_id),
    };
    if !still {
        return Some(Outcome::refused(match deed {
            Deed::Run { .. } | Deed::Send { .. } => DOT_GONE,
            _ => HANDLED,
        }));
    }
    Some(outcome(rt, deed).await)
}

/// While alive, a step taken by a yes is being decided (`State::deciding`).
struct Deciding<'a>(&'a Mind);

impl Drop for Deciding<'_> {
    fn drop(&mut self) {
        lock(&self.0.state).deciding = false;
    }
}

/// Creates the proposed dot in the account and workspace in view; the sentence, the screen to
/// open and how the turn ends.
async fn create(
    rt: &Runtime,
    proposal: SetupProposal,
    view: &View,
) -> (String, Option<OpenTarget>, Then) {
    let org = (view.account != crate::accounts::PERSONAL && !view.account.is_empty())
        .then(|| view.account.clone());
    let workspace = (!view.workspace.is_empty()).then(|| view.workspace.clone());
    match rt.setup_apply(proposal, org, workspace).await {
        Ok(applied) => {
            let mut said = format!("Created {}.", spoken_name(&applied.dot.spec.name));
            if let Some(e) = &applied.run_error {
                let why = plain_error(e);
                if !why.is_empty() {
                    said.push_str(&format!(
                        " Its first run could not start: {}",
                        sentence(&why)
                    ));
                }
            }
            (
                said,
                Some(OpenTarget::Dot {
                    dot_id: applied.dot.id,
                }),
                Then::FollowUp,
            )
        }
        Err(e) => (error_said(e), None, Then::Rest),
    }
}

/// Whether the person's own words this turn point at the deed's target: a word of 3 or more
/// letters of the dot's name, the dot on screen, or, for an answer, a question read aloud in the
/// turn before or the only one waiting. Otherwise Dots asks before sending.
fn points_at(
    deed: &Deed,
    own: &str,
    view: &View,
    picture: &Picture,
    read_aloud: &[String],
) -> bool {
    let heard = words_of(own);
    let named = |name: &str| {
        words_of(name)
            .iter()
            .any(|w| w.chars().count() >= 3 && heard.contains(w))
    };
    let on_screen = |dot_id: &str| view.dot_id.as_deref() == Some(dot_id);
    match deed {
        Deed::Send {
            dot_id, dot_name, ..
        } => named(dot_name) || on_screen(dot_id),
        Deed::Answer {
            message_id,
            dot_name,
            ..
        } => {
            let question = picture.questions.iter().find(|q| &q.id == message_id);
            named(dot_name)
                || question.is_some_and(|q| on_screen(&q.dot_id))
                || read_aloud.contains(message_id)
                || (picture.questions.len() == 1 && picture.more_questions == 0)
        }
        _ => false,
    }
}

/// Dots' own question before a send or answer the person did not point at.
fn check_question(deed: &Deed) -> String {
    match deed {
        Deed::Send { dot_name, text, .. } => {
            format!(
                "Send to {}: {}?",
                spoken_name(dot_name),
                text.trim_end_matches('.')
            )
        }
        Deed::Answer { dot_name, text, .. } => format!(
            "Answer {} with: {}?",
            spoken_name(dot_name),
            text.trim_end_matches('.')
        ),
        _ => String::new(),
    }
}

/// "<name>: <when it runs, and its first message>. Create it?"
fn create_question(p: &SetupProposal) -> String {
    let mut line = format!("{}: it {}", spoken_name(&p.name), runs(p.schedule.as_ref()));
    let first = clean(&p.first_message, FIRST_MESSAGE_BYTES);
    let first = first.trim_end_matches(['.', '!', '?']);
    if !first.is_empty() {
        line.push_str(&format!(", and starts with: {first}"));
    }
    format!("{line}. Create it?")
}

/// When a schedule runs, in a few words.
fn runs(schedule: Option<&Schedule>) -> String {
    let Some(s) = schedule else {
        return "runs when you ask".into();
    };
    match &s.rule {
        Rule::Once { at } => format!("runs once, on {}", at.format("%Y-%m-%d at %H:%M")),
        Rule::Every { minutes, .. } if minutes % 60 == 0 && *minutes >= 60 => match minutes / 60 {
            1 => "runs every hour".into(),
            h => format!("runs every {h} hours"),
        },
        Rule::Every { minutes: 1, .. } => "runs every minute".into(),
        Rule::Every { minutes, .. } => format!("runs every {minutes} minutes"),
        Rule::Daily { time } => format!("runs every day at {}", time.format("%H:%M")),
        Rule::Weekly { days, time } => format!(
            "runs on {} at {}",
            days.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            time.format("%H:%M")
        ),
        Rule::Cron { .. } => "runs on a custom schedule".into(),
    }
}

/// The text of a refusal or failure, as one plain sentence.
fn error_said(e: Error) -> String {
    let text = match e {
        Error::Invalid(m)
        | Error::Other(m)
        | Error::Conflict(m)
        | Error::NotAllowed(m)
        | Error::NotFound(m) => m,
        _ => String::new(),
    };
    // The CLI's own words after this are not said.
    const CLI_FAILED: &str = "Claude could not set up the dot";
    let text = if text.starts_with(CLI_FAILED) {
        CLI_FAILED.to_string()
    } else {
        text
    };
    let plain = plain_error(&text);
    let mut chars = plain.chars();
    match chars.next() {
        None => SETUP_FAILED.into(),
        Some(first) => sentence(&format!("{}{}", first.to_uppercase(), chars.as_str())),
    }
}

/// `s` ending in a sentence mark.
fn sentence(s: &str) -> String {
    let s = s.trim_end();
    if s.ends_with(['.', '?', '!']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// A dot name for speech, on one line.
fn spoken_name(name: &str) -> String {
    clean(name, NAME_BYTES)
}

fn is_yes(option: &str) -> bool {
    option.trim().eq_ignore_ascii_case("yes")
}

fn is_no(option: &str) -> bool {
    option.trim().eq_ignore_ascii_case("no")
}

#[cfg(test)]
mod tests;
