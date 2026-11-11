//! The bridge: the voice session's events go to Open Dot's mind, and the mind's events go to
//! the session (to be spoken) and to the UI (as `voice` events).
//!
//! One turn of the session (a `Heard`, spoken or clicked) is one call to the mind. The newest
//! `Heard` is the live turn: an older turn's remaining events are dropped here, and no session
//! call ever carries an older turn's id. A click on an option reaches the mind as `pick`,
//! spoken words as `hear`, never the other way round.
//!
//! Nothing heard or said is logged.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use dots_core::open_dot::picture::View;
use dots_core::open_dot::{MindEvent, Then};
use dots_voice::{After, Session, SessionEvent, State};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use super::events::VoiceEvent;

/// What the bridge needs of the voice session.
pub trait SessionPort: Send + Sync {
    fn open(&self);
    fn close(&self);
    fn interrupt(&self);
    fn thinking(&self);
    fn say(&self, turn: u64, text: &str, then: After);
    fn say_delta(&self, turn: u64, delta: &str);
    fn end_of_reply(&self, turn: u64, then: After);
    fn filler(&self, turn: u64, text: &str);
    /// A click on an option: the new turn's id, or `None` when the session is off.
    fn answered(&self, text: &str) -> Option<u64>;
    fn state(&self) -> State;
}

impl SessionPort for Session {
    fn open(&self) {
        Session::open(self);
    }
    fn close(&self) {
        Session::close(self);
    }
    fn interrupt(&self) {
        Session::interrupt(self);
    }
    fn thinking(&self) {
        Session::thinking(self);
    }
    fn say(&self, turn: u64, text: &str, then: After) {
        Session::say(self, turn, text, then);
    }
    fn say_delta(&self, turn: u64, delta: &str) {
        Session::say_delta(self, turn, delta);
    }
    fn end_of_reply(&self, turn: u64, then: After) {
        Session::end_of_reply(self, turn, then);
    }
    fn filler(&self, turn: u64, text: &str) {
        Session::filler(self, turn, text);
    }
    fn answered(&self, text: &str) -> Option<u64> {
        Session::answered(self, text)
    }
    fn state(&self) -> State {
        Session::state(self)
    }
}

/// What the bridge needs of Open Dot's mind (`dots_core::open_dot::Mind` over the runtime).
#[async_trait]
pub trait MindPort: Send + Sync {
    async fn hear(&self, heard: &str, view: &View, out: &UnboundedSender<MindEvent>);
    async fn pick(&self, index: usize, view: &View, out: &UnboundedSender<MindEvent>);
    fn lapse(&self, out: &UnboundedSender<MindEvent>);
    fn cancel(&self);
    /// True when a step that waited on the person was dropped.
    fn reset(&self) -> bool;
    fn pending(&self) -> bool;
}

/// How a reply ends, in the session's terms.
pub fn after(then: Then) -> After {
    match then {
        Then::Rest => After::Rest,
        Then::FollowUp => After::FollowUp,
        Then::Ask => After::Ask,
    }
}

/// What a turn brings to the mind.
enum Input {
    Hear(String),
    Pick(usize),
}

#[derive(Default)]
struct Inner {
    view: View,
    /// The turn whose mind events are applied; `None` once it ended (or was ended).
    live: Option<u64>,
    /// Turns made by a click, and the option clicked.
    picks: HashMap<u64, usize>,
    /// The options of the question showing, while they may be clicked.
    options: Vec<String>,
    /// A question was asked and has not been cleared.
    asking: bool,
    /// Options or a card are showing.
    showing: bool,
    /// What was spoken this turn ends in a word: a question spoken next needs a space.
    gap: bool,
}

impl Inner {
    fn forget(&mut self) {
        self.options.clear();
        self.asking = false;
        self.showing = false;
    }
}

pub struct Bridge {
    session: Arc<dyn SessionPort>,
    mind: Arc<dyn MindPort>,
    ui: UnboundedSender<VoiceEvent>,
    inner: Mutex<Inner>,
}

