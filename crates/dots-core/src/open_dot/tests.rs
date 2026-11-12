//! The mind against a running core, with a scripted thinker in place of the CLI.

use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;

use async_trait::async_trait;
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::engine::claude::once::{OnceAnswer, OnceUsage};
use crate::model::{Approval, ApprovalStatus, Dot, Message, NewRun, TriggerKind, WorkspaceMode};
use crate::settings::Settings;
use crate::Config;

/// One scripted model reply.
#[derive(Clone)]
struct Scripted {
    deltas: Vec<String>,
    text: String,
    usage: Option<OnceUsage>,
    delay: Duration,
    fail: Option<OnceFailure>,
    /// Answers after its delay even when cancelled: a reply that lands late.
    stubborn: bool,
    /// Waits for this before it answers (after its delay).
    hold: Option<Arc<tokio::sync::Notify>>,
    /// Panics instead of answering.
    panics: bool,
}

/// A reply of `text`, streamed in pieces of 4 bytes or so, with usage.
fn says(text: &str) -> Scripted {
    let mut deltas = Vec::new();
    let mut piece = String::new();
    for c in text.chars() {
        piece.push(c);
        if piece.len() >= 4 {
            deltas.push(std::mem::take(&mut piece));
        }
    }
    if !piece.is_empty() {
        deltas.push(piece);
    }
    Scripted {
        deltas,
        text: text.into(),
        usage: Some(OnceUsage {
            tokens_in: 10,
            tokens_out: 2,
            cache_read: 0,
            cost_usd: Some(0.001),
        }),
        delay: Duration::ZERO,
        fail: None,
        stubborn: false,
        hold: None,
        panics: false,
    }
}

/// `s`, landing only once `hold` is notified, whatever cancels it meanwhile: a reply that
/// lands late, when the test says so.
fn held(s: Scripted, hold: &Arc<tokio::sync::Notify>) -> Scripted {
    Scripted {
        stubborn: true,
        hold: Some(hold.clone()),
        ..s
    }
}

fn fails(failure: OnceFailure) -> Scripted {
    Scripted {
        fail: Some(failure),
        ..says("")
    }
}

fn slow(s: Scripted, delay: Duration) -> Scripted {
    Scripted { delay, ..s }
}

/// A thinker that answers from a queue; "Okay." when it is empty.
#[derive(Default)]
struct Fake {
    queue: Mutex<VecDeque<Scripted>>,
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>,
    cancels: Mutex<Vec<CancellationToken>>,
}

impl Fake {
    fn push(&self, s: Scripted) {
        lock(&self.queue).push_back(s);
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_prompt(&self) -> String {
        lock(&self.prompts).last().cloned().unwrap_or_default()
    }
}

#[async_trait]
impl Thinker for Fake {
    async fn think(
        &self,
        _system: &str,
        prompt: &str,
        _model: &str,
        cancel: &CancellationToken,
        on_text: &mut (dyn for<'s> FnMut(&'s str) + Send),
    ) -> std::result::Result<OnceAnswer, OnceFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        lock(&self.prompts).push(prompt.to_string());
        lock(&self.cancels).push(cancel.clone());
        let s = lock(&self.queue)
            .pop_front()
            .unwrap_or_else(|| says("Okay."));
        if s.stubborn {
            tokio::time::sleep(s.delay).await;
        } else if !s.delay.is_zero() {
            tokio::select! {
                _ = tokio::time::sleep(s.delay) => {}
                _ = cancel.cancelled() => return Err(OnceFailure::Cancelled),
            }
        }
        if let Some(hold) = &s.hold {
            hold.notified().await;
        }
        if s.panics {
            panic!("the thinker broke");
        }
        if cancel.is_cancelled() && !s.stubborn {
            return Err(OnceFailure::Cancelled);
        }
        if let Some(f) = s.fail {
            return Err(f);
        }
        for d in &s.deltas {
            on_text(d);
        }
        Ok(OnceAnswer {
            text: s.text,
            usage: s.usage,
        })
    }
}

struct T {
    dir: TempDir,
    rt: Runtime,
    fake: Arc<Fake>,
    mind: Mind,
}

impl T {
    async fn start() -> T {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.claude_path = Some(dir.path().join("no-claude-here"));
        let rt = Runtime::start(cfg).await.unwrap();
        // Nothing runs: runs only wait in the queue.
        rt.hold_runs(true).await;
        let fake = Arc::new(Fake::default());
        let mind = Mind::new(fake.clone());
        T {
            dir,
            rt,
            fake,
            mind,
        }
    }

    async fn dot(&self, name: &str) -> Dot {
        let workdir = self.dir.path().to_string_lossy().to_string();
        let mut spec = self.rt.new_dot_spec(name, "Do the task.", &workdir).await;
        spec.workspace_mode = WorkspaceMode::Folder;
        self.rt.create_dot(spec, None, None).await.unwrap()
    }

    async fn approval(&self, dot: &Dot, command: &str) -> Approval {
        let run = self
            .rt
            .store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        self.rt
            .store()
            .create_approval(&run.id, "Bash", &json!({ "command": command }))
            .await
            .unwrap()
    }

    async fn question(&self, dot: &Dot, text: &str) -> Message {
        let run = self
            .rt
            .store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        // a question is asked by a run that ended
        self.rt
            .store()
            .finish_run(&run.id, crate::model::RunStatus::Succeeded, None, None)
            .await
            .unwrap();
        self.rt
            .store()
            .add_question(&dot.id, &run.id, text, None)
            .await
            .unwrap()
            .expect("a question")
    }

    async fn hear_in(&self, words: &str, view: &View) -> Vec<MindEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.mind.hear(&self.rt, words, view, &tx).await;
        drain(&mut rx)
    }

    async fn hear(&self, words: &str) -> Vec<MindEvent> {
        self.hear_in(words, &home()).await
    }

    async fn pick(&self, index: usize) -> Vec<MindEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.mind.pick(&self.rt, index, &home(), &tx).await;
        drain(&mut rx)
    }

    async fn status(&self, approval: &Approval) -> ApprovalStatus {
        self.rt
            .store()
            .get_approval(&approval.id)
            .await
            .unwrap()
            .status
    }

    async fn runs(&self, dot: &Dot) -> usize {
        self.rt
            .store()
            .list_runs(Some(&dot.id), 50)
            .await
            .unwrap()
            .len()
    }
}

fn drain(rx: &mut mpsc::UnboundedReceiver<MindEvent>) -> Vec<MindEvent> {
    let mut got = Vec::new();
    while let Ok(e) = rx.try_recv() {
        got.push(e);
    }
    got
}

fn home() -> View {
    View {
        account: "personal".into(),
        workspace: "default".into(),
        screen: "home".into(),
        dot_id: None,
        topic_root: None,
    }
}

