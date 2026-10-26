//! Asking and replying through the runtime (check-ins spec section 5): a reply continues a
//! quiet thread from its end, starts afresh when the end has no session, is refused while the
//! thread is busy and for a closed question, and two answers at once start one run. Runs are
//! stored but never started (`max_concurrent_runs` is 0), so each test builds its thread.

mod common;

use std::path::PathBuf;

use common::spec;
use dots_core::checkins::Tool;
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, Message, MessageKind, NewRun, Run, RunStatus, TriggerKind, QUESTION_ANSWERED,
    QUESTION_DISMISSED,
};
use dots_core::store::Store;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;

struct Rt {
    _dir: TempDir,
    rt: Runtime,
}

/// A runtime that stores runs and never starts one.
async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { _dir: dir, rt }
}

impl Rt {
    fn store(&self) -> &Store {
        self.rt.store()
    }

    async fn dot(&self, name: &str) -> Dot {
        self.rt.create_dot(spec(name), None).await.unwrap()
    }

    /// A manual run of `dot` that ended with `status`, with the session `s1` when `session`.
    async fn ended(&self, dot: &Dot, status: RunStatus, session: bool) -> Run {
        let run = self
            .store()
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        if session {
            self.store().set_session(&run.id, "s1").await.unwrap();
        }
        self.store()
            .finish_run(&run.id, status, Some("Built it."), None)
            .await
            .unwrap()
    }

    /// A chat run that continues `run`; it stays queued.
    async fn chat_after(&self, run: &Run) -> Run {
        let mut new = NewRun::new(&run.dot_id, TriggerKind::Chat);
        new.payload = Some(json!({"message": "and now?"}));
        new.parent_run_id = Some(run.id.clone());
        new.root_run_id = Some(run.root_run_id.clone());
        self.store().create_run(&new).await.unwrap()
    }

    /// The question `text`, with the choices yes and no, asked in `run`.
    async fn ask(&self, run: &Run, text: &str) -> Message {
        let input = json!({"question": text, "choices": ["yes", "no"]});
        let answer = self.rt.checkins().call(&run.id, Tool::Ask, &input).await;
        assert!(answer.is_ok(), "{answer:?}");
        self.store()
            .list_messages(Some(&run.dot_id))
            .await
            .unwrap()
            .into_iter()
            .find(|m| {
                m.kind == MessageKind::Question && m.run_id.as_deref() == Some(run.id.as_str())
            })
            .unwrap()
    }
}

#[tokio::test]
async fn a_reply_continues_a_quiet_thread_from_its_end() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    assert_eq!(
        t.rt.inbox_count().await.unwrap(),
        1,
        "an open question waits"
    );
    let mut events = t.rt.subscribe();
    let reply = t.rt.reply(&question.id, " yes ").await.unwrap();
    assert_eq!(reply.trigger, TriggerKind::Chat);
    assert_eq!(reply.status, RunStatus::Queued);
    assert_eq!(reply.parent_run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(reply.root_run_id, run.root_run_id);
    assert_eq!(reply.session_id.as_deref(), Some("s1"));
    assert_eq!(
        reply.payload,
        Some(json!({"message": "Answer to your question \"Ship it?\": yes", "answer": "yes"}))
    );
    let answered = t.store().get_message(&question.id).await.unwrap();
    assert_eq!(answered.answer_run_id.as_deref(), Some(reply.id.as_str()));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
    let mut seen = Vec::new();
    while let Ok(ev) = events.try_recv() {
        seen.push(ev);
    }
    assert!(
        seen.iter()
            .any(|e| matches!(e, RuntimeEvent::RunUpdated { run } if run.id == reply.id)),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(
            e,
            RuntimeEvent::MessageUpdated { message }
                if message.id == question.id && message.answered_at.is_some()
        )),
        "{seen:?}"
    );
}

#[tokio::test]
async fn a_reply_to_a_thread_whose_end_has_no_session_starts_afresh() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let first = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&first, "Which branch?").await;
    // A chat that continued the thread failed before the dot started: no session to resume.
    let failed = t.chat_after(&first).await;
    t.store()
        .finish_run(
            &failed.id,
            RunStatus::Failed,
            None,
            Some("claude CLI not found"),
        )
        .await
        .unwrap();
    let reply = t.rt.reply(&question.id, "main").await.unwrap();
    assert_eq!(reply.trigger, TriggerKind::Chat);
    assert_eq!(reply.parent_run_id, None);
    assert_eq!(reply.root_run_id, reply.id);
    assert_eq!(reply.session_id, None);
    assert_eq!(
        reply.payload,
        Some(json!({
            "message": "Earlier you asked the person: \"Which branch?\"\nTheir answer: main",
            "answer": "main"
        }))
    );
}

