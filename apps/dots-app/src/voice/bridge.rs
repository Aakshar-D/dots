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
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use dots_core::open_dot::picture::View;
use dots_core::open_dot::{MindEvent, Then};
use dots_voice::{After, Session, SessionEvent, State};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

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
    /// Something to say that answers no turn; spoken only while `Idle`, dropped otherwise.
    fn notice(&self, text: &str);
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
    fn notice(&self, text: &str) {
        Session::notice(self, text);
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
    /// Closed (Esc) and not opened since: a `Heard` already queued starts nothing.
    closed: bool,
}

impl Inner {
    fn forget(&mut self) {
        self.options.clear();
        self.asking = false;
        self.showing = false;
    }
}

/// One turn for the mind, in the order it was heard.
struct Job {
    turn: u64,
    input: Input,
    view: View,
}

pub struct Bridge {
    session: Arc<dyn SessionPort>,
    mind: Arc<dyn MindPort>,
    ui: UnboundedSender<VoiceEvent>,
    inner: Mutex<Inner>,
    /// Turns go to one driver task, which begins them at the mind in order.
    jobs: UnboundedSender<Job>,
    /// The driver's end, until the first turn starts it.
    waiting_jobs: Mutex<Option<UnboundedReceiver<Job>>>,
}

/// Said when a mind call breaks.
const WENT_WRONG: &str = "Something went wrong. Try again.";

/// A level the UI can read: anything not finite is 0.
fn finite(level: f32) -> f32 {
    if level.is_finite() {
        level
    } else {
        0.0
    }
}

impl Bridge {
    pub fn new(
        session: Arc<dyn SessionPort>,
        mind: Arc<dyn MindPort>,
        ui: UnboundedSender<VoiceEvent>,
    ) -> Arc<Bridge> {
        let (jobs, rx) = unbounded_channel();
        Arc::new(Bridge {
            session,
            mind,
            ui,
            inner: Mutex::new(Inner::default()),
            jobs,
            waiting_jobs: Mutex::new(Some(rx)),
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
                match state {
                    State::Idle | State::Off => self.rested(state == State::Off),
                    // `end` clears the live turn before the session can listen again by
                    // itself: a live turn here was cut (the person talked over Dot).
                    State::Listening => self.cut(),
                    _ => {}
                }
            }
            // A level that is not a number would reach the UI as `null`.
            SessionEvent::Level { mic, voice } => self.send(VoiceEvent::Level {
                mic: finite(mic),
                voice: finite(voice),
            }),
            SessionEvent::Hearing { text } => self.send(VoiceEvent::Hearing { text }),
            SessionEvent::Saying { text } => self.send(VoiceEvent::Saying { text }),
            // The session is listening already. A turn still live ("Hey Dot" said over Dot)
            // is dropped, as `open` drops it. The window is not raised: the reply is spoken.
            SessionEvent::Woke => {
                {
                    let mut s = self.lock();
                    s.closed = false;
                    if s.live.is_some() {
                        self.mind.cancel();
                        self.drop_live(&mut s);
                    }
                }
                self.send(VoiceEvent::Woke);
            }
            SessionEvent::Trouble { message } => self.send(VoiceEvent::Trouble { message }),
        }
    }

    /// The account, workspace and screen the next turns see.
    pub fn set_view(&self, view: View) {
        self.lock().view = view;
    }

    /// Hotkey, rail, tray: listen. While Dot thinks or speaks the session takes this as
    /// interrupt, so a turn in flight is dropped first, as `stop` drops it.
    pub fn open(&self) {
        let mut s = self.lock();
        s.closed = false;
        if s.live.is_some() {
            self.mind.cancel();
            self.drop_live(&mut s);
        }
        self.session.open();
    }

    /// The session listens again while a turn is live: it cut that turn. The turn is dropped
    /// with what it showed; a deed already started still finishes and says its outcome.
    fn cut(&self) {
        let mut s = self.lock();
        if s.live.is_some() {
            self.mind.cancel();
            self.drop_live(&mut s);
        }
    }

