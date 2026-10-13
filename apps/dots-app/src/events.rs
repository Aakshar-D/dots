//! Forwards `RuntimeEvent`s to the webview, turns some into OS notifications, and keeps the
//! tray's Inbox count current.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dots_core::events::RuntimeEvent;
use dots_core::model::{Run, RunStatus, TriggerKind};
use dots_core::Runtime;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

use crate::tray;

/// Every `RuntimeEvent`, as the same JSON (`{"type": "run_updated", ...}`).
pub const RUNTIME_EVENT: &str = "runtime";
/// Events were dropped (the webview fell behind); the UI refetches everything.
pub const LAGGED_EVENT: &str = "runtime-lagged";
/// The tray asks the UI to show a route, e.g. `/inbox`.
pub const NAVIGATE_EVENT: &str = "navigate";
/// The status changed outside the UI (the tray paused schedules); payload is the `Status`.
pub const STATUS_EVENT: &str = "status";

/// Something worth an OS notification.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    ApprovalNeeded { run_id: String, tool: String },
    RunEnded { run: Box<Run> },
}

/// Decides which events notify: approval requests, and a run moving from queued or running
/// into succeeded, failed or awaiting approval. Chat runs never notify (the user is watching
/// them). A run first seen in a final state (e.g. marked reviewed) and a parked run closed by
/// a decision do not notify either. A run that parks on an approval request this notifier
/// has announced does not notify a second time: one park, one notification.
#[derive(Default)]
pub struct Notifier {
    active: HashMap<String, RunStatus>,
    /// Runs with an announced approval request that have neither parked nor ended since.
    asked: HashSet<String>,
}

impl Notifier {
    /// Events were missed (the stream lagged): what this notifier remembers of live runs may
    /// be out of date, so it starts again. A run then notifies only once it is seen queued or
    /// running again.
    pub fn forget(&mut self) {
        self.active.clear();
        self.asked.clear();
    }

    pub fn decide(&mut self, ev: &RuntimeEvent) -> Option<Notice> {
        match ev {
            RuntimeEvent::ApprovalRequested { approval } => {
                self.asked.insert(approval.run_id.clone());
                Some(Notice::ApprovalNeeded {
                    run_id: approval.run_id.clone(),
                    tool: approval.tool.clone(),
                })
            }
            RuntimeEvent::RunUpdated { run } => {
                let prev = if run.status.is_terminal() {
                    self.active.remove(&run.id)
                } else {
                    self.active.insert(run.id.clone(), run.status)
                };
                // A run parks on a request (`ApprovalHub::ask`), and the request was announced:
                // that was the notice for this park. Parking or ending closes the request.
                let told = !matches!(run.status, RunStatus::Queued | RunStatus::Running)
                    && self.asked.remove(&run.id);
                let was_working = matches!(prev, Some(RunStatus::Queued | RunStatus::Running));
                let ended = match run.status {
                    RunStatus::Succeeded | RunStatus::Failed => true,
                    RunStatus::AwaitingApproval => !told,
                    _ => false,
                };
                (was_working && ended && run.trigger != TriggerKind::Chat).then(|| {
                    Notice::RunEnded {
                        run: Box::new(run.clone()),
                    }
                })
            }
            _ => None,
        }
    }
}

/// First non-empty line of `text`, at most 200 characters.
fn first_line(text: Option<&str>, fallback: &str) -> String {
    let line = text
        .and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or(fallback);
    if line.chars().count() > 200 {
        line.chars().take(199).collect::<String>() + "…"
    } else {
        line.to_string()
    }
}

/// (title, body) of the notification.
pub fn notice_text(notice: &Notice, dot_name: &str) -> (String, String) {
    match notice {
        Notice::ApprovalNeeded { tool, .. } => (
            format!("{dot_name} needs approval"),
            format!("{tool} is waiting in the Inbox."),
        ),
        Notice::RunEnded { run } => match run.status {
            RunStatus::Succeeded => (
                format!("{dot_name} finished"),
                first_line(run.summary.as_deref(), "The run succeeded."),
            ),
            RunStatus::Failed => (
                format!("{dot_name} failed"),
                first_line(run.error.as_deref(), "The run failed."),
            ),
            _ => (
                format!("{dot_name} is waiting for approval"),
                "Open the Inbox to decide.".to_string(),
            ),
        },
    }
}

