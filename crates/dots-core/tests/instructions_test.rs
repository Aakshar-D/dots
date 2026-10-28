//! Instructions a dot proposes for itself (conversational chat spec section 4).

mod common;

use common::{spec, temp_store};
use dots_core::events::RuntimeEvent;
use dots_core::instructions::{
    may_propose, InstructionProposal, InstructionStatus, Proposals, DECIDED_ALREADY,
    MAX_INSTRUCTIONS, MAX_REASON, NOT_HERE, SENT, UNCHANGED,
};
use dots_core::model::{NewRun, Run, RunStatus, TriggerKind};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::json;
use tokio::sync::broadcast;

async fn chat(store: &Store, dot_id: &str, typed: bool) -> Run {
    let mut new = NewRun::new(dot_id, TriggerKind::Chat);
    new.payload = Some(if typed {
        json!({"message": "from now on, buy the closest brand", "typed": true})
    } else {
        json!({"message": "The person added the app notes to the Library."})
    });
    store.create_run(&new).await.unwrap()
}

fn proposals(store: &Store) -> (Proposals, broadcast::Receiver<RuntimeEvent>) {
    let (bus, rx) = broadcast::channel(16);
    (Proposals::new(store.clone(), bus), rx)
}

fn changed(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<String> {
    let mut ids = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::InstructionProposalChanged { id, .. } = ev {
            ids.push(id);
        }
    }
    ids
}

#[tokio::test]
async fn a_person_s_chat_proposes_and_a_newer_proposal_replaces_the_waiting_one() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, mut rx) = proposals(&store);
    let next = "Do the thing.\nIf an item is out of stock, buy the closest brand.";
    assert_eq!(
        p.propose(&run.id, &format!("  {next}\n"), " Buy the closest brand ")
            .await
            .unwrap(),
        SENT
    );
    let listed = store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    let first = &listed[0];
    assert_eq!(first.base, "Do the thing.");
    assert_eq!(first.instructions, next);
    assert_eq!(first.reason, "Buy the closest brand");
    assert_eq!(first.status, InstructionStatus::Pending);
    assert_eq!(first.run_id, run.id);
    assert_eq!(first.root_run_id, run.id);
    assert_eq!(changed(&mut rx), vec![first.id.clone()]);
    // The person did not decide yet and the dot proposes again: the newer one waits instead.
    p.propose(&run.id, "Do the thing twice.", "Twice")
        .await
        .unwrap();
    let listed = store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].instructions, "Do the thing twice.");
    assert_eq!(listed[0].status, InstructionStatus::Pending);
    assert_eq!(listed[1].id, first.id);
    assert_eq!(listed[1].status, InstructionStatus::Replaced);
    assert!(listed[1].decided_at.is_some());
    assert_eq!(
        changed(&mut rx),
        vec![first.id.clone(), listed[0].id.clone()]
    );
    assert_eq!(
        store.list_instruction_proposals(None).await.unwrap().len(),
        2
    );
}