    /// Esc: stop everything, drop what waits, forget the conversation. A sentence the session
    /// had already sent starts nothing until the next `open` (or "Hey Dot").
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
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

    /// Stop speaking (or thinking) and listen. A turn in flight is dropped with what it
    /// showed: the session has ended it.
    pub fn stop(&self) {
        let mut s = self.lock();
        self.mind.cancel();
        if s.live.is_some() {
            self.drop_live(&mut s);
        }
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
        match self.session.answered(&text) {
            Some(turn) => {
                s.options.clear();
                s.picks.insert(turn, index);
            }
            // The session is off: the options are dead.
            None => {
                s.forget();
                self.send(VoiceEvent::Cleared);
            }
        }
    }

    /// A finished sentence (or a click) is the live turn from now on.
    fn heard(self: &Arc<Self>, text: String, turn: u64) {
        let job = {
            let mut s = self.lock();
            if s.closed {
                s.picks.remove(&turn);
                return;
            }
            s.live = Some(turn);
            s.gap = false;
            self.session.thinking();
            self.send(VoiceEvent::Heard { text: text.clone() });
            let input = match s.picks.remove(&turn) {
                Some(index) => Input::Pick(index),
                None => Input::Hear(text),
            };
            Job {
                turn,
                input,
                view: s.view.clone(),
            }
        };
        self.drive();
        let _ = self.jobs.send(job);
    }

    /// The session rested (Idle) or stopped (Off). A turn in flight was given up by the
    /// session (its think limit, a failure, a close), and a step still waiting has lapsed
    /// (its asking time ran out): both are dropped, with what they showed.
    fn rested(&self, off: bool) {
        let mut s = self.lock();
        if off {
            s.picks.clear();
        }
        if s.live.is_some() || self.mind.pending() {
            self.drop_live(&mut s);
        }
    }

