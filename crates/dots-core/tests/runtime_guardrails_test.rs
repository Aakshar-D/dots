//! Workspace guardrails at run time (guardrails spec §4): the account serves a signed member list
//! whose Payments workspace has the default guardrails, from the fake OpenBao. A Member dot's
//! `git push origin main` and its `Edit` of `migrations/x.sql` are refused; an Admin dot's ask
//! and can't take a rule. A Member's dot past the policy ceiling can't be saved or run, and one
//! made before the guardrails arrived is held until its policy comes down.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::clock::FakeClock;
use common::fake_openbao::FakeOpenBao;
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sign_roster, token, ACME, ACME_NAME, ISSUED,
    START, TOKEN,
};
use common::{spec, wait_status};
use dots_core::accounts::guardrails::GUARDRAIL_REFUSAL;
use dots_core::accounts::{identity, GuardRole, Guardrails};
use dots_core::model::{DotSpec, HoldReason, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::policy::{Policy, Preset};
use dots_core::vault::org::Orgs;
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Error, Runtime};
use minisign::KeyPair;
use serde_json::{json, Value};
use tempfile::TempDir;

const PUSH: &str = "This needs an Admin: pushing to main or master is protected in the Payments \
                    workspace.";
const MIGRATION: &str = "This needs an Admin: the file migrations/x.sql is protected in the \
                         Payments workspace.";
const CEILING: &str = "Acme Engineering: a Member's dot in the Payments workspace may use up to \
                       Sandboxed, and this policy is Trusted. Ask an Admin or Owner.";
const HELD: &str = "Acme Engineering: this dot's tool policy is past what a Member may use in the \
                    Payments workspace (this policy is Trusted). Change the policy, or ask an \
                    Admin.";

struct Rt {
    dir: TempDir,
    rt: Runtime,
    bao: FakeOpenBao,
    keys: KeyPair,
}

/// A run that asks for `git push origin main`, then an edit of `migrations/x.sql`.
fn script() -> Value {
    json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"approve": {"tool_name": "Bash", "input": {"command": "git push origin main"}}},
        {"approve": {"tool_name": "Edit", "input": {
            "file_path": "migrations/x.sql", "old_string": "a", "new_string": "b"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "ok",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ])
}

impl Rt {
    async fn start() -> Rt {
        let bao = FakeOpenBao::start().await;
        bao.allow_token(TOKEN, false);
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("script.json");
        std::fs::write(&script_path, script().to_string()).unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        cfg.clock = FakeClock::new(at(START));
        cfg.secrets = Some(Arc::new(MemorySecrets::new()));
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        cfg.claude_env.push((
            "FAKE_CLAUDE_SCRIPT".into(),
            script_path.to_string_lossy().to_string(),
        ));
        let rt = Runtime::start(cfg).await.unwrap();
        Rt {
            dir,
            rt,
            bao,
            keys: keys(),
        }
    }

    fn orgs(&self) -> &Orgs {
        self.rt.vault().orgs().expect("the runtime's orgs")
    }

    async fn me(&self) -> String {
        identity(self.rt.store()).await.unwrap().public_key
    }

    /// Serves a list at `seq` with Default and Payments (with `guardrails`), an owner who is
    /// not this PC, and this PC as `role`, in both workspaces.
    async fn serve(&self, seq: u64, role: &str, guardrails: Option<Guardrails>) {
        let ws_role = if role == "member" { "member" } else { "admin" };
        let mut payments = json!({"id": "payments", "name": "Payments"});
        if let Some(g) = guardrails {
            payments["guardrails"] = serde_json::to_value(g).unwrap();
        }
        let text = json!({
            "v": 1, "org_id": ACME, "issued_at": "2026-10-28T09:00:00Z", "seq": seq,
            "workspaces": [{"id": "default", "name": "Default"}, payments],
            "members": [
                member("01OWNER", "owner@acme.com", "owner", &public_key(&keys())),
                json!({
                    "id": "01ME", "name": "Me", "email": "me@acme.com", "role": role,
                    "keys": [self.me().await],
                    "workspaces": {"default": ws_role, "payments": ws_role},
                    "added_at": "2026-10-28T09:00:00Z"
                }),
            ]
        })
        .to_string();
        let extra = json!({
            "v": 2, "roster": "required", "admins": [], "plain": ["X"], "offline_hours": 0
        });
        self.bao.put_env("secret", "dots", json!({"X": "1"}));
        self.bao.put_policy(
            "secret",
            "dots",
            &sign(&self.keys, &policy(ACME, ISSUED, extra)),
        );
        self.bao
            .put_roster("secret", "dots", &sign_roster(&self.keys, &text));
    }