#[tokio::test]
async fn only_a_topic_the_person_started_may_propose() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let (p, _rx) = proposals(&store);
    let scheduled = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let made = chat(&store, &dot.id, false).await;
    for run in [&scheduled, &made] {
        assert!(!may_propose(&store, run).await.unwrap());
        assert_eq!(
            p.propose(&run.id, "Other.", "Why").await.unwrap_err(),
            NOT_HERE
        );
    }
    // A run that continues the person's topic may, a workflow step's run may not.
    let typed = chat(&store, &dot.id, true).await;
    let mut resume = NewRun::new(&dot.id, TriggerKind::Resume);
    resume.parent_run_id = Some(typed.id.clone());
    resume.root_run_id = Some(typed.id.clone());
    let resume = store.create_run(&resume).await.unwrap();
    assert!(may_propose(&store, &resume).await.unwrap());
    let mut step = typed.clone();
    step.workflow_step_run_id = Some("STEP".into());
    assert!(!may_propose(&store, &step).await.unwrap());
    assert!(store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_text_and_the_reason_are_checked() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, _rx) = proposals(&store);
    let long = "x".repeat(MAX_INSTRUCTIONS + 1);
    let reason = "r".repeat(MAX_REASON + 1);
    for (instructions, why, err) in [
        (
            "  ",
            "Why",
            "the instructions must be 1 to 20000 characters",
        ),
        (
            long.as_str(),
            "Why",
            "the instructions must be 1 to 20000 characters",
        ),
        ("Other.", " ", "the reason must be 1 to 500 characters"),
        (
            "Other.",
            reason.as_str(),
            "the reason must be 1 to 500 characters",
        ),
        (" Do the thing. ", "Why", UNCHANGED),
    ] {
        assert_eq!(
            p.propose(&run.id, instructions, why).await.unwrap_err(),
            err
        );
    }
    assert!(store
        .list_instruction_proposals(None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_proposal_is_decided_once_and_goes_with_its_dot() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let (p, _rx) = proposals(&store);
    p.propose(&run.id, "Other.", "Why").await.unwrap();
    let id = store.list_instruction_proposals(None).await.unwrap()[0]
        .id
        .clone();
    let next = NewRun::new(&dot.id, TriggerKind::Chat);
    let (decided, answer) = store
        .decide_instruction_proposal(&id, InstructionStatus::Approved, &next)
        .await
        .unwrap();
    assert_eq!(decided.status, InstructionStatus::Approved);
    assert_eq!(decided.answer_run_id.as_deref(), Some(answer.id.as_str()));
    assert!(decided.decided_at.is_some());
    let again = store
        .decide_instruction_proposal(&id, InstructionStatus::Rejected, &next)
        .await
        .unwrap_err();
    assert!(matches!(again, Error::Conflict(m) if m == DECIDED_ALREADY));
    assert!(matches!(
        store
            .decide_instruction_proposal("nope", InstructionStatus::Rejected, &next)
            .await
            .unwrap_err(),
        Error::NotFound(_)
    ));
    // The refused decisions stored no run.
    let runs = store.list_runs(Some(&dot.id), 10).await.unwrap();
    assert_eq!(runs.len(), 2);
    for r in &runs {
        store
            .finish_run(&r.id, RunStatus::Succeeded, None, None)
            .await
            .unwrap();
    }
    store.delete_dot_if_idle(&dot.id).await.unwrap();
    assert!(store
        .list_instruction_proposals(None)
        .await
        .unwrap()
        .is_empty());
}

/// A proposal of the run `run`, made at `created_at`, with the id `id`.
fn proposal(run: &Run, id: &str, created_at: &str) -> InstructionProposal {
    InstructionProposal {
        id: id.into(),
        dot_id: run.dot_id.clone(),
        run_id: run.id.clone(),
        root_run_id: run.root_run_id.clone(),
        base: "Do the thing.".into(),
        instructions: format!("Do the thing {id}."),
        reason: "Why".into(),
        status: InstructionStatus::Pending,
        answer_run_id: None,
        created_at: created_at.into(),
        decided_at: None,
    }
}

#[tokio::test]
async fn proposals_made_in_the_same_millisecond_list_the_newer_first() {
    let (_d, store) = temp_store().await;
    let dot = store.create_dot(&spec("shopper"), None).await.unwrap();
    let run = chat(&store, &dot.id, true).await;
    let at = "2026-10-28T09:00:00.000Z";
    // The newer one has the smaller id: the order cannot come from the ids.
    store
        .add_instruction_proposal(&proposal(&run, "b", at))
        .await
        .unwrap();
    store
        .add_instruction_proposal(&proposal(&run, "a", at))
        .await
        .unwrap();
    let ids: Vec<String> = store
        .list_instruction_proposals(Some(&dot.id))
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["a", "b"]);
}

#[tokio::test]
async fn only_the_waiting_proposals_are_pending() {
    let (_d, store) = temp_store().await;
    let one = store.create_dot(&spec("one"), None).await.unwrap();
    let two = store.create_dot(&spec("two"), None).await.unwrap();
    let three = store.create_dot(&spec("three"), None).await.unwrap();
    let (p, _rx) = proposals(&store);
    for dot in [&one, &two, &three] {
        let run = chat(&store, &dot.id, true).await;
        p.propose(&run.id, "Other.", "Why").await.unwrap();
    }
    // One is replaced by a newer one, one is decided.
    let run = chat(&store, &one.id, true).await;
    p.propose(&run.id, "Other again.", "Why").await.unwrap();
    let decided = store
        .list_instruction_proposals(Some(&two.id))
        .await
        .unwrap()[0]
        .id
        .clone();
    store
        .decide_instruction_proposal(
            &decided,
            InstructionStatus::Rejected,
            &NewRun::new(&two.id, TriggerKind::Chat),
        )
        .await
        .unwrap();
    let pending = store.pending_instruction_proposals().await.unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|p| (p.dot_id.as_str(), p.instructions.as_str()))
            .collect::<Vec<_>>(),
        [
            (one.id.as_str(), "Other again."),
            (three.id.as_str(), "Other.")
        ]
    );
    assert!(pending
        .iter()
        .all(|p| p.status == InstructionStatus::Pending));
}