    /// `mind.lapse` (it ends the turn in flight and drops a step that waits); the live turn
    /// and what showed go, and the UI is told when something showed.
    fn drop_live(&self, s: &mut Inner) {
        let lapsed = self.lapse();
        let showing = s.showing || s.asking;
        s.live = None;
        s.forget();
        if showing || lapsed {
            self.send(VoiceEvent::Cleared);
        }
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
                // Resting, the session speaks nothing more (its speech failed): the question
                // is written instead, and clicks still answer it.
                let silent = matches!(self.session.state(), State::Idle | State::Off);
                let written = (silent && !spoken.is_empty()).then(|| spoken.to_string());
                self.send(VoiceEvent::Ask { question, options });
                if let Some(text) = written {
                    self.send(VoiceEvent::Saying { text });
                }
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
    /// turn; with none, it is said as a notice while the session rests, and only written
    /// otherwise. The session shows what it speaks (`Saying`), so the bridge writes it only
    /// when nothing will speak it.
    fn did(&self, s: &mut Inner, turn: u64, said: String) {
        if s.live == Some(turn) || said.is_empty() {
            return;
        }
        if let Some(live) = s.live {
            self.session.say_delta(live, &format!("{said} "));
            s.gap = false;
        } else if self.session.state() == State::Idle {
            self.session.notice(&said);
        } else {
            self.send(VoiceEvent::Saying { text: said });
        }
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

    /// The mind call for `turn` panicked: the turn ends, and the person is told.
    fn broke(&self, turn: u64) {
        let mut s = self.lock();
        if s.live == Some(turn) {
            self.end(&mut s, turn, After::Rest);
            self.send(VoiceEvent::Trouble {
                message: WENT_WRONG.into(),
            });
        }
    }

    /// Starts the driver once: it begins each turn at the mind in the order heard. A turn is
    /// begun only after the one before it has begun at the mind (a newer call is what ends an
    /// older one there), but without waiting for it to finish.
    fn drive(self: &Arc<Self>) {
        let Some(mut jobs) = self
            .waiting_jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return;
        };
        // Weak: the driver ends with the bridge.
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(job) = jobs.recv().await {
                let Some(bridge) = weak.upgrade() else { break };
                let (begun_tx, begun) = oneshot::channel();
                bridge.run(job, begun_tx);
                drop(bridge);
                // Dropped unsent when the call panicked before it began: go on all the same.
                let _ = begun.await;
            }
        });
    }

    /// One turn: the mind call (on its own task, so a panic is caught) and the loop that
    /// applies its events.
    fn run(self: &Arc<Self>, job: Job, begun: oneshot::Sender<()>) {
        let bridge = Arc::clone(self);
        let Job { turn, input, view } = job;
        tokio::spawn(async move {
            let (tx, mut rx) = unbounded_channel();
            let mind = bridge.mind.clone();
            let checker = bridge.clone();
            // Owns `tx`: once the call returns (or panics), the channel closes.
            let call = tokio::spawn(async move {
                let call = async {
                    match input {
                        Input::Hear(text) => mind.hear(&text, &view, &tx).await,
                        Input::Pick(index) => mind.pick(index, &view, &tx).await,
                    }
                };
                let mut call = std::pin::pin!(call);
                let mut begun = Some(begun);
                // The first poll happens under the bridge's lock, and only while `turn` is
                // still the live turn: a close, stop or rest that dropped it before the
                // driver got here cannot slip in between, and the mind never sees it. The
                // mind takes the turn before its first await; one that must wait for a deed
                // in flight checks on its own that nothing was dropped meanwhile.
                // `begun` is sent once that first poll is over, whatever it did.
                std::future::poll_fn(|cx| {
                    if let Some(b) = begun.take() {
                        let s = checker.lock();
                        if s.live != Some(turn) {
                            drop(s);
                            let _ = b.send(());
                            return std::task::Poll::Ready(());
                        }
                        let polled = call.as_mut().poll(cx);
                        drop(s);
                        let _ = b.send(());
                        return polled;
                    }
                    call.as_mut().poll(cx)
                })
                .await
            });
            let apply = async {
                while let Some(ev) = rx.recv().await {
                    bridge.apply(turn, ev);
                }
            };
            let (called, ()) = tokio::join!(call, apply);
            match called {
                Err(e) if e.is_panic() => bridge.broke(turn),
                _ => bridge.finished(turn),
            }
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
        Notice(String),
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
        state: Mutex<State>,
    }

    impl FakeSession {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::default(),
                last: AtomicU64::new(100),
                off: AtomicBool::new(false),
                state: Mutex::new(State::Thinking),
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
            *self.state.lock().unwrap()
        }
        fn notice(&self, text: &str) {
            self.push(Call::Notice(text.into()));
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
        /// The mind call panics.
        Panic,
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
                    Step::Panic => panic!("the mind broke (a test)"),
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
        // The dead options go away.
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
        r.bridge.pick(1);
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
    async fn a_heard_queued_before_close_starts_nothing() {
        let mut r = rig();
        r.bridge.close();
        r.heard("queued", 1);
        settle().await;
        assert!(r
            .mind
            .calls()
            .iter()
            .all(|c| !matches!(c, MindCall::Hear(_))));
        assert!(!r.session.calls().contains(&Call::Thinking));
        assert!(r.ui().is_empty());
        // Opened again, a sentence goes to the mind.
        r.bridge.open();
        r.mind.script(vec![say("Hi."), end(Then::Rest)]);
        r.heard("hello", 2);
        settle().await;
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Hear("hello".into())));
        assert_eq!(r.session.calls().last(), Some(&Call::End(2, After::Rest)));
        // So does one after "Hey Dot".
        r.bridge.close();
        r.bridge.on_session(SessionEvent::Woke);
        r.heard("again", 3);
        settle().await;
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Hear("again".into())));
    }

    #[tokio::test]
    async fn stop_drops_the_live_turn() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            Step::Send(MindEvent::Card {
                card: Card::List {
                    title: "t".into(),
                    lines: vec![],
                },
            }),
            Step::Wait(gate.clone()),
            say("late"),
            end(Then::Rest),
        ]);
        r.heard("hello", 1);
        settle().await;
        r.ui();
        r.bridge.stop();
        assert_eq!(&r.mind.calls()[1..], &[MindCall::Cancel, MindCall::Lapse]);
        assert_eq!(r.session.calls().last(), Some(&Call::Interrupt));
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
        gate.notify_one();
        settle().await;
        assert_eq!(r.session.calls().last(), Some(&Call::Interrupt));
    }

    #[tokio::test]
    async fn idle_with_a_live_turn_lapses_and_clears() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            Step::Send(MindEvent::Card {
                card: Card::List {
                    title: "t".into(),
                    lines: vec![],
                },
            }),
            Step::Wait(gate.clone()),
            say("late"),
            end(Then::Rest),
        ]);
        r.heard("hello", 1);
        settle().await;
        r.ui();
        // The session gave up on the turn (its think limit, or a failure).
        r.bridge.on_session(SessionEvent::State {
            state: State::Idle,
            mic_open: false,
        });
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Lapse));
        assert_eq!(
            r.ui(),
            vec![
                VoiceEvent::State {
                    state: State::Idle,
                    mic_open: false
                },
                VoiceEvent::Cleared,
            ]
        );
        gate.notify_one();
        settle().await;
        assert_eq!(r.session.calls(), vec![Call::Thinking]);
    }

    #[tokio::test]
    async fn a_panicking_mind_still_ends_the_turn() {
        let mut r = rig();
        r.mind.script(vec![say("Hm"), Step::Panic]);
        r.heard("break", 1);
        settle().await;
        assert_eq!(
            r.session.calls(),
            vec![
                Call::Thinking,
                Call::Delta(1, "Hm".into()),
                Call::End(1, After::Rest)
            ]
        );
        assert_eq!(
            r.ui(),
            vec![
                heard_ev("break"),
                VoiceEvent::Trouble {
                    message: "Something went wrong. Try again.".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_question_asked_while_the_session_rests_is_shown_and_written() {
        let mut r = rig();
        *r.session.state.lock().unwrap() = State::Idle;
        r.mind.script(vec![
            ask("Approve it?", &["Yes", "No"]),
            Step::Pending(true),
            end(Then::Ask),
        ]);
        r.heard("approve", 1);
        settle().await;
        assert_eq!(
            r.ui(),
            vec![
                heard_ev("approve"),
                ask_ev("Approve it?", &["Yes", "No"]),
                VoiceEvent::Saying {
                    text: "Approve it?".into()
                },
            ]
        );
        // Clicks still work.
        r.bridge.pick(0);
        assert_eq!(
            r.session.calls().last(),
            Some(&Call::Answered("Yes".into()))
        );
    }

    #[tokio::test]
    async fn off_forgets_the_clicks() {
        let r = rig();
        asked_plain(&r).await;
        r.bridge.pick(0); // turn 101, whose `Heard` never comes
        r.bridge.on_session(SessionEvent::State {
            state: State::Off,
            mic_open: false,
        });
        r.bridge.open();
        r.heard("Yes", 101);
        settle().await;
        assert_eq!(r.mind.calls().last(), Some(&MindCall::Hear("Yes".into())));
    }

    /// Turn 1 asks "Approve it?" (as `asked`, for a rig not borrowed mutably).
    async fn asked_plain(r: &Rig) {
        r.mind.script(vec![
            ask("Approve it?", &["Yes", "No"]),
            Step::Pending(true),
            end(Then::Ask),
        ]);
        r.heard("approve it", 1);
        settle().await;
    }

    /// A mind in which a newer turn's beginning ends the older one, as the real mind does.
    #[derive(Default)]
    struct OrderMind {
        begun: Mutex<Vec<String>>,
        current: Mutex<Option<String>>,
        pending: Mutex<Option<String>>,
    }

    #[async_trait]
    impl MindPort for OrderMind {
        async fn hear(&self, heard: &str, _view: &View, out: &UnboundedSender<MindEvent>) {
            // Begun at the first poll, with no await before it.
            self.begun.lock().unwrap().push(heard.into());
            *self.current.lock().unwrap() = Some(heard.into());
            *self.pending.lock().unwrap() = None;
            for _ in 0..3 {
                tokio::task::yield_now().await;
            }
            let cur = self.current.lock().unwrap();
            if cur.as_deref() == Some(heard) {
                *self.pending.lock().unwrap() = Some(heard.into());
                let _ = out.send(MindEvent::Ask {
                    question: heard.into(),
                    options: vec!["Yes".into(), "No".into()],
                });
                let _ = out.send(MindEvent::SayEnd { then: Then::Ask });
            }
        }
        async fn pick(&self, _index: usize, _view: &View, _out: &UnboundedSender<MindEvent>) {}
        fn lapse(&self, _out: &UnboundedSender<MindEvent>) {
            *self.pending.lock().unwrap() = None;
        }
        fn cancel(&self) {}
        fn reset(&self) -> bool {
            self.pending.lock().unwrap().take().is_some()
        }
        fn pending(&self) -> bool {
            self.pending.lock().unwrap().is_some()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mind_turns_begin_in_the_order_they_were_heard() {
        let session = FakeSession::new();
        let mind = Arc::new(OrderMind::default());
        let (tx, mut ui) = unbounded_channel();
        let bridge = Bridge::new(session.clone(), mind.clone(), tx);
        let mut heard = Vec::new();
        for pair in 0..50u64 {
            let (a, b) = (format!("a{pair}"), format!("b{pair}"));
            for (text, turn) in [(&a, pair * 2 + 1), (&b, pair * 2 + 2)] {
                bridge.on_session(SessionEvent::Heard {
                    text: text.clone(),
                    turn,
                });
                heard.push(text.clone());
            }
            // The newer turn's question shows, and only its step waits.
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut asked = None;
            while asked.as_ref() != Some(&b) {
                assert!(
                    std::time::Instant::now() < until,
                    "pair {pair}: no question"
                );
                match ui.try_recv() {
                    Ok(VoiceEvent::Ask { question, .. }) => asked = Some(question),
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
                }
            }
            // An older turn the newer one replaced before the driver took it never begins;
            // the ones that begin do so in the order heard, the newest last.
            let begun = mind.begun.lock().unwrap().clone();
            let mut at = heard.iter();
            for text in &begun {
                assert!(at.any(|h| h == text), "pair {pair}: out of order {begun:?}");
            }
            assert_eq!(begun.last(), Some(&b), "pair {pair}");
            assert_eq!(begun.iter().filter(|t| *t == &b).count(), 1, "pair {pair}");
            assert_eq!(
                *mind.pending.lock().unwrap(),
                Some(b.clone()),
                "pair {pair}"
            );
        }
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
        // The session shows what it speaks: the bridge does not write it as well.
        assert_eq!(r.ui(), vec![heard_ev("yes"), heard_ev("yes")]);
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
        // With no live turn and the session busy with no turn of ours (listening), it is only
        // written.
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
        *r.session.state.lock().unwrap() = State::Listening;
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
        // Resting, it is said as a notice, and the session shows it as it speaks it.
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            Step::Wait(gate.clone()),
            Step::Send(MindEvent::Did {
                said: "Approved.".into(),
            }),
            end(Then::Rest),
        ]);
        r.bridge.open();
        r.heard("approve it", 5);
        settle().await;
        r.bridge.close();
        *r.session.state.lock().unwrap() = State::Idle;
        gate.notify_one();
        settle().await;
        assert_eq!(
            r.session.calls().last(),
            Some(&Call::Notice("Approved.".into()))
        );
        assert_eq!(r.ui(), vec![heard_ev("approve it")]);
    }

    /// Turn 1 shows a card, then waits for `gate`, then carries out a deed and says it.
    async fn deed_in_flight(r: &mut Rig, gate: &Arc<Notify>) {
        r.mind.script(vec![
            Step::Send(MindEvent::Card {
                card: Card::List {
                    title: "t".into(),
                    lines: vec![],
                },
            }),
            Step::Wait(gate.clone()),
            Step::Send(MindEvent::Did {
                said: "Sent to Research.".into(),
            }),
            say("Sent to Research."),
            end(Then::Rest),
        ]);
        r.heard("tell research to look at the build", 1);
        settle().await;
        r.ui();
    }

    /// After the turn was dropped: nothing of it reaches the session, and the deed that was
    /// already under way is written.
    async fn finishes_unspoken(r: &mut Rig, gate: &Arc<Notify>) {
        let before = r.session.calls();
        *r.session.state.lock().unwrap() = State::Listening;
        gate.notify_one();
        settle().await;
        assert_eq!(r.session.calls(), before);
        assert_eq!(
            r.ui(),
            vec![VoiceEvent::Saying {
                text: "Sent to Research.".into()
            }]
        );
    }

    #[tokio::test]
    async fn open_during_thinking_drops_the_turn() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        deed_in_flight(&mut r, &gate).await;
        r.bridge.open();
        assert_eq!(&r.mind.calls()[1..], &[MindCall::Cancel, MindCall::Lapse]);
        assert_eq!(r.session.calls().last(), Some(&Call::Open));
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared]);
        finishes_unspoken(&mut r, &gate).await;
        // With no turn in flight, open only opens.
        r.bridge.open();
        assert_eq!(r.mind.calls().len(), 3);
    }

    #[tokio::test]
    async fn talking_over_drops_the_turn() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        deed_in_flight(&mut r, &gate).await;
        r.bridge.on_session(SessionEvent::State {
            state: State::Listening,
            mic_open: true,
        });
        assert_eq!(&r.mind.calls()[1..], &[MindCall::Cancel, MindCall::Lapse]);
        assert_eq!(
            r.ui(),
            vec![
                VoiceEvent::State {
                    state: State::Listening,
                    mic_open: true
                },
                VoiceEvent::Cleared
            ]
        );
        finishes_unspoken(&mut r, &gate).await;
        // Listening with no turn live drops nothing.
        r.bridge.on_session(SessionEvent::State {
            state: State::Listening,
            mic_open: true,
        });
        assert_eq!(r.mind.calls().len(), 3);
    }

    // ---- the wake phrase, the stop phrase and talking over Dot (step 5) ----------------------

    #[tokio::test]
    async fn a_wake_after_close_lets_the_next_heard_start_a_turn() {
        let mut r = rig();
        r.bridge.close();
        r.ui();
        r.bridge.on_session(SessionEvent::Woke);
        assert_eq!(r.ui(), vec![VoiceEvent::Woke]);
        r.mind.script(vec![say("Hi."), end(Then::Rest)]);
        r.heard("what needs me", 1);
        settle().await;
        assert_eq!(
            r.mind.calls().last(),
            Some(&MindCall::Hear("what needs me".into()))
        );
        assert_eq!(r.session.calls().last(), Some(&Call::End(1, After::Rest)));
        assert_eq!(r.ui(), vec![heard_ev("what needs me")]);
    }

    #[tokio::test]
    async fn a_wake_during_a_live_turn_drops_it_as_open_does() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        deed_in_flight(&mut r, &gate).await;
        // "Hey Dot" said over Dot speaking.
        r.bridge.on_session(SessionEvent::Woke);
        assert_eq!(&r.mind.calls()[1..], &[MindCall::Cancel, MindCall::Lapse]);
        assert_eq!(r.ui(), vec![VoiceEvent::Cleared, VoiceEvent::Woke]);
        finishes_unspoken(&mut r, &gate).await;
        // With no turn live, a wake drops nothing.
        r.bridge.on_session(SessionEvent::Woke);
        assert_eq!(r.mind.calls().len(), 3);
        assert_eq!(r.ui(), vec![VoiceEvent::Woke]);
    }

    #[tokio::test]
    async fn listening_after_dot_stop_drops_the_live_turn_and_speaks_no_more_of_it() {
        let mut r = rig();
        let gate = Arc::new(Notify::new());
        r.mind.script(vec![
            say("Here is a long answer."),
            Step::Wait(gate.clone()),
            say(" And more of it."),
            end(Then::FollowUp),
        ]);
        r.heard("tell me everything", 1);
        settle().await;
        r.ui();
        let before = r.session.calls();
        assert_eq!(
            before.last(),
            Some(&Call::Delta(1, "Here is a long answer.".into()))
        );
        // "Dot, stop": the session stops speaking and listens.
        r.bridge.on_session(SessionEvent::State {
            state: State::Listening,
            mic_open: true,
        });
        assert_eq!(&r.mind.calls()[1..], &[MindCall::Cancel, MindCall::Lapse]);
        gate.notify_one();
        settle().await;
        assert_eq!(
            r.session.calls(),
            before,
            "nothing more for the stopped turn"
        );
        assert_eq!(
            r.ui(),
            vec![VoiceEvent::State {
                state: State::Listening,
                mic_open: true
            }]
        );
    }

    /// The single worker is busy until the returned sender is used (or dropped): what is
    /// spawned meanwhile waits.
    fn hold_the_worker() -> std::sync::mpsc::Sender<()> {
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        tokio::spawn(async move {
            let _ = held_tx.send(());
            let _ = go_rx.recv();
        });
        held_rx.recv().unwrap();
        go_tx
    }

    // One worker, and the test body on its own thread: the driver cannot run until the test
    // lets the worker go, so the turn is surely dropped before the driver takes it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn a_turn_dropped_before_the_driver_takes_it_never_reaches_the_mind() {
        for drop_it in ["close", "stop", "idle", "open"] {
            let r = rig();
            r.mind.script(vec![
                Step::Pending(true),
                ask("Approve it?", &["Yes", "No"]),
                end(Then::Ask),
            ]);
            let go = hold_the_worker();
            r.heard("approve the research one", 1);
            match drop_it {
                "close" => r.bridge.close(),
                "stop" => r.bridge.stop(),
                "open" => r.bridge.open(),
                _ => r.bridge.on_session(SessionEvent::State {
                    state: State::Idle,
                    mic_open: false,
                }),
            }
            go.send(()).unwrap();
            // The driver, the turn task and the call all get to run.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            assert!(
                !r.mind
                    .calls()
                    .iter()
                    .any(|c| matches!(c, MindCall::Hear(_))),
                "{drop_it}: {:?}",
                r.mind.calls()
            );
            assert!(!r.mind.pending(), "{drop_it}: a step waits");
            assert!(
                !r.session
                    .calls()
                    .iter()
                    .any(|c| matches!(c, Call::End(..) | Call::Delta(..))),
                "{drop_it}: {:?}",
                r.session.calls()
            );
        }
    }

    #[tokio::test]
    async fn a_level_that_is_not_a_number_is_sent_as_zero() {
        let mut r = rig();
        r.bridge.on_session(SessionEvent::Level {
            mic: f32::NAN,
            voice: f32::INFINITY,
        });
        assert_eq!(
            r.ui(),
            vec![VoiceEvent::Level {
                mic: 0.0,
                voice: 0.0
            }]
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