    async fn join(&self) {
        let code = code(
            ACME,
            ACME_NAME,
            openbao(&self.bao.address(), "dots"),
            &self.keys,
        );
        self.orgs().join(&code, Some(token())).await.unwrap();
    }

    /// Serves the list again at `seq` and fetches it.
    async fn become_(&self, seq: u64, role: &str, guardrails: Option<Guardrails>) {
        self.serve(seq, role, guardrails).await;
        self.rt.refresh_org(ACME).await.unwrap();
    }

    fn folder_spec(&self, name: &str, preset: Preset) -> DotSpec {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.approval_wait_secs = 0;
        s.policy = Policy::preset(preset);
        s
    }

    /// A dot of Acme's Payments workspace.
    async fn payments_dot(&self, preset: Preset) -> Result<String, Error> {
        let dot = self
            .rt
            .create_dot(
                self.folder_spec("deploy", preset),
                Some(ACME.into()),
                Some("payments".into()),
            )
            .await?;
        Ok(dot.id)
    }

    /// The text of every tool result of `run_id`.
    async fn tool_results(&self, run_id: &str) -> Vec<String> {
        self.rt
            .store()
            .list_events(run_id, 0)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "tool_result")
            .map(|e| e.data.to_string())
            .collect()
    }
}

fn member(id: &str, email: &str, role: &str, key: &str) -> Value {
    json!({
        "id": id, "name": email, "email": email, "role": role, "keys": [key],
        "workspaces": {"default": "admin"}, "added_at": "2026-10-28T09:00:00Z"
    })
}

fn refused<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::NotAllowed(text)) => text,
        other => panic!("expected NotAllowed, got {other:?}"),
    }
}