impl Bridge {
    pub fn new(
        session: Arc<dyn SessionPort>,
        mind: Arc<dyn MindPort>,
        ui: UnboundedSender<VoiceEvent>,
    ) -> Arc<Bridge> {
        Arc::new(Bridge {
            session,
            mind,
            ui,
            inner: Mutex::new(Inner::default()),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, event: VoiceEvent) {
        // A closed receiver only means the UI is gone.
        let _ = self.ui.send(event);
    }

    /// Every session event, in the order the session sent them. Must be called within a tokio
    /// runtime: a `Heard` starts the mind's turn as a task.
    pub fn on_session(self: &Arc<Self>, ev: SessionEvent) {
        match ev {
            SessionEvent::Heard { text, turn } => self.heard(text, turn),
            SessionEvent::State { state, mic_open } => {
                self.send(VoiceEvent::State { state, mic_open });
                if state == State::Idle {
                    self.idle();
                }
            }
            SessionEvent::Level { mic, voice } => self.send(VoiceEvent::Level { mic, voice }),
            SessionEvent::Hearing { text } => self.send(VoiceEvent::Hearing { text }),
            SessionEvent::Saying { text } => self.send(VoiceEvent::Saying { text }),
            // The session is listening already.
            SessionEvent::Woke => self.send(VoiceEvent::Woke),
            SessionEvent::Trouble { message } => self.send(VoiceEvent::Trouble { message }),
        }
    }

    /// The account, workspace and screen the next turns see.
    pub fn set_view(&self, view: View) {
        self.lock().view = view;
    }

    /// Hotkey, rail, tray: listen.
    pub fn open(&self) {
        self.session.open();
    }

    /// Esc: stop everything, drop what waits, forget the conversation.
    pub fn close(&self) {
        let mut s = self.lock();
        self.session.close();
        self.mind.cancel();
        let lapsed = self.lapse();
        let dropped = self.mind.reset();
        let showing = s.showing || s.asking;
        s.live = None;
        s.picks.clear();
        s.forget();
        if showing || lapsed || dropped {
            self.send(VoiceEvent::Cleared);
        }
    }

    /// Stop speaking (or thinking) and listen.
    pub fn stop(&self) {
        self.mind.cancel();
        self.session.interrupt();
    }

    /// A click (or key) on option `index` of the question showing. Only the first click on a
    /// question counts: the options are forgotten until the next question.
    pub fn pick(&self, index: usize) {
        // Held across `answered`, so the clicked turn's `Heard` finds its index.
        let mut s = self.lock();
        let Some(text) = s.options.get(index).cloned() else {
            return;
        };
        if let Some(turn) = self.session.answered(&text) {
            s.options.clear();
            s.picks.insert(turn, index);
        }
    }

    /// A finished sentence (or a click) is the live turn from now on.
    fn heard(self: &Arc<Self>, text: String, turn: u64) {
        let (input, view) = {
            let mut s = self.lock();
            s.live = Some(turn);
            s.gap = false;
            self.session.thinking();
            self.send(VoiceEvent::Heard { text: text.clone() });
            let input = match s.picks.remove(&turn) {
                Some(index) => Input::Pick(index),
                None => Input::Hear(text),
            };
            (input, s.view.clone())
        };
        self.start(turn, input, view);
    }

    /// The session rested: a step still waiting has lapsed (its asking time ran out, or the
    /// session was closed).
    fn idle(&self) {
        let mut s = self.lock();
        if !self.mind.pending() {
            return;
        }
        self.lapse();
        s.live = None;
        s.forget();
        self.send(VoiceEvent::Cleared);
    }

    /// `mind.lapse`; whether it dropped a step that waited.
    fn lapse(&self) -> bool {
        let (tx, mut rx) = unbounded_channel();
        self.mind.lapse(&tx);
        let mut cleared = false;
        while let Ok(ev) = rx.try_recv() {
            cleared |= ev == MindEvent::Cleared;
        }
        cleared
    }

    /// One mind event of `turn`: applied only while `turn` is the live turn. The lock is held
    /// while the session and the UI are told, so a newer `Heard` cannot come between.
    fn apply(&self, turn: u64, ev: MindEvent) {
        let mut s = self.lock();
        if let MindEvent::Did { said } = ev {
            self.did(&mut s, turn, said);
            return;
        }
        if s.live != Some(turn) {
            return;
        }
        match ev {
            // The session is Thinking since the `Heard`.
            MindEvent::Thinking => {}
            MindEvent::Filler { text } => self.session.filler(turn, &text),
            MindEvent::Say { delta } => {
                if !delta.is_empty() {
                    s.gap = !delta.ends_with(char::is_whitespace);
                    self.session.say_delta(turn, &delta);
                }
            }
            MindEvent::Ask { question, options } => {
                s.options = options.clone();
                s.asking = true;
                s.showing = true;
                // The mind sends the question only here: it is spoken once, after the words.
                let spoken = question.trim();
                if !spoken.is_empty() {
                    let spoken = if s.gap {
                        format!(" {spoken}")
                    } else {
                        spoken.to_string()
                    };
                    s.gap = true;
                    self.session.say_delta(turn, &spoken);
                }
                self.send(VoiceEvent::Ask { question, options });
            }
            MindEvent::SayEnd { then } => self.end(&mut s, turn, after(then)),
            MindEvent::Card { card } => {
                s.showing = true;
                self.send(VoiceEvent::Card { card });
            }
            MindEvent::Open { target } => self.send(VoiceEvent::Open { target }),
            MindEvent::Cleared => {
                s.forget();
                self.send(VoiceEvent::Cleared);
            }
            MindEvent::Did { .. } => {}
        }
    }

    /// A deed of `turn` was carried out. For the live turn its outcome is spoken through
    /// `Say`. A stale turn's outcome would be lost with the turn: it is spoken on the live
    /// turn, or, with none, shown only.
    fn did(&self, s: &mut Inner, turn: u64, said: String) {
        if s.live == Some(turn) || said.is_empty() {
            return;
        }
        if let Some(live) = s.live {
            self.session.say_delta(live, &format!("{said} "));
            s.gap = false;
        }
        self.send(VoiceEvent::Saying { text: said });
    }

    /// Ends the live turn at the session. A question still showing whose step the mind no
    /// longer holds (an older turn dropped it, and its `Cleared` was dropped as stale) goes.
    fn end(&self, s: &mut Inner, turn: u64, then: After) {
        self.session.end_of_reply(turn, then);
        s.live = None;
        if s.asking && !self.mind.pending() {
            s.forget();
            self.send(VoiceEvent::Cleared);
        }
    }

    /// The mind's call for `turn` returned. A turn the mind made nothing of (a click on an
    /// option no longer offered, a second yes) sends no `SayEnd`: it is ended here, so the
    /// session does not wait for a reply.
    fn finished(&self, turn: u64) {
        let mut s = self.lock();
        if s.live == Some(turn) {
            self.end(&mut s, turn, After::Rest);
        }
    }

    fn start(self: &Arc<Self>, turn: u64, input: Input, view: View) {
        let bridge = Arc::clone(self);
        tokio::spawn(async move {
            let (tx, mut rx) = unbounded_channel();
            let mind = bridge.mind.clone();
            // Owns `tx`: once the call returns, the channel closes and the loop below ends.
            let call = async move {
                match input {
                    Input::Hear(text) => mind.hear(&text, &view, &tx).await,
                    Input::Pick(index) => mind.pick(index, &view, &tx).await,
                }
            };
            let apply = async {
                while let Some(ev) = rx.recv().await {
                    bridge.apply(turn, ev);
                }
            };
            tokio::join!(call, apply);
            bridge.finished(turn);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dots_core::open_dot::act::OpenTarget;
    use dots_core::open_dot::Card;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use tokio::sync::mpsc::UnboundedReceiver;
    use tokio::sync::Notify;

    // ---- fakes ---------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Open,
        Close,
        Interrupt,
        Thinking,
        Say(u64, String, After),
        Delta(u64, String),
        End(u64, After),
        Filler(u64, String),
        Answered(String),
    }

    impl Call {
        fn turn(&self) -> Option<u64> {
            match self {
                Call::Say(t, ..) | Call::Delta(t, _) | Call::End(t, _) | Call::Filler(t, _) => {
                    Some(*t)
                }
                _ => None,
            }
        }
    }

    struct FakeSession {
        calls: Mutex<Vec<Call>>,
        /// The last turn id handed out by `answered`.
        last: AtomicU64,
        off: AtomicBool,
    }

    impl FakeSession {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::default(),
                last: AtomicU64::new(100),
                off: AtomicBool::new(false),
            })
        }
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
        fn push(&self, call: Call) {
            self.calls.lock().unwrap().push(call);
        }
    }

    impl SessionPort for FakeSession {
        fn open(&self) {
            self.push(Call::Open);
        }
        fn close(&self) {
            self.push(Call::Close);
        }
        fn interrupt(&self) {
            self.push(Call::Interrupt);
        }
        fn thinking(&self) {
            self.push(Call::Thinking);
        }
        fn say(&self, turn: u64, text: &str, then: After) {
            self.push(Call::Say(turn, text.into(), then));
        }
        fn say_delta(&self, turn: u64, delta: &str) {
            self.push(Call::Delta(turn, delta.into()));
        }
        fn end_of_reply(&self, turn: u64, then: After) {
            self.push(Call::End(turn, then));
        }
        fn filler(&self, turn: u64, text: &str) {
            self.push(Call::Filler(turn, text.into()));
        }
        fn answered(&self, text: &str) -> Option<u64> {
            if self.off.load(Ordering::SeqCst) {
                return None;
            }
            self.push(Call::Answered(text.into()));
            Some(self.last.fetch_add(1, Ordering::SeqCst) + 1)
        }
        fn state(&self) -> State {
            State::Idle
        }
    }

    /// One step of a scripted mind turn.
    #[allow(clippy::large_enum_variant)]
    enum Step {
        Send(MindEvent),
        /// Waits until the test notifies.
        Wait(Arc<Notify>),
        /// Sets what `pending` answers.
        Pending(bool),
    }

    #[derive(Debug, Clone, PartialEq)]
    enum MindCall {
        Hear(String),
        Pick(usize),
        Lapse,
        Cancel,
        Reset,
    }

    #[derive(Default)]
    struct FakeMind {
        calls: Mutex<Vec<MindCall>>,
        /// One script per `hear` or `pick`, in order; none sends nothing.
        scripts: Mutex<VecDeque<Vec<Step>>>,
        pending: AtomicBool,
        reset_drops: AtomicBool,
    }

    impl FakeMind {
        fn script(&self, steps: Vec<Step>) {
            self.scripts.lock().unwrap().push_back(steps);
        }
        fn calls(&self) -> Vec<MindCall> {
            self.calls.lock().unwrap().clone()
        }
        fn push(&self, call: MindCall) {
            self.calls.lock().unwrap().push(call);
        }
        async fn run(&self, out: &UnboundedSender<MindEvent>) {
            let steps = self.scripts.lock().unwrap().pop_front().unwrap_or_default();
            for step in steps {
                match step {
                    Step::Send(ev) => {
                        let _ = out.send(ev);
                    }
                    Step::Wait(n) => n.notified().await,
                    Step::Pending(p) => self.pending.store(p, Ordering::SeqCst),
                }
            }
        }
    }

    #[async_trait]
    impl MindPort for FakeMind {
        async fn hear(&self, heard: &str, _view: &View, out: &UnboundedSender<MindEvent>) {
            self.push(MindCall::Hear(heard.into()));
            self.run(out).await;
        }
        async fn pick(&self, index: usize, _view: &View, out: &UnboundedSender<MindEvent>) {
            self.push(MindCall::Pick(index));
            self.run(out).await;
        }
        fn lapse(&self, out: &UnboundedSender<MindEvent>) {
            self.push(MindCall::Lapse);
            if self.pending.swap(false, Ordering::SeqCst) {
                let _ = out.send(MindEvent::Cleared);
            }
        }
        fn cancel(&self) {
            self.push(MindCall::Cancel);
        }
        fn reset(&self) -> bool {
            self.push(MindCall::Reset);
            self.pending.store(false, Ordering::SeqCst);
            self.reset_drops.load(Ordering::SeqCst)
        }
        fn pending(&self) -> bool {
            self.pending.load(Ordering::SeqCst)
        }
    }

    // ---- the rig -------------------------------------------------------------------------

    struct Rig {
        bridge: Arc<Bridge>,
        session: Arc<FakeSession>,
        mind: Arc<FakeMind>,
        ui: UnboundedReceiver<VoiceEvent>,
    }

    fn rig() -> Rig {
        let session = FakeSession::new();
        let mind = Arc::new(FakeMind::default());
        let (tx, ui) = unbounded_channel();
        let bridge = Bridge::new(session.clone(), mind.clone(), tx);
        Rig {
            bridge,
            session,
            mind,
            ui,
        }
    }

    impl Rig {
        fn ui(&mut self) -> Vec<VoiceEvent> {
            let mut out = Vec::new();
            while let Ok(ev) = self.ui.try_recv() {
                out.push(ev);
            }
            out
        }
        fn heard(&self, text: &str, turn: u64) {
            self.bridge.on_session(SessionEvent::Heard {
                text: text.into(),
                turn,
            });
        }
    }

    /// Lets every spawned turn run as far as it can (single-threaded test runtime).
    async fn settle() {
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }
    }