/// Whether the Inbox count may have changed.
pub fn affects_inbox(ev: &RuntimeEvent) -> bool {
    !matches!(ev, RuntimeEvent::RunEvent { .. })
}

pub fn spawn_forwarder(app: AppHandle, rt: Arc<Runtime>) {
    let mut rx = rt.subscribe();
    tauri::async_runtime::spawn(async move {
        let mut notifier = Notifier::default();
        tray::refresh_count(&app, &rt).await;
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Err(e) = app.emit(RUNTIME_EVENT, &ev) {
                        tracing::warn!("emitting a runtime event failed: {e}");
                    }
                    if let Some(notice) = notifier.decide(&ev) {
                        notify(&app, &rt, &notice).await;
                    }
                    if affects_inbox(&ev) {
                        tray::refresh_count(&app, &rt).await;
                    }
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::warn!("UI event forwarder skipped {n} events");
                    notifier.forget();
                    let _ = app.emit(LAGGED_EVENT, ());
                    tray::refresh_count(&app, &rt).await;
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

async fn notify(app: &AppHandle, rt: &Runtime, notice: &Notice) {
    let run_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => run_id,
        Notice::RunEnded { run } => &run.id,
    };
    let name = match rt.get_run(run_id).await {
        Ok(run) => rt.get_dot(&run.dot_id).await.map(|d| d.spec.name).ok(),
        Err(_) => None,
    };
    let (title, body) = notice_text(notice, name.as_deref().unwrap_or("A dot"));
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!("showing a notification failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dots_core::model::{Approval, ApprovalStatus, RunEventRecord};
    use serde_json::json;
    use RunStatus::*;

    fn run(id: &str, status: RunStatus, trigger: TriggerKind) -> Run {
        Run {
            id: id.into(),
            dot_id: "d".into(),
            root_run_id: id.into(),
            parent_run_id: None,
            trigger,
            payload: None,
            status,
            session_id: None,
            workspace_path: None,
            branch: None,
            base_commit: None,
            summary: Some("\n  All green.\nmore".into()),
            error: Some("exit 3".into()),
            tokens_in: 0,
            tokens_out: 0,
            queued_at: "t".into(),
            started_at: None,
            ended_at: None,
            reviewed_at: None,
        }
    }

    /// Feeds `seq` as RunUpdated events for one run; which of them notified.
    fn notified(n: &mut Notifier, id: &str, trigger: TriggerKind, seq: &[RunStatus]) -> Vec<bool> {
        seq.iter()
            .map(|s| {
                let ev = RuntimeEvent::RunUpdated {
                    run: run(id, *s, trigger),
                };
                n.decide(&ev).is_some()
            })
            .collect()
    }

    #[test]
    fn a_run_notifies_once_when_it_ends_or_parks() {
        let mut n = Notifier::default();
        let m = TriggerKind::Manual;
        assert_eq!(
            notified(&mut n, "a", m, &[Queued, Running, Succeeded, Succeeded]),
            [false, false, true, false]
        );
        assert_eq!(
            notified(&mut n, "b", m, &[Queued, Running, Failed]),
            [false, false, true]
        );
        // Parking notifies; closing the parked run after a decision does not.
        assert_eq!(
            notified(
                &mut n,
                "c",
                m,
                &[Queued, Running, AwaitingApproval, Succeeded]
            ),
            [false, false, true, false]
        );
        assert_eq!(
            notified(&mut n, "d", m, &[Queued, Cancelled]),
            [false, false]
        );
    }

    /// An approval request of run `run_id`.
    fn asked(run_id: &str) -> RuntimeEvent {
        RuntimeEvent::ApprovalRequested {
            approval: Approval {
                id: "a1".into(),
                run_id: run_id.into(),
                tool: "Bash".into(),
                input: json!({}),
                input_hash: "h".into(),
                status: ApprovalStatus::Pending,
                note: None,
                parked: false,
                resolved: false,
                created_at: "t".into(),
                decided_at: None,
            },
        }
    }

    #[test]
    fn one_park_is_one_notification() {
        let mut n = Notifier::default();
        let s = TriggerKind::Schedule;
        // The request notifies. When nobody answers and the run parks on it, the user has
        // been told already; closing the parked run after the decision is quiet as before.
        assert_eq!(notified(&mut n, "r", s, &[Queued, Running]), [false, false]);
        assert!(matches!(
            n.decide(&asked("r")),
            Some(Notice::ApprovalNeeded { .. })
        ));
        assert_eq!(
            notified(&mut n, "r", s, &[AwaitingApproval, Succeeded]),
            [false, false]
        );
        // Each request notifies; the park after the second one still does not.
        assert_eq!(
            notified(&mut n, "two", s, &[Queued, Running]),
            [false, false]
        );
        assert!(n.decide(&asked("two")).is_some());
        assert!(n.decide(&asked("two")).is_some());
        assert_eq!(notified(&mut n, "two", s, &[AwaitingApproval]), [false]);
        // A request answered while the run waited hides nothing: the run's end notifies.
        assert_eq!(
            notified(&mut n, "live", s, &[Queued, Running]),
            [false, false]
        );
        assert!(n.decide(&asked("live")).is_some());
        assert_eq!(notified(&mut n, "live", s, &[Succeeded]), [true]);
        // Only the parked run is still remembered, and no request.
        assert_eq!(n.active.keys().collect::<Vec<_>>(), ["two"]);
        assert!(n.asked.is_empty());
    }

    #[test]
    fn a_lag_clears_the_memory_of_live_runs() {
        let mut n = Notifier::default();
        let m = TriggerKind::Manual;
        assert_eq!(notified(&mut n, "a", m, &[Queued, Running]), [false, false]);
        assert_eq!(notified(&mut n, "b", m, &[Queued, Running]), [false, false]);
        assert!(n.decide(&asked("b")).is_some());
        n.forget();
        // "a" may have ended during the gap and be published again by "Mark reviewed".
        assert_eq!(notified(&mut n, "a", m, &[Succeeded]), [false]);
        // The request seen before the gap may not be the one "b" parks on.
        assert_eq!(
            notified(&mut n, "b", m, &[Running, AwaitingApproval]),
            [false, true]
        );
        // A run seen working after the gap is trusted again.
        assert_eq!(notified(&mut n, "c", m, &[Running, Failed]), [false, true]);
    }

    #[test]
    fn chat_runs_and_runs_first_seen_finished_stay_quiet() {
        let mut n = Notifier::default();
        assert_eq!(
            notified(
                &mut n,
                "c",
                TriggerKind::Chat,
                &[Queued, Running, Succeeded]
            ),
            [false, false, false]
        );
        // e.g. "Mark reviewed" publishes an old run again.
        assert_eq!(
            notified(&mut n, "old", TriggerKind::Schedule, &[Succeeded]),
            [false]
        );
    }

    #[test]
    fn approval_requests_notify_and_run_events_do_not() {
        let mut n = Notifier::default();
        assert_eq!(
            n.decide(&asked("r1")),
            Some(Notice::ApprovalNeeded {
                run_id: "r1".into(),
                tool: "Bash".into()
            })
        );
        let ev = RuntimeEvent::RunEvent {
            event: RunEventRecord {
                run_id: "r1".into(),
                seq: 1,
                ts: "t".into(),
                kind: "assistant_text".into(),
                data: json!({}),
            },
        };
        assert_eq!(n.decide(&ev), None);
        assert!(!affects_inbox(&ev));
    }

    #[test]
    fn notification_texts() {
        let ended = |s| Notice::RunEnded {
            run: Box::new(run("r", s, TriggerKind::Manual)),
        };
        assert_eq!(
            notice_text(&ended(Succeeded), "nightly"),
            ("nightly finished".into(), "All green.".into())
        );
        assert_eq!(
            notice_text(&ended(Failed), "nightly"),
            ("nightly failed".into(), "exit 3".into())
        );
        assert_eq!(
            notice_text(&ended(AwaitingApproval), "nightly").0,
            "nightly is waiting for approval"
        );
        let ask = Notice::ApprovalNeeded {
            run_id: "r".into(),
            tool: "Bash".into(),
        };
        assert_eq!(
            notice_text(&ask, "nightly"),
            (
                "nightly needs approval".into(),
                "Bash is waiting in the Inbox.".into()
            )
        );
        let long = "x".repeat(300);
        let body = first_line(Some(&long), "");
        assert_eq!(body.chars().count(), 200);
        assert!(body.ends_with('…'));
        assert_eq!(first_line(Some("  \n"), "fallback"), "fallback");
    }
}