#[tokio::test]
async fn a_member_dot_is_refused_protected_calls() {
    let t = Rt::start().await;
    t.serve(1, "member", Some(Guardrails::defaults())).await;
    t.join().await;
    let dot = t.payments_dot(Preset::Sandboxed).await.unwrap();
    let ceiling = t.rt.dot_ceiling(&dot).await.unwrap();
    assert_eq!(ceiling.applies_as, Some(GuardRole::Member));
    assert_eq!(ceiling.max_preset, Some(Preset::Sandboxed));
    assert_eq!(ceiling.policy_refusal, None);

    let run = t.rt.run_now(&dot).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 20).await;
    let results = t.tool_results(&run.id).await;
    assert_eq!(results.len(), 2, "{results:?}");
    assert!(results[0].contains(PUSH), "{results:?}");
    assert!(results[1].contains(MIGRATION), "{results:?}");
    // Refused at once: nothing waits for anyone.
    assert!(t
        .rt
        .store()
        .list_pending_approvals()
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_admin_dot_asks_and_takes_no_rule() {
    let t = Rt::start().await;
    t.serve(1, "admin", Some(Guardrails::defaults())).await;
    t.join().await;
    // An Admin's dot has no ceiling: Trusted saves, and still asks before a protected call.
    let dot = t.payments_dot(Preset::Trusted).await.unwrap();
    assert_eq!(t.rt.dot_ceiling(&dot).await.unwrap().max_preset, None);
    let run = t.rt.run_now(&dot).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::AwaitingApproval, 20).await;
    let pending = t.rt.store().list_pending_approvals().await.unwrap();
    assert_eq!(pending.len(), 2);
    let push = pending
        .iter()
        .find(|a| a.tool == "Bash")
        .expect("the push asks");
    let card = t.rt.approval_guardrail(&push.id).await.unwrap().unwrap();
    assert!(!card.needs_admin);
    assert_eq!(card.what, "pushing to main or master");
    assert_eq!(
        card.text,
        "Pushing to main or master is protected in the Payments workspace, so it always asks."
    );
    assert!(t
        .rt
        .approval_rule_candidates(&push.id)
        .await
        .unwrap()
        .is_empty());
    match t
        .rt
        .decide_approval(
            &push.id,
            true,
            None,
            Some("Bash(git push origin main)".into()),
        )
        .await
    {
        Err(Error::Invalid(text)) => assert_eq!(text, GUARDRAIL_REFUSAL),
        other => panic!("expected Invalid, got {other:?}"),
    }
    // Approving without a rule goes.
    t.rt.decide_approval(&push.id, true, None, None)
        .await
        .unwrap();

    // The role drops to Member while the edit waits: it now needs an Admin.
    t.become_(2, "member", Some(Guardrails::defaults())).await;
    let edit = pending.iter().find(|a| a.tool == "Edit").unwrap();
    let card = t.rt.approval_guardrail(&edit.id).await.unwrap().unwrap();
    assert!(card.needs_admin);
    assert_eq!(card.text, MIGRATION);
    assert_eq!(
        refused(t.rt.decide_approval(&edit.id, true, None, None).await),
        MIGRATION
    );
    // Nor all at once.
    assert_eq!(
        refused(t.rt.decide_run_approvals(&run.id, None, true, None).await),
        MIGRATION
    );
    // Denying is still the Member's to do.
    t.rt.decide_approval(&edit.id, false, None, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn without_guardrails_nothing_changes() {
    let t = Rt::start().await;
    t.serve(1, "member", None).await;
    t.join().await;
    let dot = t.payments_dot(Preset::Trusted).await.unwrap();
    let ceiling = t.rt.dot_ceiling(&dot).await.unwrap();
    assert_eq!(ceiling.applies_as, None);
    assert!(ceiling.presets.iter().all(|p| p.refusal.is_none()));
    let run = t.rt.run_now(&dot).await.unwrap();
    // Trusted asks before a push (its own rule) and allows the edit.
    wait_status(t.rt.store(), &run.id, RunStatus::AwaitingApproval, 20).await;
    let pending = t.rt.store().list_pending_approvals().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert!(t
        .rt
        .approval_guardrail(&pending[0].id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_member_dot_past_the_ceiling_is_refused_then_held() {
    let t = Rt::start().await;
    t.serve(1, "member", Some(Guardrails::defaults())).await;
    t.join().await;
    assert_eq!(refused(t.payments_dot(Preset::Trusted).await), CEILING);
    let ceiling =
        t.rt.policy_ceiling(
            Some(ACME),
            "payments",
            Some(&Policy::preset(Preset::Trusted)),
        )
        .await
        .unwrap();
    assert_eq!(ceiling.policy_refusal.as_deref(), Some(CEILING));
    let trusted = ceiling
        .presets
        .iter()
        .find(|p| p.preset == Preset::Trusted)
        .unwrap();
    assert_eq!(trusted.refusal.as_deref(), Some(CEILING));
    assert!(ceiling
        .presets
        .iter()
        .filter(|p| p.preset != Preset::Trusted)
        .all(|p| p.refusal.is_none()));

    // A Trusted dot made before the guardrails: its runs wait.
    t.become_(2, "member", None).await;
    let dot = t.payments_dot(Preset::Trusted).await.unwrap();
    t.become_(3, "member", Some(Guardrails::defaults())).await;
    assert_eq!(refused(t.rt.run_now(&dot).await), CEILING);
    assert_eq!(
        t.rt.dot_ceiling(&dot).await.unwrap().held.as_deref(),
        Some(HELD)
    );
    let mut s = t.rt.store().get_dot(&dot).await.unwrap().spec;
    // Other fields can still change while the policy stays as it is; a new policy past the
    // ceiling can't be saved.
    s.instructions = "Deploy carefully.".into();
    s.name = "deploy-old".into();
    t.rt.update_dot(&dot, s.clone()).await.unwrap();
    let mut wider = s.clone();
    wider.policy.rules.push(dots_core::policy::Rule::new(
        "Bash",
        dots_core::policy::Action::Allow,
    ));
    assert!(refused(t.rt.update_dot(&dot, wider).await).contains("may use up to Sandboxed"));

    t.rt.hold_runs(true).await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Schedule))
            .await
            .unwrap();
    t.rt.hold_runs(false).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        t.rt.store().get_run(&run.id).await.unwrap().status,
        RunStatus::Queued
    );
    let inbox = t.rt.inbox().await.unwrap();
    assert_eq!(inbox.held.len(), 1);
    assert_eq!(
        inbox.held[0].reason,
        HoldReason::Role {
            account: ACME_NAME.into(),
            text: HELD.into(),
        }
    );
    assert_eq!(refused(t.rt.run_release_hold(&run.id).await), HELD);

    // Brought down to Sandboxed, it runs.
    s.policy = Policy::preset(Preset::Sandboxed);
    t.rt.update_dot(&dot, s).await.unwrap();
    wait_status(t.rt.store(), &run.id, RunStatus::Succeeded, 20).await;
}

#[tokio::test]
async fn the_guardrails_view_and_the_edit() {
    let t = Rt::start().await;
    t.serve(1, "member", Some(Guardrails::defaults())).await;
    t.join().await;
    let view = t.orgs().guardrails_view(ACME, "payments").await.unwrap();
    assert_eq!(view.workspace_name, "Payments");
    assert_eq!(view.seq, 1);
    assert_eq!(view.guardrails, Some(Guardrails::defaults()));
    assert_eq!(view.applies_as, Some(GuardRole::Member));
    assert_eq!(
        view.edit_refusal.as_deref(),
        Some("Acme Engineering: a Member can't change guardrails. Ask an Admin or Owner.")
    );
    assert_eq!(view.sign_blocker, view.edit_refusal);
    assert_eq!(view.git_actions.len(), 6);
    let default = t.orgs().guardrails_view(ACME, "default").await.unwrap();
    assert_eq!(default.guardrails, None);
    assert_eq!(default.applies_as, None);
    // A Member can't change them.
    let edit = dots_core::accounts::edit::RosterEdit::SetGuardrails {
        workspace_id: "default".into(),
        guardrails: Some(Guardrails::defaults()),
    };
    assert_eq!(
        refused(t.orgs().edit_members(ACME, 1, edit).await),
        "Acme Engineering: a Member can't change guardrails. Ask an Admin or Owner."
    );
    // An Admin who isn't a signer is told so.
    t.become_(2, "admin", None).await;
    let view = t.orgs().guardrails_view(ACME, "payments").await.unwrap();
    assert_eq!(view.edit_refusal, None);
    assert!(view
        .sign_blocker
        .as_deref()
        .is_some_and(|b| b.contains("signing admins")));
    assert!(matches!(
        t.orgs().guardrails_view(ACME, "nope").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn a_dot_whose_guardrails_cannot_be_read_does_not_run() {
    let t = Rt::start().await;
    // An Admin: its role reaches every workspace, so only the guardrails stop the dot.
    t.serve(1, "admin", Some(Guardrails::defaults())).await;
    t.join().await;
    let dot = t.payments_dot(Preset::Sandboxed).await.unwrap();
    // Its workspace is not on the member list.
    t.rt.store().set_dot_workspace(&dot, "ghost").await.unwrap();
    let text = refused(t.rt.run_now(&dot).await);
    assert!(text.contains("can't read the guardrails"), "{text}");
    t.rt.hold_runs(true).await;
    let run =
        t.rt.store()
            .create_run(&NewRun::new(&dot, TriggerKind::Schedule))
            .await
            .unwrap();
    t.rt.hold_runs(false).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        t.rt.store().get_run(&run.id).await.unwrap().status,
        RunStatus::Queued
    );
    let inbox = t.rt.inbox().await.unwrap();
    assert!(matches!(&inbox.held[0].reason, HoldReason::Role { text: held, .. } if held == &text));
}