/// Everything said, joined.
fn said(events: &[MindEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            MindEvent::Say { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

fn asks(events: &[MindEvent]) -> Option<(String, Vec<String>)> {
    events.iter().find_map(|e| match e {
        MindEvent::Ask { question, options } => Some((question.clone(), options.clone())),
        _ => None,
    })
}

fn ended(events: &[MindEvent]) -> Option<Then> {
    match events.last() {
        Some(MindEvent::SayEnd { then }) => Some(*then),
        _ => None,
    }
}

/// The outcomes of the deeds carried out (`Did`), in order.
fn dids(events: &[MindEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            MindEvent::Did { said } => Some(said.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn did_follows_every_deed() {
    // approve and deny, each after its confirmation
    let t = T::start().await;
    asking_to_approve(&t).await;
    let got = t.hear("yes").await;
    assert_eq!(dids(&got), vec!["Approved.".to_string()], "{got:?}");
    assert_eq!(said(&got), "Approved.");
    assert!(matches!(got.last(), Some(MindEvent::SayEnd { .. })));
    t.rt.shutdown();
    let t = T::start().await;
    asking_to_deny(&t).await;
    assert_eq!(dids(&t.hear("yes").await), vec!["Denied.".to_string()]);
    t.rt.shutdown();
    // run, after its confirmation
    let t = T::start().await;
    t.dot("Deploy watcher").await;
    t.fake.push(says("DO {\"act\":\"run\",\"d\":\"d1\"}"));
    let asked = t.hear("run the deploy watcher").await;
    assert!(dids(&asked).is_empty(), "nothing is done by asking");
    assert_eq!(
        dids(&t.hear("yes").await),
        vec!["Deploy watcher is running.".to_string()]
    );
    t.rt.shutdown();
    // send and answer, straight: only the outcome, not the words before it
    let t = T::start().await;
    t.dot("Research").await;
    t.fake.push(says(
        "Sending.\nDO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"look at the build\"}",
    ));
    let got = t.hear("tell research to look at the build").await;
    assert_eq!(said(&got), "Sending. Sent to Research.");
    assert_eq!(dids(&got), vec!["Sent to Research.".to_string()]);
    t.rt.shutdown();
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    t.question(&dot, "Which branch?").await;
    t.fake.push(says(
        "DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"main\"}",
    ));
    let got = t.hear("tell the release helper main").await;
    assert_eq!(dids(&got), vec!["Answered Release helper.".to_string()]);
    t.rt.shutdown();
    // create
    let t = T::start().await;
    let p = a_proposal();
    let question = create_question(&p);
    waiting_on(
        &t,
        Waiting::Create {
            proposal: p,
            question,
        },
    );
    let got = t.hear("yes").await;
    assert_eq!(dids(&got), vec!["Created Downloads digest.".to_string()]);
    // a no does nothing, and says no `Did`
    asking_to_approve(&t).await;
    assert!(dids(&t.hear("no").await).is_empty());
    t.rt.shutdown();
}

fn approve_a1() -> Scripted {
    says("Approving now.\nDO {\"act\":\"approve\",\"a\":\"a1\"}")
}

/// A dot with a waiting approval, and the mind asking to approve it.
async fn asking_to_approve(t: &T) -> Approval {
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    t.fake.push(approve_a1());
    let got = t.hear("approve the research one").await;
    assert_eq!(
        asks(&got),
        Some((
            "Research wants to use Bash on ls -la. Approve this request?".into(),
            yes_no()
        ))
    );
    assert!(t.mind.pending());
    approval
}

/// "Approve it" with nothing asked yet asks the model for the deed (the spec's "Approve it"
/// decides an approval); a plain "yes" still has nothing to answer.
#[tokio::test]
async fn approve_it_with_nothing_asked_goes_to_the_model() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    for words in ["Approve it.", "deny that", "run it", "create it", "send it"] {
        let calls = t.fake.calls();
        t.fake.push(says("Which one?"));
        let got = t.hear(words).await;
        assert_eq!(t.fake.calls(), calls + 1, "{words}");
        assert_eq!(said(&got), "Which one?", "{words}");
    }
    t.fake.push(approve_a1());
    let got = t.hear("Approve it.").await;
    assert_eq!(
        asks(&got),
        Some((
            "Research wants to use Bash on ls -la. Approve this request?".into(),
            yes_no()
        ))
    );
    assert_eq!(said(&t.hear("yes").await), "Approved.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
    t.rt.shutdown();
}

#[tokio::test]
async fn stop_and_cancel_clear_pending() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let got = t.hear("stop").await;
    assert_eq!(
        got,
        vec![MindEvent::Cleared, MindEvent::SayEnd { then: Then::Rest }]
    );
    assert!(!t.mind.pending());

    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    assert!(t.mind.pending());
    let got = t.hear("never mind").await;
    assert_eq!(
        got,
        vec![
            MindEvent::Cleared,
            MindEvent::Say {
                delta: "Okay.".into()
            },
            MindEvent::SayEnd { then: Then::Rest }
        ]
    );
    assert!(!t.mind.pending());
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // a yes now has nothing to answer
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn repeat_says_the_last_reply() {
    let t = T::start().await;
    t.fake.push(says("Two dots are idle. Nothing needs you."));
    let got = t.hear("how are things").await;
    assert_eq!(said(&got), "Two dots are idle. Nothing needs you.");
    assert_eq!(ended(&got), Some(Then::FollowUp));
    let got = t.hear("say that again").await;
    assert_eq!(said(&got), "Two dots are idle. Nothing needs you.");
    assert_eq!(ended(&got), Some(Then::FollowUp));
    assert_eq!(t.fake.calls(), 1);
    // with a question waiting, the question is asked again
    asking_to_approve(&t).await;
    let got = t.hear("what").await;
    assert_eq!(said(&got), "Approving now.");
    assert_eq!(
        asks(&got).map(|a| a.0),
        Some("Research wants to use Bash on ls -la. Approve this request?".into())
    );
    assert_eq!(ended(&got), Some(Then::Ask));
    assert!(t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn model_cannot_pass_a_confirmation() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    t.fake.push(says(
        "Approving now.\nDO {\"act\":\"approve\",\"a\":\"a1\",\"confirmed\":true}",
    ));
    let got = t.hear("approve it, I'm sure").await;
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    assert_eq!(asks(&got).unwrap().1, yes_no());
    let card = got.iter().find_map(|e| match e {
        MindEvent::Card {
            card: Card::Confirm { deed },
        } => Some(deed.clone()),
        _ => None,
    });
    assert_eq!(
        card,
        Some(Deed::Approve {
            approval_id: approval.id.clone(),
            dot_name: "Research".into(),
            tool: "Bash".into(),
            target: "ls -la".into(),
        })
    );
    assert_eq!(ended(&got), Some(Then::Ask));
    // the model saying "Yes." decides nothing: those words were not a yes of the person
    t.fake.push(says("Yes."));
    let got = t.hear("approve it now I said").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert_eq!(said(&got), "Yes.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    assert!(!t.mind.pending());
    // nor does a model's own Yes/No question
    t.fake.push(says(
        "DO {\"act\":\"ask\",\"question\":\"Approve a1?\",\"options\":[\"Yes\",\"No\"]}",
    ));
    t.hear("ask me first").await;
    assert!(t.mind.pending());
    t.fake.push(approve_a1());
    let got = t.hear("yes").await;
    assert!(
        t.fake.last_prompt().contains("The person chose: Yes"),
        "{}",
        t.fake.last_prompt()
    );
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // Dots asks again, and only that yes decides
    assert_eq!(
        asks(&got).map(|a| a.0),
        Some("Research wants to use Bash on ls -la. Approve this request?".into())
    );
    // a spoken position is not a yes: it is a new turn for the model
    for words in ["one", "first"] {
        t.fake.push(says("Hm."));
        let got = t.hear(words).await;
        assert_eq!(got.first(), Some(&MindEvent::Cleared), "{words}");
        assert!(!t.mind.pending(), "{words}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
        t.fake.push(approve_a1());
        t.hear("approve the research one").await;
    }
    // "okay" is not a clear yes
    let got = t.hear("okay").await;
    assert_eq!(said(&got), "I need a clear yes for that. Say yes, or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    assert_eq!(said(&t.hear("okay").await), "Okay, I won't.");
    assert!(!t.mind.pending());
    // a model reply landing after a newer turn asks nothing and leaves nothing waiting
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(held(approve_a1(), &hold));
    t.fake.push(says("It is noon."));
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let view = home();
    let calls = t.fake.calls();
    let first = t.mind.hear(&t.rt, "approve the research one", &view, &tx1);
    let second = async {
        while t.fake.calls() == calls {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.hear(&t.rt, "what time is it", &view, &tx2).await;
        // the first reply lands only now, after the newer turn
        hold.notify_one();
    };
    let _ = tokio::join!(first, second);
    let one = drain(&mut rx1);
    assert!(asks(&one).is_none(), "{one:?}");
    assert_eq!(said(&drain(&mut rx2)), "It is noon.");
    assert!(!t.mind.pending());
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "Approved.");
    assert_eq!(ended(&got), Some(Then::FollowUp));
    assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
    t.rt.shutdown();
}

#[tokio::test]
async fn position_words_do_not_pass_a_confirmation() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    for words in ["one", "first", "the first one", "number 1", "last", "two"] {
        t.fake.push(says("Hm."));
        let calls = t.fake.calls();
        let got = t.hear(words).await;
        assert_eq!(got.first(), Some(&MindEvent::Cleared), "{words}");
        // the words went on to the model as a new turn
        assert_eq!(t.fake.calls(), calls + 1, "{words}");
        assert!(!t.mind.pending(), "{words}");
        assert_eq!(
            t.status(&approval).await,
            ApprovalStatus::Pending,
            "{words}"
        );
        t.fake.push(approve_a1());
        t.hear("approve the research one").await;
    }
    // a click on Yes is a yes
    let got = t.pick(0).await;
    assert_eq!(said(&got), "Approved.");
    t.rt.shutdown();
}

#[tokio::test]
async fn position_words_do_not_send_an_unsure_answer() {
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    let q = t.question(&dot, "Which branch?").await;
    t.fake.push(says(
        "DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"main\",\"unsure\":true}",
    ));
    t.hear("mmm main I guess").await;
    assert!(t.mind.pending());
    t.hear("first").await;
    assert!(!t.mind.pending());
    let m = t.rt.store().get_message(&q.id).await.unwrap();
    assert!(m.answered_at.is_none());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_weak_yes_asks_for_a_clear_one() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let got = t.hear("okay").await;
    assert_eq!(said(&got), "I need a clear yes for that. Say yes, or no.");
    assert_eq!(ended(&got), Some(Then::Ask));
    assert!(!got.contains(&MindEvent::Cleared));
    assert!(t.mind.pending());
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // a second weak one drops it
    let got = t.hear("sure").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert_eq!(said(&got), "Okay, I won't.");
    assert_eq!(ended(&got), Some(Then::Rest));
    assert!(!t.mind.pending());
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // a weak one, then a firm one
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    assert_eq!(
        said(&t.hear("right").await),
        "I need a clear yes for that. Say yes, or no."
    );
    assert_eq!(said(&t.hear("yes please").await), "Approved.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
    t.rt.shutdown();
}

#[tokio::test]
async fn anything_but_yes_drops_the_confirmation() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    t.fake.push(says("The backup is idle."));
    let got = t.hear("what about the backup").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert!(got.contains(&MindEvent::Thinking));
    assert_eq!(said(&got), "The backup is idle.");
    assert!(!t.mind.pending());
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // no
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    assert!(t.mind.pending());
    let got = t.hear("no").await;
    assert_eq!(said(&got), "Okay, I won't.");
    assert_eq!(ended(&got), Some(Then::Rest));
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn lapse_drops_the_confirmation() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    t.mind.lapse(&tx);
    assert_eq!(drain(&mut rx), vec![MindEvent::Cleared]);
    assert!(!t.mind.pending());
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn lapse_during_a_turn_leaves_nothing_pending() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    // the reply lands after the stage closed
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(held(approve_a1(), &hold));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let heard = t.mind.hear(&t.rt, "approve the research one", &view, &tx);
    let closed = async {
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (gone, _) = mpsc::unbounded_channel();
        t.mind.lapse(&gone);
        hold.notify_one();
    };
    let _ = tokio::join!(heard, closed);
    let got = drain(&mut rx);
    assert!(asks(&got).is_none(), "{got:?}");
    assert!(!t.mind.pending());
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

/// A newer input (a click, a sentence) arrived at the stage while a turn was in flight: that
/// turn's question is never shown there, so its step is not installed, and a click meant for
/// the question that did show cannot answer it.
#[tokio::test]
async fn a_turn_overtaken_by_a_newer_input_installs_no_step() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(held(approve_a1(), &hold));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let heard = t.mind.hear(&t.rt, "approve the research one", &view, &tx);
    let clicked = async {
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.input_arrived();
        hold.notify_one();
    };
    let _ = tokio::join!(heard, clicked);
    drain(&mut rx);
    assert!(!t.mind.pending());
    // The click that overtook it (Yes on what showed before) is applied to nothing.
    t.pick(0).await;
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    // A turn begun after the input installs its step as ever.
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    assert!(t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_stale_confirmation_is_not_passed() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    asked_ago(&t, PENDING_LIMIT + Duration::from_secs(1));
    let got = t.hear("yes").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    assert!(!t.mind.pending());
    // a click on a stale one does nothing either
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    asked_ago(&t, PENDING_LIMIT + Duration::from_secs(1));
    t.pick(0).await;
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_cancelled_turn_sends_nothing() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    // the model's answer lands after the person said "cancel"
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(held(
        says("Sending.\nDO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"deploy\"}"),
        &hold,
    ));
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let view = home();
    let first = t.mind.hear(&t.rt, "tell research to deploy", &view, &tx1);
    let second = async {
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.hear(&t.rt, "cancel", &view, &tx2).await;
        hold.notify_one();
    };
    let _ = tokio::join!(first, second);
    assert_eq!(t.runs(&dot).await, 0);
    assert_eq!(said(&drain(&mut rx1)), "");
    assert_eq!(said(&drain(&mut rx2)), "Okay.");
    // cancel() alone, with the answer in, sends nothing either
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(held(
        says("DO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"deploy\"}"),
        &hold,
    ));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let heard = t.mind.hear(&t.rt, "tell research to deploy", &view, &tx);
    let calls = t.fake.calls();
    let stop = async {
        while t.fake.calls() == calls {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.cancel();
        hold.notify_one();
    };
    let _ = tokio::join!(heard, stop);
    assert_eq!(t.runs(&dot).await, 0);
    let got = drain(&mut rx);
    assert_eq!(said(&got), "");
    assert_eq!(ended(&got), Some(Then::Rest));
    t.rt.shutdown();
}

#[tokio::test]
async fn double_yes_reports_the_deed_once() {
    for twice in ["click", "say"] {
        let t = T::start().await;
        let approval = asking_to_approve(&t).await;
        let (tx1, mut rx1) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let view = home();
        if twice == "click" {
            let _ = tokio::join!(
                t.mind.pick(&t.rt, 0, &view, &tx1),
                t.mind.pick(&t.rt, 0, &view, &tx2)
            );
        } else {
            let _ = tokio::join!(
                t.mind.hear(&t.rt, "yes", &view, &tx1),
                t.mind.hear(&t.rt, "yes", &view, &tx2)
            );
        }
        let one = drain(&mut rx1);
        let two = drain(&mut rx2);
        assert_eq!(said(&one), "Approved.", "{twice}: {one:?}");
        assert_eq!(ended(&one), Some(Then::FollowUp), "{twice}");
        // the second yes is that same answer: nothing more is said
        assert!(two.is_empty(), "{twice}: {two:?}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
        t.rt.shutdown();
    }
}

#[tokio::test]
async fn reset_and_idle_say_what_they_dropped() {
    let mut t = T::start().await;
    // short, so moving a time this far back needs no long uptime
    t.mind.idle_forget = Duration::from_secs(5);
    asking_to_approve(&t).await;
    assert!(t.mind.reset());
    assert!(!t.mind.pending());
    assert!(!t.mind.reset());
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    assert!(t.mind.pending());
    // the person last spoke longer ago than the idle period (and the step is still in time)
    heard_ago(&t, Duration::from_secs(6));
    let got = t.hear("yes").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert!(!t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn pick_by_click_equals_saying_it() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let got = t.pick(1).await;
    assert_eq!(said(&got), "Okay, I won't.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.fake.push(approve_a1());
    t.hear("approve the research one").await;
    let got = t.pick(0).await;
    assert_eq!(said(&got), "Approved.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
    // a pick with nothing offered does nothing, and is no turn
    let got = t.pick(0).await;
    assert_eq!(got, vec![]);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_pick_past_the_options_keeps_what_waits() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    assert_eq!(t.pick(5).await, vec![]);
    assert!(t.mind.pending());
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

/// A click the mind refuses (an option no longer offered) is still the newest turn at the
/// stage, which drops every older turn's events: an older turn still in flight at the mind is
/// ended, so its late reply can neither ask nor leave a step waiting that nobody was shown.
#[tokio::test]
async fn a_refused_pick_ends_the_turn_in_flight() {
    let t = T::start().await;
    let backup = t.dot("Backup").await;
    // "Run Backup now?" lands late, after the click
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake
        .push(held(says("DO {\"act\":\"run\",\"d\":\"d1\"}"), &hold));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let heard = t.mind.hear(&t.rt, "run the backup dot", &view, &tx);
    let click = async {
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // nothing is offered: the click is refused
        let clicked = t.pick(0).await;
        hold.notify_one();
        clicked
    };
    let (_, clicked) = tokio::join!(heard, click);
    assert_eq!(clicked, vec![]);
    let got = drain(&mut rx);
    assert!(asks(&got).is_none(), "{got:?}");
    assert!(!t.mind.pending(), "a step waits that was never shown");
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.runs(&backup).await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn which_dot_then_still_confirms_a_run() {
    let t = T::start().await;
    let deploy = t.dot("Deploy watcher").await;
    t.dot("Backup").await;
    t.fake
        .push(says("Running it.\nDO {\"act\":\"run\",\"d\":\"deploy\"}"));
    let got = t.hear("run deploy").await;
    assert_eq!(
        asks(&got),
        Some((
            "Which dot did you mean?".into(),
            vec!["Deploy watcher".to_string()]
        ))
    );
    let got = t.hear("deploy watcher").await;
    assert_eq!(
        asks(&got),
        Some(("Run Deploy watcher now?".into(), yes_no()))
    );
    assert_eq!(t.runs(&deploy).await, 0);
    assert_eq!(t.fake.calls(), 1);
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "Deploy watcher is running.");
    assert_eq!(t.runs(&deploy).await, 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn filler_once_when_slow() {
    let mut t = T::start().await;
    t.mind.filler_after = Duration::from_millis(50);
    // the reply waits until the filler was sent: no race between the two timers
    let hold = Arc::new(tokio::sync::Notify::new());
    t.fake.push(Scripted {
        hold: Some(hold.clone()),
        ..says("Here it is.")
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let heard = t.mind.hear(&t.rt, "how are things", &view, &tx);
    let mut got = Vec::new();
    let release = async {
        while let Some(e) = rx.recv().await {
            let filler = matches!(e, MindEvent::Filler { .. });
            got.push(e);
            if filler {
                hold.notify_one();
                break;
            }
        }
    };
    tokio::join!(heard, release);
    got.extend(drain(&mut rx));
    let fillers: Vec<usize> = got
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(e, MindEvent::Filler { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(fillers.len(), 1, "{got:?}");
    assert_eq!(
        got[fillers[0]],
        MindEvent::Filler {
            text: "One moment.".into()
        }
    );
    let first_say = got
        .iter()
        .position(|e| matches!(e, MindEvent::Say { .. }))
        .unwrap();
    assert!(fillers[0] < first_say);
    t.rt.shutdown();
}

#[tokio::test]
async fn no_filler_when_fast() {
    let t = T::start().await;
    t.fake.push(says("Here it is."));
    let got = t.hear("how are things").await;
    assert!(
        !got.iter().any(|e| matches!(e, MindEvent::Filler { .. })),
        "{got:?}"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn cli_missing_says_so_and_leaves_nothing_pending() {
    let t = T::start().await;
    asking_to_approve(&t).await;
    t.fake.push(fails(OnceFailure::NoCli));
    let got = t.hear("what needs me").await;
    assert_eq!(
        said(&got),
        "I can't think right now: the Claude CLI isn't set up. Open Settings to fix it."
    );
    assert_eq!(ended(&got), Some(Then::Rest));
    assert!(!t.mind.pending());
    t.fake.push(fails(OnceFailure::TimedOut));
    assert_eq!(
        said(&t.hear("hello").await),
        "That took too long. Try again."
    );
    // the CLI's own words are never said nor sent
    t.fake.push(fails(OnceFailure::Failed(
        "token sk-live-123 at C:\\Users\\pat".into(),
    )));
    let got = t.hear("hello").await;
    assert_eq!(said(&got), "Something went wrong reaching Claude.");
    assert!(!format!("{got:?}").contains("sk-live"));
    t.fake.push(fails(OnceFailure::Cancelled));
    let got = t.hear("hello").await;
    assert_eq!(said(&got), "");
    assert_eq!(ended(&got), Some(Then::Rest));
    assert!(!t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn cap_reached_skips_the_model_but_quick_answers_work() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let settings = Settings {
        spend_daily_usd: Some(0.01),
        ..t.rt.settings().await
    };
    t.rt.update_settings(settings).await.unwrap();
    t.rt.store()
        .add_open_dot_usage(
            "haiku",
            &OnceUsage {
                tokens_in: 1,
                tokens_out: 1,
                cache_read: 0,
                cost_usd: Some(1.0),
            },
        )
        .await
        .unwrap();
    let calls = t.fake.calls();
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "Approved.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Approved);
    let got = t.hear("what needs me").await;
    assert_eq!(
        said(&got),
        "You've reached today's spending limit, so I can't think right now."
    );
    assert_eq!(ended(&got), Some(Then::Rest));
    assert!(!got.contains(&MindEvent::Thinking));
    assert_eq!(t.fake.calls(), calls);
    t.rt.shutdown();
}

#[tokio::test]
async fn idle_forgets_turns() {
    let mut t = T::start().await;
    t.mind.idle_forget = Duration::from_secs(5);
    t.fake.push(says("First answer."));
    t.hear("first question").await;
    t.hear("second question").await;
    assert!(t.fake.last_prompt().contains("Person: first question"));
    // the person last spoke longer ago than the idle period
    heard_ago(&t, Duration::from_secs(6));
    t.hear("third question").await;
    let p = t.fake.last_prompt();
    assert!(!p.contains("Earlier in this conversation"), "{p}");
    let got = t.hear("say that again").await;
    assert_eq!(said(&got), "Okay.");
    t.rt.shutdown();
}

#[tokio::test]
async fn second_hear_cancels_the_first() {
    let t = T::start().await;
    t.fake
        .push(slow(says("The first answer."), Duration::from_secs(20)));
    t.fake.push(says("The second answer."));
    let (tx1, mut rx1) = mpsc::unbounded_channel();
    let (tx2, mut rx2) = mpsc::unbounded_channel();
    let view = home();
    let first = t.mind.hear(&t.rt, "first", &view, &tx1);
    let second = async {
        // the first is thinking by now
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.hear(&t.rt, "second", &view, &tx2).await
    };
    let started = std::time::Instant::now();
    tokio::join!(first, second);
    assert!(started.elapsed() < Duration::from_secs(10));
    let one = drain(&mut rx1);
    let two = drain(&mut rx2);
    assert_eq!(one, vec![MindEvent::Thinking], "{one:?}");
    assert_eq!(said(&two), "The second answer.");
    assert!(lock(&t.fake.cancels)[0].is_cancelled());
    t.rt.shutdown();
}

#[tokio::test]
async fn usage_recorded_per_request() {
    let t = T::start().await;
    t.fake.push(says("One."));
    t.fake.push(Scripted {
        usage: None,
        ..says("Two.")
    });
    t.fake.push(fails(OnceFailure::TimedOut));
    t.hear("one").await;
    t.hear("two").await;
    t.hear("three").await;
    // quick answers ask nothing
    t.hear("yes").await;
    let used = t.rt.store().open_dot_usage_since(None).await.unwrap();
    assert_eq!(used.map(|u| (u.requests, u.tokens_in)), Some((1, 10)));
    t.rt.shutdown();
}

#[tokio::test]
async fn block_text_is_never_in_a_say_event() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    for reply in [
        "Running it.\nDO {\"act\":\"run\",\"d\":\"d1\"}",
        "Running it.\n```\nDO {\"act\":\"run\",\"d\":\"d1\"}\n```",
        "Running it.\nDO {\"act\":\"run\"",
    ] {
        t.fake.push(says(reply));
        let got = t.hear("run research").await;
        let words = said(&got);
        assert!(words.starts_with("Running it."), "{words:?}");
        for bad in ["DO", "{", "act", "```"] {
            assert!(!words.contains(bad), "{bad} in {words:?}");
        }
        t.hear("no").await;
    }
    assert_eq!(t.runs(&dot).await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn words_streamed_are_not_said_twice() {
    let t = T::start().await;
    // the stream and the final text differ: only what streamed, and the held end, are said
    t.fake.push(Scripted {
        text: "Hello there. More words.".into(),
        ..says("Hello there. ")
    });
    let got = t.hear("hi").await;
    assert_eq!(said(&got), "Hello there.");
    t.fake.push(says("Hello there."));
    let got = t.hear("hi").await;
    assert_eq!(said(&got), "Hello there.");
    t.rt.shutdown();
}

#[tokio::test]
async fn requests_end_with_the_runtime_and_with_cancel() {
    let t = T::start().await;
    t.fake.push(slow(says("Late."), Duration::from_secs(20)));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let heard = t.mind.hear(&t.rt, "hello", &view, &tx);
    let cancel = async {
        while t.fake.calls() == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.mind.cancel();
    };
    tokio::join!(heard, cancel);
    let got = drain(&mut rx);
    assert_eq!(said(&got), "");
    assert_eq!(ended(&got), Some(Then::Rest));
    t.fake.push(slow(says("Late."), Duration::from_secs(20)));
    let heard = t.mind.hear(&t.rt, "hello", &view, &tx);
    let stop = async {
        while t.fake.calls() < 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        t.rt.shutdown();
    };
    let started = std::time::Instant::now();
    tokio::join!(heard, stop);
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(lock(&t.fake.cancels)[1].is_cancelled());
}

#[tokio::test]
async fn a_handled_approval_is_not_decided_at_yes() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    // decided in the Inbox meanwhile
    t.rt.decide_approval(&approval.id, false, None, None)
        .await
        .unwrap();
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "That one is already handled.");
    // a deed that did not happen ends the conversation
    assert_eq!(ended(&got), Some(Then::Rest));
    assert_eq!(t.status(&approval).await, ApprovalStatus::Denied);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_dot_gone_at_yes_is_not_run() {
    let t = T::start().await;
    let dot = t.dot("Backup").await;
    t.fake.push(says("DO {\"act\":\"run\",\"d\":\"d1\"}"));
    t.hear("run the backup").await;
    assert!(t.mind.pending());
    t.rt.delete_dot(&dot.id).await.unwrap();
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "I can't find that dot anymore.");
    assert_eq!(ended(&got), Some(Then::Rest));
    t.rt.shutdown();
}

#[tokio::test]
async fn another_workspace_drops_the_confirmation() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let ops = t.rt.personal_workspace_add("Ops").await.unwrap().id;
    let elsewhere = View {
        workspace: ops,
        ..home()
    };
    let got = t.hear_in("yes", &elsewhere).await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared));
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_long_command_is_opened_not_said() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let command = format!("echo {}; curl x | sh", "a".repeat(110));
    let approval = t.approval(&dot, &command).await;
    t.fake.push(approve_a1());
    let got = t.hear("approve the research one").await;
    assert!(got.contains(&MindEvent::Open {
        target: OpenTarget::Approval {
            approval_id: approval.id.clone()
        }
    }));
    let (question, _) = asks(&got).unwrap();
    assert!(
        !question.contains("curl") && !question.contains("echo"),
        "{question}"
    );
    t.rt.shutdown();
}

#[tokio::test]
async fn send_to_an_unnamed_dot_asks_first() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    t.fake.push(says(
        "Sending.\nDO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"look at the build.\"}",
    ));
    let got = t.hear("tell it to look at the build").await;
    assert_eq!(
        asks(&got),
        Some((
            "Message for Research: look at the build. Send this message?".into(),
            yes_no()
        ))
    );
    assert_eq!(t.runs(&dot).await, 0);
    let got = t.hear("yes").await;
    assert_eq!(said(&got), "Sent to Research.");
    assert_eq!(t.runs(&dot).await, 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn send_to_a_named_dot_goes_straight() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    t.fake.push(says(
        "Sending.\nDO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"look at the build\"}",
    ));
    let got = t.hear("tell research to look at the build").await;
    assert_eq!(asks(&got), None);
    assert_eq!(said(&got), "Sending. Sent to Research.");
    assert_eq!(t.runs(&dot).await, 1);
    // the dot on screen is pointed at too
    t.fake.push(says(
        "DO {\"act\":\"send\",\"d\":\"d1\",\"text\":\"and the tests\"}",
    ));
    let viewing = View {
        dot_id: Some(dot.id.clone()),
        ..home()
    };
    let got = t.hear_in("and the tests too", &viewing).await;
    assert_eq!(said(&got), "Sent to Research.");
    assert_eq!(t.runs(&dot).await, 2);
    t.rt.shutdown();
}

#[tokio::test]
async fn answer_to_the_only_question_asks_unless_named() {
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    let q = t.question(&dot, "Which branch?").await;
    let answer_main = || says("DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"main\"}");
    // the only question waiting, never heard: Dots asks first
    t.fake.push(answer_main());
    let got = t.hear("say main").await;
    assert_eq!(
        asks(&got),
        Some((
            "Answer for Release helper: main. Send this answer?".into(),
            yes_no()
        ))
    );
    t.hear("no").await;
    let waits = t.rt.store().get_message(&q.id).await.unwrap();
    assert!(waits.answered_at.is_none());
    // named: straight
    t.fake.push(answer_main());
    let got = t.hear("tell the release helper main").await;
    assert_eq!(said(&got), "Answered Release helper.");
    let answered = t.rt.store().get_message(&q.id).await.unwrap();
    assert!(answered.answered_at.is_some());
    t.rt.shutdown();
}

#[tokio::test]
async fn read_aloud_counts_only_for_the_turn_after() {
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    t.question(&dot, "Which branch?").await;
    let read = || says("DO {\"act\":\"read_question\",\"q\":\"q1\"}");
    let answer_main = || says("DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"main\"}");
    // read, then "stop": the reading is two turns back
    t.fake.push(read());
    t.hear("what does it ask").await;
    t.hear("stop").await;
    t.fake.push(answer_main());
    let got = t.hear("say main").await;
    assert!(asks(&got).is_some(), "{got:?}");
    t.hear("no").await;
    // read, then read again by "say that again": the turn before read it
    t.fake.push(read());
    t.hear("what does it ask").await;
    let again = t.hear("say that again").await;
    assert_eq!(said(&again), "Release helper asks: Which branch?");
    t.fake.push(answer_main());
    let got = t.hear("say main").await;
    assert_eq!(said(&got), "Answered Release helper.");
    t.rt.shutdown();
}

/// The handle of the dot named `name` in the picture of now.
async fn handle_of(t: &T, name: &str) -> String {
    let p = Picture::take(&t.rt, &home()).await.unwrap();
    p.dots
        .iter()
        .find(|d| d.name == name)
        .map(|d| d.handle.clone())
        .expect(name)
}

fn send_to(handle: &str) -> Scripted {
    says(&format!(
        "DO {{\"act\":\"send\",\"d\":\"{handle}\",\"text\":\"look at it\"}}"
    ))
}

#[tokio::test]
async fn common_words_do_not_name_a_dot() {
    let t = T::start().await;
    for name in [
        "The helper",
        "Yes please",
        "Deploy watcher",
        "Deploy checker",
    ] {
        t.dot(name).await;
    }
    let cases = [
        ("The helper", "send the message to it", false),
        ("The helper", "tell the dot to look at it please", false),
        ("Yes please", "yes please tell it to look", false),
        ("Yes please", "ok yes please ask it", false),
        // "deploy" is in two dots' names: it names neither
        ("Deploy watcher", "tell deploy to look at it", false),
        ("Deploy watcher", "tell the watcher to look at it", true),
        ("The helper", "tell the helper to look at it", true),
    ];
    for (name, words, straight) in cases {
        let handle = handle_of(&t, name).await;
        t.fake.push(send_to(&handle));
        let got = t.hear(words).await;
        if straight {
            assert_eq!(said(&got), format!("Sent to {name}."), "{words}");
            assert!(asks(&got).is_none(), "{words}");
        } else {
            assert_eq!(
                asks(&got).map(|a| a.0),
                Some(format!(
                    "Message for {name}: look at it. Send this message?"
                )),
                "{words}"
            );
            t.hear("no").await;
        }
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn a_clicked_model_option_names_nothing() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    let ask = || {
        says("DO {\"act\":\"ask\",\"question\":\"Which one?\",\"options\":[\"Research\",\"Something else\"]}")
    };
    t.fake.push(ask());
    t.hear("send a note").await;
    assert!(t.mind.pending());
    // the option's words were written by the model: a click on it names nothing
    t.fake.push(send_to("d1"));
    let got = t.pick(0).await;
    assert_eq!(
        asks(&got),
        Some((
            "Message for Research: look at it. Send this message?".into(),
            yes_no()
        ))
    );
    assert_eq!(t.runs(&dot).await, 0);
    t.hear("no").await;
    // the person saying it does
    t.fake.push(ask());
    t.hear("send a note").await;
    t.fake.push(send_to("d1"));
    let got = t.hear("research").await;
    assert_eq!(said(&got), "Sent to Research.");
    assert_eq!(t.runs(&dot).await, 1);
    t.rt.shutdown();
}

#[tokio::test]
async fn answer_to_an_unmentioned_question_asks_first() {
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    let other = t.dot("Backup").await;
    let q = t.question(&dot, "Which branch?").await;
    // newest first: the other dot's question is q1
    tokio::time::sleep(Duration::from_millis(20)).await;
    t.question(&other, "Which folder?").await;
    t.fake.push(says(
        "DO {\"act\":\"answer\",\"q\":\"q2\",\"text\":\"main\"}",
    ));
    let got = t.hear("say main").await;
    assert_eq!(
        asks(&got),
        Some((
            "Answer for Release helper: main. Send this answer?".into(),
            yes_no()
        ))
    );
    assert!(t
        .rt
        .store()
        .get_message(&q.id)
        .await
        .unwrap()
        .answered_at
        .is_none());
    let got = t.hear("no").await;
    assert_eq!(said(&got), "Okay, I won't.");
    // read aloud in the turn before: pointed at
    t.fake
        .push(says("DO {\"act\":\"read_question\",\"q\":\"q2\"}"));
    let got = t.hear("what does the release one ask").await;
    assert_eq!(said(&got), "Release helper asks: Which branch?");
    t.fake.push(says(
        "DO {\"act\":\"answer\",\"q\":\"q2\",\"text\":\"main\"}",
    ));
    let got = t.hear("say main").await;
    assert_eq!(said(&got), "Answered Release helper.");
    t.rt.shutdown();
}

#[tokio::test]
async fn an_unsure_answer_is_read_back_and_no_sends_nothing() {
    let t = T::start().await;
    let dot = t.dot("Release helper").await;
    let q = t.question(&dot, "Which branch?").await;
    t.fake.push(says(
        "DO {\"act\":\"answer\",\"q\":\"q1\",\"text\":\"main\",\"unsure\":true}",
    ));
    let got = t.hear("mmm main I guess").await;
    assert_eq!(
        asks(&got),
        Some(("I heard: main. Send this answer?".into(), yes_no()))
    );
    let got = t.hear("no").await;
    assert_eq!(said(&got), "Okay, not sent.");
    let m = t.rt.store().get_message(&q.id).await.unwrap();
    assert!(m.answered_at.is_none());
    t.rt.shutdown();
}

#[tokio::test]
async fn what_needs_me_lists_and_opens_the_inbox_for_private_ones() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    t.approval(&dot, "ls -la").await;
    t.approval(&dot, "cat ~/.ssh/id_rsa").await;
    t.fake
        .push(says("Here's what needs you.\nDO {\"act\":\"needs\"}"));
    let got = t.hear("what needs me").await;
    assert_eq!(
        said(&got),
        "Here's what needs you. Two things need you. Research wants approval to use Bash. \
         One private approval needs you."
    );
    assert!(got.iter().any(|e| matches!(
        e,
        MindEvent::Card {
            card: Card::List { lines, .. }
        } if lines.len() == 2
    )));
    assert!(got.contains(&MindEvent::Open {
        target: OpenTarget::Inbox
    }));
    assert!(!format!("{got:?}").contains("ssh"));
    t.rt.shutdown();
}

#[tokio::test]
async fn an_open_the_model_already_announced_is_not_said_twice() {
    let t = T::start().await;
    t.fake.push(says(
        "Opening the inbox.\nDO {\"act\":\"open\",\"target\":\"inbox\"}",
    ));
    let got = t.hear("show me the inbox please").await;
    assert_eq!(said(&got), "Opening the inbox.");
    assert!(got.contains(&MindEvent::Open {
        target: OpenTarget::Inbox
    }));
    assert_eq!(ended(&got), Some(Then::FollowUp));

    // Said in the model's own words, after other words: still once.
    t.fake.push(says(
        "Sure. Opening your Inbox now.\nDO {\"act\":\"open\",\"target\":\"inbox\"}",
    ));
    let got = t.hear("show me the inbox please").await;
    assert_eq!(said(&got), "Sure. Opening your Inbox now.");
    // Another screen named: Dots says where it really went.
    t.fake.push(says(
        "Opening Settings.\nDO {\"act\":\"open\",\"target\":\"inbox\"}",
    ));
    let got = t.hear("show me the inbox please").await;
    assert_eq!(said(&got), "Opening Settings. Opening the Inbox.");

    // Other words before it: Dots' own line is still said.
    t.fake.push(says(
        "Sure thing.\nDO {\"act\":\"open\",\"target\":\"inbox\"}",
    ));
    let got = t.hear("show me the inbox please").await;
    assert_eq!(said(&got), "Sure thing. Opening the Inbox.");
    // No words at all: Dots' line alone.
    t.fake
        .push(says("DO {\"act\":\"open\",\"target\":\"settings\"}"));
    let got = t.hear("take me to the settings page").await;
    assert_eq!(said(&got), "Opening Settings.");
    t.rt.shutdown();
}

#[tokio::test]
async fn a_bad_block_with_no_words_asks_again() {
    let t = T::start().await;
    t.fake.push(says("DO {\"act\":\"fly\"}"));
    let got = t.hear("do the thing").await;
    assert_eq!(
        said(&got),
        "I didn't catch what to do with that. Say it another way?"
    );
    assert_eq!(ended(&got), Some(Then::Rest));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_bad_block_after_words_says_it_was_not_done() {
    let t = T::start().await;
    t.fake.push(says("Running it.\nDO {\"act\":\"fly\"}"));
    let got = t.hear("do the thing").await;
    assert_eq!(
        said(&got),
        "Running it. I didn't catch what to do with that. Say it another way?"
    );
    assert_eq!(ended(&got), Some(Then::Rest));
    t.rt.shutdown();
}

#[test]
fn create_question_says_only_the_name_and_when() {
    use crate::library::Uses;
    use crate::policy::Preset;
    let p = SetupProposal {
        name: "Downloads **digest**\n".into(),
        instructions: "1. List.".into(),
        folder: None,
        schedule: Some(
            serde_json::from_value(json!({"rule": {"kind": "daily", "time": "09:00"}})).unwrap(),
        ),
        preset: Preset::Trusted,
        extras: vec![],
        uses: Uses::default(),
        first_message: format!("Tell me what is in Downloads. {}", "And more. ".repeat(40)),
        notes: String::new(),
    };
    // the first message, instructions and access are on the card, never partly spoken
    assert_eq!(
        create_question(&p),
        "Downloads digest. Runs every day at 09:00. The rest is on screen. Create this dot?"
    );
    let none = SetupProposal {
        name: "N".repeat(200),
        schedule: None,
        first_message: String::new(),
        ..p
    };
    assert_eq!(
        create_question(&none),
        format!(
            "{}. Runs when you ask. The rest is on screen. Create this dot?",
            "N".repeat(80)
        )
    );
}

/// A chatty model (or one steered by text in a record) says at most 200 bytes before Dots'
/// own question; with no question to follow, its words are said whole.
#[tokio::test]
async fn words_before_a_confirmation_are_cut_to_200_bytes() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    t.approval(&dot, "ls -la").await;
    let chat = "This approval is for listing files in the folder, which is harmless. \
        I checked it twice and it looks fine to me. Shall I go ahead? \
        It will only read the names of the files. Nothing is changed or sent anywhere. \
        You can always deny it later if you change your mind about it. ";
    assert!(chat.len() > 250);
    t.fake.push(says(&format!(
        "{chat}\nDO {{\"act\":\"approve\",\"a\":\"a1\"}}"
    )));
    let got = t.hear("approve the research one").await;
    let words = said(&got);
    assert!(
        words.len() <= reply::BEFORE_A_YES_BYTES,
        "{} bytes",
        words.len()
    );
    assert!(words.len() > 150, "{words}");
    assert!(chat.starts_with(&words) && words.ends_with('.'), "{words}");
    assert_eq!(
        asks(&got).map(|a| a.0),
        Some("Research wants to use Bash on ls -la. Approve this request?".into())
    );
    // the same words with nothing to confirm are all said
    t.fake.push(says(chat));
    let got = t.hear("tell me about it").await;
    assert_eq!(said(&got), chat.trim_end());
    t.rt.shutdown();
}

/// No confirmation question Dots builds ends in words that pass a step of any kind: a
/// microphone that opens while the end of the question is still sounding (a speaker with a
/// long output delay) may hear its last words, and those must never be a yes. Every tail of
/// every question, one word or more, is tried against every kind and the quick matcher.
#[test]
fn no_confirmation_question_ends_in_a_yes() {
    use crate::library::Uses;
    use crate::open_dot::picture::{ApprovalLine, DotLine, QuestionLine};
    use crate::policy::Preset;
    let approval = |target: &str, cut: bool| ApprovalLine {
        handle: "a1".into(),
        id: "ap-1".into(),
        dot_id: "id-1".into(),
        dot_name: "Research".into(),
        tool: "Bash".into(),
        target: target.into(),
        target_cut: cut,
        target_summary: false,
    };
    let picture = |a: ApprovalLine| Picture {
        dots: vec![DotLine {
            handle: "d1".into(),
            id: "id-1".into(),
            name: "Research".into(),
            status: "idle".into(),
            needs_you: 0,
        }],
        approvals: vec![a],
        questions: vec![QuestionLine {
            handle: "q1".into(),
            id: "m-1".into(),
            dot_id: "id-1".into(),
            dot_name: "Research".into(),
            text: "Which branch?".into(),
            choices: vec![],
        }],
        ..Picture::default()
    };
    let view = home();
    let mut questions = Vec::new();
    for a in [
        approval("npm test", false),
        approval("", false),
        approval(&"x".repeat(100), true),
    ] {
        let p = picture(a);
        for wish in [
            Wish::Approve { a: "a1".into() },
            Wish::Deny { a: "a1".into() },
            Wish::Run { d: "d1".into() },
            Wish::Answer {
                q: "q1".into(),
                text: "go ahead".into(),
                unsure: true,
            },
        ] {
            match plan(&wish, &p, &view) {
                Step::Confirm { question, .. } | Step::Ask { question, .. } => {
                    questions.push(question)
                }
                other => panic!("{wish:?} gave {other:?}"),
            }
        }
    }
    // The person's own text is "yes" and "go ahead": it is said, but never last.
    for text in ["yes", "go ahead."] {
        questions.push(check_question(&Deed::Send {
            dot_id: "id-1".into(),
            dot_name: "Research".into(),
            text: text.into(),
            topic_root: None,
        }));
        questions.push(check_question(&Deed::Answer {
            message_id: "m-1".into(),
            dot_name: "Research".into(),
            text: text.into(),
        }));
    }
    let proposal = |name: &str| SetupProposal {
        name: name.into(),
        instructions: "1. List.".into(),
        folder: None,
        schedule: None,
        preset: Preset::Trusted,
        extras: vec![],
        uses: Uses::default(),
        first_message: String::new(),
        notes: String::new(),
    };
    questions.push(create_question(&proposal("Downloads digest")));
    questions.push(create_question(&proposal("Do it")));
    questions.push(CLEAR_YES.to_string());
    assert_eq!(questions.len(), 19);
    for question in &questions {
        let words: Vec<&str> = question.split_whitespace().collect();
        for from in 0..words.len() {
            let tail = words[from..].join(" ");
            for kind in ConfirmKind::ALL {
                assert!(
                    !firm_yes(&tail, kind),
                    "{question:?} ends in {tail:?}, a yes to {kind:?}"
                );
            }
            assert_ne!(
                quick(&tail, &[]),
                Some(Quick::Yes),
                "{question:?} ends in {tail:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_panicking_thinker_still_ends_the_turn() {
    use futures_util::FutureExt;
    let t = T::start().await;
    t.fake.push(Scripted {
        panics: true,
        ..says("Never.")
    });
    let (tx, mut rx) = mpsc::unbounded_channel();
    let view = home();
    let turn = std::panic::AssertUnwindSafe(t.mind.hear(&t.rt, "hello", &view, &tx));
    assert!(turn.catch_unwind().await.is_err());
    let got = drain(&mut rx);
    assert_eq!(
        got,
        vec![MindEvent::Thinking, MindEvent::SayEnd { then: Then::Rest }]
    );
    // the mind goes on
    t.fake.push(says("Hello."));
    assert_eq!(said(&t.hear("hello").await), "Hello.");
    t.rt.shutdown();
}

#[tokio::test]
async fn every_turn_ends_once() {
    let t = T::start().await;
    let dot = t.dot("Research").await;
    t.approval(&dot, "ls -la").await;
    let turns: Vec<(&str, Option<Scripted>)> = vec![
        ("how are things", Some(says("Fine."))),
        ("approve the research one", Some(approve_a1())),
        ("okay", None),
        ("yes", None),
        ("what", None),
        ("stop", None),
        ("never mind", None),
        ("hello", Some(fails(OnceFailure::TimedOut))),
        ("do the thing", Some(says("Hm.\nDO {\"act\":\"fly\"}"))),
        ("", None),
        ("yes", None),
    ];
    for (words, reply) in turns {
        if let Some(r) = reply {
            t.fake.push(r);
        }
        let got = t.hear(words).await;
        let ends = got
            .iter()
            .filter(|e| matches!(e, MindEvent::SayEnd { .. }))
            .count();
        assert_eq!(ends, 1, "{words}: {got:?}");
        assert!(
            matches!(got.last(), Some(MindEvent::SayEnd { .. })),
            "{words}: {got:?}"
        );
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn nothing_heard_asks_nothing() {
    let t = T::start().await;
    asking_to_approve(&t).await;
    let calls = t.fake.calls();
    let got = t.hear("  ").await;
    assert_eq!(
        got,
        vec![MindEvent::Cleared, MindEvent::SayEnd { then: Then::Rest }]
    );
    assert_eq!(t.fake.calls(), calls);
    assert!(!t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn repeat_says_only_the_question_when_it_came_alone() {
    let t = T::start().await;
    t.dot("Research").await;
    t.fake.push(says("Two dots are idle."));
    t.hear("how are things").await;
    let dot = t.dot("Backup").await;
    t.approval(&dot, "ls -la").await;
    // asked with no words of the model
    t.fake.push(says("DO {\"act\":\"approve\",\"a\":\"a1\"}"));
    t.hear("approve the backup one").await;
    let got = t.hear("say that again").await;
    assert_eq!(said(&got), "");
    assert_eq!(
        asks(&got).map(|a| a.0),
        Some("Backup wants to use Bash on ls -la. Approve this request?".into())
    );
    assert!(t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn model_and_setup_questions_are_cleaned() {
    let t = T::start().await;
    let long = "x".repeat(400);
    t.fake.push(says(&format!(
        "DO {{\"act\":\"ask\",\"question\":\"**Which** `one`?\\u200b {long}\",\"options\":[\"*A*\",\"B\\u202e\",\"`#`\"]}}"
    )));
    let got = t.hear("pick something").await;
    let (question, options) = asks(&got).unwrap();
    assert!(question.starts_with("Which one? xxx"), "{question}");
    assert!(question.len() <= 300, "{}", question.len());
    assert_eq!(options, vec!["A".to_string(), "B".to_string()]);
    t.rt.shutdown();
}

#[tokio::test]
async fn markdown_is_never_said() {
    let t = T::start().await;
    t.fake
        .push(says("**Two** things `need` you. ## Done\n- the *backup*"));
    let got = t.hear("how are things").await;
    let words = said(&got);
    for mark in ["*", "`", "#"] {
        assert!(!words.contains(mark), "{mark} in {words:?}");
    }
    assert!(words.starts_with("Two things need you."), "{words}");
    t.rt.shutdown();
}

/// A dot with a waiting approval, and the mind asking to deny it.
async fn asking_to_deny(t: &T) -> Approval {
    let dot = t.dot("Research").await;
    let approval = t.approval(&dot, "ls -la").await;
    ask_to_deny_again(t).await;
    approval
}

async fn ask_to_deny_again(t: &T) {
    t.fake.push(says("DO {\"act\":\"deny\",\"a\":\"a1\"}"));
    let got = t.hear("deny the research one").await;
    assert_eq!(
        asks(&got),
        Some((
            "Research wants to use Bash on ls -la. Deny this request?".into(),
            yes_no()
        ))
    );
    assert!(t.mind.pending());
}

#[tokio::test]
async fn approve_said_to_a_deny_question_denies_nothing() {
    let t = T::start().await;
    let approval = asking_to_deny(&t).await;
    for words in ["approve", "approve it", "Approve it, please."] {
        t.fake.push(says("Hm."));
        let calls = t.fake.calls();
        let got = t.hear(words).await;
        // dropped, and the words went on to the model as a new turn
        assert_eq!(got.first(), Some(&MindEvent::Cleared), "{words}: {got:?}");
        assert_eq!(t.fake.calls(), calls + 1, "{words}");
        assert!(!said(&got).contains("Denied"), "{words}: {got:?}");
        assert!(!t.mind.pending(), "{words}");
        assert_eq!(
            t.status(&approval).await,
            ApprovalStatus::Pending,
            "{words}"
        );
        ask_to_deny_again(&t).await;
    }
    t.rt.shutdown();
}

#[tokio::test]
async fn deny_said_to_a_deny_question_denies() {
    for words in ["deny", "deny it", "deny that", "yes"] {
        let t = T::start().await;
        let approval = asking_to_deny(&t).await;
        let got = t.hear(words).await;
        assert_eq!(said(&got), "Denied.", "{words}: {got:?}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Denied, "{words}");
        t.rt.shutdown();
    }
}

#[tokio::test]
async fn go_ahead_said_to_a_deny_question_asks_for_a_clear_yes() {
    for words in ["go ahead", "go for it", "Yes, go ahead."] {
        let t = T::start().await;
        let approval = asking_to_deny(&t).await;
        let calls = t.fake.calls();
        let got = t.hear(words).await;
        assert_eq!(
            said(&got),
            "I need a clear yes for that. Say yes, or no.",
            "{words}: {got:?}"
        );
        assert_eq!(t.fake.calls(), calls, "{words}");
        assert!(t.mind.pending(), "{words}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
        // a second one gives up; nothing is denied
        let got = t.hear(words).await;
        assert_eq!(said(&got), "Okay, I won't.", "{words}: {got:?}");
        assert!(!t.mind.pending(), "{words}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
        // and a clear yes after one still denies
        ask_to_deny_again(&t).await;
        t.hear(words).await;
        assert_eq!(said(&t.hear("yes").await), "Denied.", "{words}");
        assert_eq!(t.status(&approval).await, ApprovalStatus::Denied);
        t.rt.shutdown();
    }
}

#[tokio::test]
async fn deny_said_to_an_approve_question_is_no() {
    for words in ["deny", "deny it", "deny that"] {
        let t = T::start().await;
        let approval = asking_to_approve(&t).await;
        let calls = t.fake.calls();
        let got = t.hear(words).await;
        assert_eq!(said(&got), "Okay, I won't.", "{words}: {got:?}");
        assert_eq!(t.fake.calls(), calls, "{words}");
        assert!(!t.mind.pending(), "{words}");
        assert_eq!(
            t.status(&approval).await,
            ApprovalStatus::Pending,
            "{words}"
        );
        t.rt.shutdown();
    }
}

#[tokio::test]
async fn run_and_send_words_pass_only_their_own_kind() {
    let t = T::start().await;
    let deploy = t.dot("Deploy watcher").await;
    t.fake.push(says("DO {\"act\":\"run\",\"d\":\"d1\"}"));
    t.hear("run the deploy watcher").await;
    assert!(t.mind.pending());
    // "send it" is no answer to a run: dropped, and a new turn
    t.fake.push(says("Hm."));
    let got = t.hear("send it").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared), "{got:?}");
    assert_eq!(t.runs(&deploy).await, 0);
    t.fake.push(says("DO {\"act\":\"run\",\"d\":\"d1\"}"));
    t.hear("run the deploy watcher").await;
    let got = t.hear("run it").await;
    assert_eq!(said(&got), "Deploy watcher is running.");
    assert_eq!(t.runs(&deploy).await, 1);
    t.rt.shutdown();
}

fn a_proposal() -> SetupProposal {
    SetupProposal {
        name: "Downloads digest".into(),
        instructions: "1. List.".into(),
        folder: None,
        schedule: None,
        preset: crate::policy::Preset::Trusted,
        extras: vec![],
        uses: crate::library::Uses::default(),
        first_message: String::new(),
        notes: String::new(),
    }
}

/// Puts a step that waits in place, as a turn that asked it would.
fn waiting_on(t: &T, kind: Waiting) {
    let view = home();
    lock(&t.mind.state).pending = Some(Pending {
        kind,
        account: view.account,
        workspace: view.workspace,
        asked_at: Instant::now(),
        first_asked: Instant::now(),
        retried: false,
        repeats: 0,
        words: String::new(),
    });
}

#[tokio::test]
async fn create_it_creates() {
    for words in ["create it", "make it", "yes"] {
        let t = T::start().await;
        let p = a_proposal();
        let question = create_question(&p);
        waiting_on(
            &t,
            Waiting::Create {
                proposal: p,
                question,
            },
        );
        let got = t.hear(words).await;
        assert_eq!(said(&got), "Created Downloads digest.", "{words}: {got:?}");
        assert!(!t.mind.pending());
        t.rt.shutdown();
    }
    // "approve" is not a yes to creating
    let t = T::start().await;
    let p = a_proposal();
    let question = create_question(&p);
    waiting_on(
        &t,
        Waiting::Create {
            proposal: p,
            question,
        },
    );
    t.fake.push(says("Hm."));
    let got = t.hear("approve it").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared), "{got:?}");
    assert!(!said(&got).contains("Created"), "{got:?}");
    t.rt.shutdown();
}

/// Moves the time the person last spoke `ago` into the past.
fn heard_ago(t: &T, ago: Duration) {
    lock(&t.mind.state).last_heard = Some(Instant::now() - ago);
}

/// Moves the asking time of the step that waits `ago` into the past.
fn asked_ago(t: &T, ago: Duration) {
    if let Some(p) = lock(&t.mind.state).pending.as_mut() {
        p.asked_at = Instant::now() - ago;
    }
}

#[tokio::test]
async fn repeat_renews_the_asking_time_twice() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    let old = PENDING_LIMIT - Duration::from_secs(5);
    for n in 1..=2 {
        asked_ago(&t, old);
        let got = t.hear("say that again").await;
        assert!(asks(&got).is_some(), "{n}: {got:?}");
        let fresh = lock(&t.mind.state)
            .pending
            .as_ref()
            .map(|p| p.asked_at.elapsed() < Duration::from_secs(5));
        assert_eq!(fresh, Some(true), "repeat {n} renews");
    }
    // the third asks again but does not renew
    asked_ago(&t, old);
    let got = t.hear("say that again").await;
    assert!(asks(&got).is_some(), "{got:?}");
    let renewed = lock(&t.mind.state)
        .pending
        .as_ref()
        .map(|p| p.asked_at.elapsed() < Duration::from_secs(5));
    assert_eq!(renewed, Some(false));
    asked_ago(&t, PENDING_LIMIT + Duration::from_secs(1));
    assert!(!t.mind.pending());
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

/// Time passes for the step that waits and for the person: every clock moves `ago` back.
fn time_passes(t: &T, ago: Duration) {
    let mut s = lock(&t.mind.state);
    if let Some(at) = s.last_heard.as_mut() {
        *at -= ago;
    }
    if let Some(p) = s.pending.as_mut() {
        p.asked_at -= ago;
        p.first_asked -= ago;
    }
}

#[tokio::test]
async fn a_step_never_outlives_three_waits() {
    let mut t = T::start().await;
    // short, so moving a time this far back needs no long uptime
    t.mind.pending_limit = Duration::from_secs(5);
    let approval = asking_to_approve(&t).await;
    let wait = t.mind.pending_limit - Duration::from_secs(1);
    // the worst case: say that again at 4 s and 8 s, an unclear yes at 12 s (5 s waits)
    for words in ["say that again", "say that again"] {
        time_passes(&t, wait);
        assert!(asks(&t.hear(words).await).is_some(), "{words}");
        assert!(t.mind.pending());
    }
    time_passes(&t, wait);
    assert_eq!(
        said(&t.hear("okay").await),
        "I need a clear yes for that. Say yes, or no."
    );
    assert!(t.mind.pending(), "12 s: still asking");
    // three waits after it was first asked, it is gone, renewed or not
    time_passes(&t, Duration::from_secs(3) + Duration::from_millis(10));
    assert!(!t.mind.pending(), "15 s: gone");
    let got = t.hear("yes").await;
    assert_eq!(got.first(), Some(&MindEvent::Cleared), "{got:?}");
    assert_eq!(said(&got), "There's nothing waiting on a yes or no.");
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_click_after_the_idle_forget_says_nothing_waits() {
    let mut t = T::start().await;
    // short, so moving a time this far back needs no long uptime
    t.mind.idle_forget = Duration::from_secs(5);
    let approval = asking_to_approve(&t).await;
    heard_ago(&t, Duration::from_secs(6));
    let got = t.pick(0).await;
    assert_eq!(
        got,
        vec![
            MindEvent::Cleared,
            MindEvent::Say {
                delta: "There's nothing waiting on a yes or no.".into()
            },
            MindEvent::SayEnd { then: Then::Rest }
        ]
    );
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_click_on_an_expired_step_says_nothing_waits() {
    let t = T::start().await;
    let approval = asking_to_approve(&t).await;
    asked_ago(&t, PENDING_LIMIT + Duration::from_secs(1));
    assert!(!t.mind.pending());
    let got = t.pick(0).await;
    assert_eq!(
        got,
        vec![
            MindEvent::Cleared,
            MindEvent::Say {
                delta: "There's nothing waiting on a yes or no.".into()
            },
            MindEvent::SayEnd { then: Then::Rest }
        ]
    );
    assert_eq!(t.status(&approval).await, ApprovalStatus::Pending);
    t.rt.shutdown();
}

#[tokio::test]
async fn setup_rounds_stop_at_a_cap() {
    let t = T::start().await;
    let settings = Settings {
        spend_daily_usd: Some(0.01),
        ..t.rt.settings().await
    };
    t.rt.update_settings(settings).await.unwrap();
    // the model's own request reaches the cap; the setup agent is then not asked
    t.fake.push(Scripted {
        usage: Some(OnceUsage {
            tokens_in: 1,
            tokens_out: 1,
            cache_read: 0,
            cost_usd: Some(1.0),
        }),
        ..says("Let's make it.\nDO {\"act\":\"create\",\"description\":\"digest my downloads\"}")
    });
    let got = t.hear("make a dot that digests my downloads").await;
    assert_eq!(
        said(&got),
        "Let's make it. Let me set that up. You've reached today's spending limit, so I can't \
         think right now."
    );
    assert_eq!(ended(&got), Some(Then::Rest));
    assert!(!t.mind.pending());
    t.rt.shutdown();
}

#[tokio::test]
async fn a_turn_waiting_behind_a_deed_does_not_begin_after_a_lapse_or_reset() {
    for how in ["lapse", "reset"] {
        let t = T::start().await;
        // A deed is being carried out.
        let deed = t.mind.deeds.lock().await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let view = home();
        let mut turn = std::pin::pin!(t.mind.hear(&t.rt, "run the deploy watcher", &view, &tx));
        // The turn arrived and waits for the deed.
        std::future::poll_fn(|cx| {
            let polled = std::future::Future::poll(turn.as_mut(), cx);
            assert!(polled.is_pending(), "{how}");
            std::task::Poll::Ready(())
        })
        .await;
        // The stage closed (or rested) before the deed ended.
        let (lapsed, _) = mpsc::unbounded_channel();
        match how {
            "lapse" => t.mind.lapse(&lapsed),
            _ => {
                t.mind.reset();
            }
        }
        drop(deed);
        turn.await;
        assert!(drain(&mut rx).is_empty(), "{how}: the turn said something");
        assert_eq!(t.fake.calls(), 0, "{how}: the model was asked");
        assert!(!t.mind.pending(), "{how}");
        // The next turn begins as ever.
        assert!(!t.hear("hello").await.is_empty(), "{how}");
        t.rt.shutdown();
    }
}

#[tokio::test]
async fn a_drop_after_the_checks_still_stops_the_turn_from_beginning() {
    // The drop lands between the turn's last look and its start: begin itself refuses it.
    let t = T::start().await;
    let seen = lock(&t.mind.state).drops;
    let turn = lock(&t.mind.state).turn;
    t.mind.reset();
    assert!(t.mind.begin(&t.rt, seen).is_none(), "a dropped turn began");
    assert_eq!(
        lock(&t.mind.state).turn,
        turn + 1,
        "only the reset moved the turn"
    );
    // With the count it saw after the drop, a turn begins.
    let now = lock(&t.mind.state).drops;
    assert!(t.mind.begin(&t.rt, now).is_some());
    t.rt.shutdown();
}