    fn say(delta: &str) -> Step {
        Step::Send(MindEvent::Say {
            delta: delta.into(),
        })
    }

    fn end(then: Then) -> Step {
        Step::Send(MindEvent::SayEnd { then })
    }

    fn ask(question: &str, options: &[&str]) -> Step {
        Step::Send(MindEvent::Ask {
            question: question.into(),
            options: options.iter().map(|o| o.to_string()).collect(),
        })
    }

    fn heard_ev(text: &str) -> VoiceEvent {
        VoiceEvent::Heard { text: text.into() }
    }

    fn ask_ev(question: &str, options: &[&str]) -> VoiceEvent {
        VoiceEvent::Ask {
            question: question.into(),
            options: options.iter().map(|o| o.to_string()).collect(),
        }
    }

    /// Turn 1 asks "Approve it?" with Yes and No, and the step waits.
    async fn asked(r: &mut Rig) {
        r.mind.script(vec![
            ask("Approve it?", &["Yes", "No"]),
            Step::Pending(true),
            end(Then::Ask),
        ]);
        r.heard("approve it", 1);
        settle().await;
        r.ui();
    }

    // ---- tests ---------------------------------------------------------------------------

    #[tokio::test]
    async fn heard_goes_to_hear_and_reply_is_spoken_in_order() {
        let mut r = rig();
        r.mind.script(vec![
            Step::Send(MindEvent::Thinking),
            say("Hello"),
            say(" there."),
            end(Then::FollowUp),
        ]);
        r.heard("hi dot", 1);
        settle().await;
        assert_eq!(r.mind.calls(), vec![MindCall::Hear("hi dot".into())]);
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Delta(1, "Hello".into()),
                Call::Delta(1, " there.".into()),
                Call::End(1, After::FollowUp),
            ]
        );
        assert_eq!(r.ui(), vec![heard_ev("hi dot")]);
    }

    #[tokio::test]
    async fn click_goes_to_pick_not_hear() {
        let mut r = rig();
        asked(&mut r).await;
        r.mind.script(vec![say("Done."), end(Then::FollowUp)]);
        r.bridge.pick(0);
        assert_eq!(
            r.session.calls().last(),
            Some(&Call::Answered("Yes".into()))
        );
        // The session sends the clicked turn's `Heard` with the id `answered` gave.
        r.heard("Yes", 101);
        settle().await;
        assert_eq!(
            r.mind.calls(),
            vec![MindCall::Hear("approve it".into()), MindCall::Pick(0)]
        );
        assert_eq!(
            r.session.calls().last(),
            Some(&Call::End(101, After::FollowUp))
        );
        // Spoken words that match an option still go to `hear`.
        r.heard("Yes", 102);
        settle().await;
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Hear("Yes".into())));
    }

    #[tokio::test]
    async fn a_newer_heard_drops_the_older_turns_events() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            say("one"),
            Step::Wait(gate.clone()),
            say("late"),
            Step::Send(MindEvent::Card {
                card: Card::List {
                    title: "t".into(),
                    lines: vec![],
                },
            }),
            end(Then::Rest),
        ]);
        r.mind.script(vec![say("two"), end(Then::Rest)]);
        r.heard("first", 1);
        settle().await;
        r.heard("second", 2);
        settle().await;
        gate.notify_one();
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Delta(1, "one".into()),
                Call::Thinking,
                Call::Delta(2, "two".into()),
                Call::End(2, After::Rest),
            ]
        );
        assert_eq!(r.ui(), vec![heard_ev("first"), heard_ev("second")]);
    }

    #[tokio::test]
    async fn no_session_call_carries_a_stale_turn() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            Step::Send(MindEvent::Filler {
                text: "One moment.".into(),
            }),
            say("late"),
            ask("Run it?", &["Yes", "No"]),
            end(Then::Ask),
        ]);
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            say("fresh"),
            end(Then::FollowUp),
        ]);
        r.heard("first", 1);
        settle().await;
        r.heard("second", 2);
        settle().await;
        let before = r.session.calls().len();
        // Let the older turn finish first, then the newer one.
        gate.notify_one();
        settle().await;
        gate.notify_one();
        settle().await;
        let calls = r.session.calls();
        assert!(
            calls[before..].iter().all(|c| c.turn() == Some(2)),
            "{calls:?}"
        );
        assert_eq!(calls.last(), Some(&Call::End(2, After::FollowUp)));
        // The stale question never showed, so a click does nothing.
        assert!(!r.ui().iter().any(|e| matches!(e, VoiceEvent::Ask { .. })));
        r.bridge.pick(0);
        assert!(!r
            .session
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Answered(_))));
    }

    #[tokio::test]
    async fn ask_speaks_the_question_and_shows_options() {
        let mut r = rig();
        r.mind.script(vec![
            say("Okay."),
            ask("Approve it?", &["Yes", "No"]),
            Step::Pending(true),
            end(Then::Ask),
        ]);
        r.heard("approve", 1);
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Delta(1, "Okay.".into()),
                Call::Delta(1, " Approve it?".into()),
                Call::End(1, After::Ask),
            ]
        );
        assert_eq!(
            r.ui(),
            vec![heard_ev("approve"), ask_ev("Approve it?", &["Yes", "No"])]
        );
        // The question alone is spoken without a leading space.
        r.mind
            .script(vec![ask("Which dot?", &["Mail", "News"]), end(Then::Ask)]);
        r.heard("run it", 2);
        settle().await;
        assert!(r
            .session
            .calls()
            .contains(&Call::Delta(2, "Which dot?".into())));
    }

    #[tokio::test]
    async fn idle_while_pending_lapses() {
        let mut r = rig();
        asked(&mut r).await;
        r.bridge.on_session(SessionEvent::State {
            state: State::Idle,
            mic_open: false,
        });
        r.bridge.on_session(SessionEvent::State {
            state: State::Idle,
            mic_open: true,
        });
        let lapses = r
            .mind
            .calls()
            .iter()
            .filter(|c| **c == MindCall::Lapse)
            .count();
        assert_eq!(lapses, 1);
        assert_eq!(
            r.ui(),
            vec![
                VoiceEvent::State {
                    state: State::Idle,
                    mic_open: false
                },
                VoiceEvent::Cleared,
                VoiceEvent::State {
                    state: State::Idle,
                    mic_open: true
                },
            ]
        );
        // The options are gone: a click does nothing.
        r.bridge.pick(0);
        assert!(!r
            .session
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Answered(_))));
    }

    #[tokio::test]
    async fn idle_with_nothing_pending_does_not_lapse() {
        let mut r = rig();
        r.bridge.on_session(SessionEvent::State {
            state: State::Idle,
            mic_open: false,
        });
        assert!(r.mind.calls().is_empty());
        assert_eq!(r.ui().len(), 1);
    }

    #[tokio::test]
    async fn close_resets_the_mind_and_clears() {
        let mut r = rig();
        asked(&mut r).await;
        r.bridge.close();
        assert_eq!(r.session.calls().last(), Some(&Call::Close));
        assert_eq!(
            &r.mind.calls()[1..],
            &[MindCall::Cancel, MindCall::Lapse, MindCall::Reset]
        );
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
    }

    #[tokio::test]
    async fn close_sends_cleared_when_reset_dropped_a_step() {
        let mut r = rig();
        r.bridge.close();
        assert_eq!(r.ui(), vec![]);
        r.mind.reset_drops.store(true, Ordering::SeqCst);
        r.bridge.close();
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
    }

    #[tokio::test]
    async fn close_ends_the_turn_in_flight() {
        let r = rig();
        let gate = Arc::new(Notify::new());
        r.mind
            .script(vec![Step::Wait(gate.clone()), say("late"), end(Then::Rest)]);
        r.heard("hello", 1);
        settle().await;
        r.bridge.close();
        gate.notify_one();
        settle().await;
        assert_eq!(r.session.calls(), vec![Call::Thinking, Call::Close]);
    }

    #[tokio::test]
    async fn stop_cancels_and_listens() {
        let r = rig();
        r.bridge.stop();
        assert_eq!(r.mind.calls(), vec![MindCall::Cancel]);
        assert_eq!(r.session.calls(), vec![Call::Interrupt]);
        r.bridge.open();
        assert_eq!(r.session.calls(), vec![Call::Interrupt, Call::Open]);
    }

    #[tokio::test]
    async fn pick_out_of_range_is_ignored() {
        let mut r = rig();
        r.bridge.pick(0); // nothing showing
        asked(&mut r).await;
        r.bridge.pick(2);
        assert!(!r
            .session
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Answered(_))));
        // A second click on the same question is no second turn.
        r.bridge.pick(1);
        r.bridge.pick(0);
        let answered: Vec<_> = r
            .session
            .calls()
            .into_iter()
            .filter(|c| matches!(c, Call::Answered(_)))
            .collect();
        assert_eq!(answered, vec![Call::Answered("No".into())]);
    }

    #[tokio::test]
    async fn answered_none_does_nothing() {
        let mut r = rig();
        asked(&mut r).await;
        r.session.off.store(true, Ordering::SeqCst);
        r.bridge.pick(0);
        settle().await;
        assert_eq!(r.mind.calls(), vec![MindCall::Hear("approve it".into())]);
        assert!(r.ui().is_empty());
        // A later spoken turn with any id is heard, not picked.
        r.heard("yes", 101);
        settle().await;
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Hear("yes".into())));
    }

    #[tokio::test]
    async fn filler_goes_to_filler() {
        let mut r = rig();
        r.mind.script(vec![
            Step::Send(MindEvent::Thinking),
            Step::Send(MindEvent::Filler {
                text: "One moment.".into(),
            }),
            say("Here."),
            end(Then::Rest),
        ]);
        r.heard("what's up", 1);
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Filler(1, "One moment.".into()),
                Call::Delta(1, "Here.".into()),
                Call::End(1, After::Rest),
            ]
        );
        assert_eq!(r.ui(), vec![heard_ev("what's up")]);
    }

    #[tokio::test]
    async fn then_maps_to_after() {
        assert_eq!(after(Then::Rest), After::Rest);
        assert_eq!(after(Then::FollowUp), After::FollowUp);
        assert_eq!(after(Then::Ask), After::Ask);
        let r = rig();
        for (turn, then) in [(1, Then::Rest), (2, Then::FollowUp), (3, Then::Ask)] {
            r.mind.script(vec![end(then)]);
            r.heard("x", turn);
            settle().await;
            assert_eq!(
                r.session.calls().last(),
                Some(&Call::End(turn, after(then)))
            );
        }
    }

    #[tokio::test]
    async fn card_and_open_are_forwarded() {
        let mut r = rig();
        let card = Card::List {
            title: "What needs you".into(),
            lines: vec!["Mail asks: which?".into()],
        };
        r.mind.script(vec![
            say("One question."),
            Step::Send(MindEvent::Card { card: card.clone() }),
            Step::Send(MindEvent::Open {
                target: OpenTarget::Inbox,
            }),
            end(Then::FollowUp),
        ]);
        r.heard("what needs me", 1);
        settle().await;
        assert_eq!(
            r.ui(),
            vec![
                heard_ev("what needs me"),
                VoiceEvent::Card { card },
                VoiceEvent::Open {
                    target: OpenTarget::Inbox
                },
            ]
        );
        // The card is showing: close clears it.
        r.bridge.close();
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
    }

    #[tokio::test]
    async fn cleared_is_forwarded_and_forgets_the_options() {
        let mut r = rig();
        asked(&mut r).await;
        r.mind.script(vec![
            Step::Pending(false),
            Step::Send(MindEvent::Cleared),
            say("Okay, I won't."),
            end(Then::Rest),
        ]);
        r.heard("no", 2);
        settle().await;
        assert_eq!(r.ui(), vec![heard_ev("no"), VoiceEvent::Cleared]);
        r.bridge.pick(0);
        assert!(!r
            .session
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Answered(_))));
    }

    #[tokio::test]
    async fn a_step_dropped_by_a_stale_turn_is_cleared_when_the_live_turn_ends() {
        let mut r = rig();
        asked(&mut r).await;
        let old = Arc::new(Notify::new());
        let new = Arc::new(Notify::new());
        // The older turn drops the step, but its `Cleared` comes after a newer `Heard`.
        r.mind.script(vec![
            Step::Wait(old.clone()),
            Step::Pending(false),
            Step::Send(MindEvent::Cleared),
            end(Then::Rest),
        ]);
        r.mind
            .script(vec![Step::Wait(new.clone()), say("Hm."), end(Then::Rest)]);
        r.heard("something", 2);
        settle().await;
        r.heard("else", 3);
        settle().await;
        old.notify_one();
        settle().await;
        assert_eq!(r.ui(), vec![heard_ev("something"), heard_ev("else")]);
        new.notify_one();
        settle().await;
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
    }

    #[tokio::test]
    async fn a_stale_deeds_outcome_is_spoken_on_the_live_turn() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        // Turn 1 carries out a deed while turn 2 is heard; the mind begins turn 2 after it.
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            Step::Send(MindEvent::Did {
                said: "Approved.".into(),
            }),
            say("Approved."),
            end(Then::FollowUp),
        ]);
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            say("Nothing else waits."),
            end(Then::Rest),
        ]);
        r.heard("yes", 1);
        settle().await;
        r.heard("yes", 2);
        settle().await;
        gate.notify_one();
        settle().await;
        gate.notify_one();
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Thinking,
                Call::Delta(2, "Approved. ".into()),
                Call::Delta(2, "Nothing else waits.".into()),
                Call::End(2, After::Rest),
            ]
        );
        assert_eq!(
            r.ui(),
            vec![
                heard_ev("yes"),
                heard_ev("yes"),
                VoiceEvent::Saying {
                    text: "Approved.".into()
                },
            ]
        );
        // The live turn's own `Did` is not spoken twice.
        r.mind.script(vec![
            Step::Send(MindEvent::Did {
                said: "Denied.".into(),
            }),
            say("Denied."),
            end(Then::Rest),
        ]);
        r.heard("deny it", 3);
        settle().await;
        assert_eq!(
            &r.session.calls()[5..],
            &[
                Call::Thinking,
                Call::Delta(3, "Denied.".into()),
                Call::End(3, After::Rest),
            ]
        );
        assert_eq!(r.ui(), vec![heard_ev("deny it")]);
        // With no live turn and the session resting, it is only shown.
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            Step::Send(MindEvent::Did {
                said: "Sent to Research.".into(),
            }),
            end(Then::Rest),
        ]);
        r.heard("send it", 4);
        settle().await;
        r.bridge.close();
        let before = r.session.calls().len();
        gate.notify_one();
        settle().await;
        assert_eq!(r.session.calls().len(), before);
        assert_eq!(
            r.ui(),
            vec![
                heard_ev("send it"),
                VoiceEvent::Saying {
                    text: "Sent to Research.".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_turn_with_no_events_still_ends() {
        let r = rig();
        r.mind.script(vec![]);
        r.heard("yes", 7);
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![Call::Thinking, Call::End(7, After::Rest)]
        );
    }

    #[tokio::test]
    async fn session_events_are_forwarded() {
        let mut r = rig();
        let events = [
            (
                SessionEvent::State {
                    state: State::Listening,
                    mic_open: true,
                },
                VoiceEvent::State {
                    state: State::Listening,
                    mic_open: true,
                },
            ),
            (
                SessionEvent::Level {
                    mic: 0.5,
                    voice: 0.0,
                },
                VoiceEvent::Level {
                    mic: 0.5,
                    voice: 0.0,
                },
            ),
            (
                SessionEvent::Hearing { text: "he".into() },
                VoiceEvent::Hearing { text: "he".into() },
            ),
            (
                SessionEvent::Saying { text: "Hi.".into() },
                VoiceEvent::Saying { text: "Hi.".into() },
            ),
            (SessionEvent::Woke, VoiceEvent::Woke),
            (
                SessionEvent::Trouble {
                    message: "no mic".into(),
                },
                VoiceEvent::Trouble {
                    message: "no mic".into(),
                },
            ),
        ];
        for (ev, want) in events {
            r.bridge.on_session(ev);
            assert_eq!(r.ui(), vec![want]);
        }
        assert!(r.mind.calls().is_empty());
        assert!(r.session.calls().is_empty());
    }
}