#[tokio::test]
async fn a_reply_to_a_busy_thread_is_refused_with_the_words_of_chat() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    let busy = t.chat_after(&run).await;
    let refused = t.rt.reply(&question.id, "yes").await;
    assert!(
        matches!(&refused, Err(Error::Conflict(m))
            if m.starts_with("the dot is still working on this chat")),
        "{refused:?}"
    );
    t.store()
        .set_status(&busy.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();
    let refused = t.rt.reply(&question.id, "yes").await;
    assert!(
        matches!(&refused, Err(Error::Conflict(m))
            if m.starts_with("this chat is waiting for an approval")),
        "{refused:?}"
    );
    // The same words as a chat message sent to that thread now.
    let chat = t.rt.chat(&dot.id, "and now?", Some(&busy.id)).await;
    assert_eq!(
        format!("{:?}", chat.unwrap_err()),
        format!("{:?}", refused.unwrap_err())
    );
    // The question stays open, and no refused reply stored a run.
    assert!(t.store().get_message(&question.id).await.unwrap().waits());
    assert_eq!(
        t.store().list_thread(&run.root_run_id).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn a_closed_question_or_a_note_cannot_be_answered() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let answered = t.ask(&run, "Ship it?").await;
    t.rt.reply(&answered.id, "yes").await.unwrap();
    let again = t.rt.reply(&answered.id, "no").await;
    assert!(
        matches!(&again, Err(Error::Conflict(m)) if m == QUESTION_ANSWERED),
        "{again:?}"
    );

    let other = t.ended(&dot, RunStatus::Succeeded, true).await;
    let dismissed = t.ask(&other, "Which branch?").await;
    let closed = t.rt.checkins().dismiss(&dismissed.id).await.unwrap();
    assert!(closed.dismissed_at.is_some());
    let late = t.rt.reply(&dismissed.id, "main").await;
    assert!(
        matches!(&late, Err(Error::Conflict(m)) if m == QUESTION_DISMISSED),
        "{late:?}"
    );

    let third = t.ended(&dot, RunStatus::Succeeded, true).await;
    let open = t.ask(&third, "Merge it?").await;
    assert!(matches!(
        t.rt.reply(&open.id, "  ").await,
        Err(Error::Invalid(_))
    ));
    t.rt.checkins()
        .call(&third.id, Tool::Notify, &json!({"text": "Half way."}))
        .await
        .unwrap();
    let note = t
        .store()
        .list_messages(Some(&dot.id))
        .await
        .unwrap()
        .into_iter()
        .find(|m| m.kind == MessageKind::Note)
        .unwrap();
    assert!(matches!(
        t.rt.reply(&note.id, "ok").await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        t.rt.reply("no-such-message", "ok").await,
        Err(Error::NotFound(_))
    ));
}

/// Review Focus 3: a double click, or the Inbox and the chat bar together.
#[tokio::test]
async fn two_replies_at_once_start_one_run() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    let question = t.ask(&run, "Ship it?").await;
    let (a, b) = tokio::join!(
        t.rt.reply(&question.id, "yes"),
        t.rt.reply(&question.id, "no")
    );
    assert_eq!(
        [&a, &b].iter().filter(|r| r.is_ok()).count(),
        1,
        "{a:?} {b:?}"
    );
    assert!(
        matches!(
            a.as_ref().err().or(b.as_ref().err()),
            Some(Error::Conflict(_))
        ),
        "{a:?} {b:?}"
    );
    assert_eq!(
        t.store().list_thread(&run.root_run_id).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn messages_are_read_and_dismissed_and_the_waiting_ones_counted() {
    let t = start().await;
    let dot = t.dot("asker").await;
    let run = t.ended(&dot, RunStatus::Succeeded, true).await;
    t.rt.checkins()
        .call(&run.id, Tool::Notify, &json!({"text": "Half way."}))
        .await
        .unwrap();
    let question = t.ask(&run, "Ship it?").await;
    assert_eq!(t.rt.inbox_count().await.unwrap(), 2);
    let listed = t.rt.checkins().list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(listed.len(), 2);
    let note = listed.iter().find(|m| m.kind == MessageKind::Note).unwrap();
    let mut events = t.rt.subscribe();
    let read = t.rt.checkins().mark_read(&note.id).await.unwrap();
    assert!(read.read_at.is_some());
    assert!(matches!(
        events.try_recv().unwrap(),
        RuntimeEvent::MessageUpdated { message } if message.id == note.id
    ));
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    // A question that was read still waits for its answer.
    t.rt.checkins().mark_read(&question.id).await.unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 1);
    t.rt.checkins().dismiss(&question.id).await.unwrap();
    assert_eq!(t.rt.inbox_count().await.unwrap(), 0);
}
