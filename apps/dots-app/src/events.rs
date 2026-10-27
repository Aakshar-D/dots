//! Forwards `RuntimeEvent`s to the webview, turns some into OS notifications, and keeps the
//! tray's Inbox count current.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use dots_core::events::RuntimeEvent;
use dots_core::limits::HoldChange;
use dots_core::model::{Message, MessageKind, PausedReason, Run, RunStatus, TriggerKind};
use dots_core::Runtime;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::broadcast::error::RecvError;

use crate::{notify_routes, tray};

/// Every `RuntimeEvent`, as the same JSON (`{"type": "run_updated", ...}`).
pub const RUNTIME_EVENT: &str = "runtime";
/// Events were dropped (the webview fell behind); the UI refetches everything.
pub const LAGGED_EVENT: &str = "runtime-lagged";
/// The tray asks the UI to show a route, e.g. `/inbox`.
pub const NAVIGATE_EVENT: &str = "navigate";
/// The status changed outside the UI (the tray paused schedules); payload is the `Status`.
pub const STATUS_EVENT: &str = "status";
/// The update strip changed (updates spec §6); payload is the `UpdateNote`.
pub const UPDATE_EVENT: &str = "update";

/// Something worth an OS notification.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    ApprovalNeeded {
        run_id: String,
        tool: String,
        /// The call's input, previewed in the body (`call_preview`).
        input: Value,
    },
    RunEnded {
        run: Box<Run>,
    },
    /// A schedule needs attention: its occurrence failed in the end (Scheduler v2 spec §7).
    ScheduleFailed {
        dot_id: String,
        error: Option<String>,
    },
    /// A dot posted a note or asked a question (check-ins spec section 10). A question always
    /// notifies; a note only while Settings say so (`message_notifies`).
    MessagePosted {
        message: Box<Message>,
    },
    /// Dots paused a watcher: too many runs in an hour, or an error it cannot get past
    /// (watchers spec section 7). The core publishes it once per pause.
    WatcherPaused {
        dot_id: String,
        name: String,
        reason: PausedReason,
        error: Option<String>,
    },
    /// Background runs started or stopped being held for Claude usage (usage and limits spec
    /// section 4). The core says so once per change.
    Hold {
        change: dots_core::limits::HoldChange,
    },
}

/// Decides which events notify: approval requests, a run moving from queued or running into
/// succeeded, failed or awaiting approval, and a schedule that needs attention. Chat runs
/// never notify (the user is watching them). A scheduled run that fails does not notify by
/// itself: it is retried, or its schedule needs attention, which notifies. A run first seen
/// in a final state (e.g. marked reviewed) and a parked run closed by a decision do not
/// notify either. A run that parks on an approval request this notifier has announced does
/// not notify a second time: one park, one notification.
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
                    input: approval.input.clone(),
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
                    RunStatus::Succeeded => true,
                    RunStatus::Failed => run.trigger != TriggerKind::Schedule,
                    RunStatus::AwaitingApproval => !told,
                    _ => false,
                };
                (was_working && ended && run.trigger != TriggerKind::Chat).then(|| {
                    Notice::RunEnded {
                        run: Box::new(run.clone()),
                    }
                })
            }
            // The core publishes this once, when the flag goes from clear to set.
            RuntimeEvent::ScheduleAttention { dot_id, error, .. } => Some(Notice::ScheduleFailed {
                dot_id: dot_id.clone(),
                error: error.clone(),
            }),
            RuntimeEvent::MessagePosted { message } => Some(Notice::MessagePosted {
                message: Box::new(message.clone()),
            }),
            RuntimeEvent::WatcherPaused {
                dot_id,
                name,
                reason,
                error,
                ..
            } => Some(Notice::WatcherPaused {
                dot_id: dot_id.clone(),
                name: name.clone(),
                reason: *reason,
                error: error.clone(),
            }),
            RuntimeEvent::LimitsUpdated {
                change: Some(change),
                ..
            } => Some(Notice::Hold {
                change: change.clone(),
            }),
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

/// A one-line preview of a call's input, as the UI's `toolPreview` shows it: its `command`,
/// `file_path`, `path`, `pattern` or `url` (the first line that is not blank), else the input
/// as compact JSON cut to 80 characters. `None` when there is nothing to show.
pub fn call_preview(input: &Value) -> Option<String> {
    if let Some(fields) = input.as_object() {
        for key in ["command", "file_path", "path", "pattern", "url"] {
            if let Some(Value::String(text)) = fields.get(key) {
                return text
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .map(str::to_string);
            }
        }
        if fields.is_empty() {
            return None;
        }
    }
    if input.is_null() {
        return None;
    }
    let text = input.to_string();
    Some(if text.chars().count() > 80 {
        text.chars().take(80).collect::<String>() + "…"
    } else {
        text
    })
}

/// (title, body) of the notification (needs-you spec section 4.2). None sends the person to
/// the Inbox: a click on the notification opens what it is about (`notice_route`).
pub fn notice_text(notice: &Notice, dot_name: &str) -> (String, String) {
    match notice {
        Notice::ApprovalNeeded { tool, input, .. } => (
            format!("{dot_name} needs your approval"),
            match call_preview(input) {
                Some(preview) => first_line(Some(format!("{tool}: {preview}").as_str()), tool),
                None => first_line(Some(tool.as_str()), "A call"),
            },
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
                format!("{dot_name} is waiting for your approval"),
                first_line(run.summary.as_deref(), "A call waits for your decision."),
            ),
        },
        Notice::ScheduleFailed { error, .. } => (
            format!("{dot_name}'s schedule needs attention"),
            first_line(error.as_deref(), "Its run failed."),
        ),
        Notice::MessagePosted { message } => match message.kind {
            MessageKind::Question => (
                format!("{dot_name} asks"),
                first_line(Some(message.text.as_str()), "A question."),
            ),
            MessageKind::Note => (
                format!("{dot_name} says"),
                first_line(Some(message.text.as_str()), "A note."),
            ),
        },
        Notice::WatcherPaused {
            name,
            reason,
            error,
            ..
        } => {
            let why = match reason {
                PausedReason::Flood => dots_core::watchers::FLOOD_TEXT.to_string(),
                PausedReason::Error => first_line(error.as_deref(), "an error"),
            };
            (
                format!("{dot_name}'s watcher paused"),
                format!("{name} paused: {why}"),
            )
        }
        Notice::Hold { change } => (
            "Claude usage".to_string(),
            match change {
                HoldChange::Started {
                    window,
                    utilization,
                } => dots_core::limits::hold_started(window, *utilization),
                HoldChange::Ended => dots_core::limits::HOLD_ENDED.to_string(),
            },
        ),
    }
}

/// Where a click on the notice goes: the conversation of the run (`/chat/<dot>/<run>`, which
/// the home opens on the run's topic) for a run or an approval, the dot's conversation
/// (`/home/<dot>`) for a question, a note or a schedule, and the dot's Watchers tab for a
/// watcher Dots paused.
pub fn notice_route(notice: &Notice, dot_id: &str) -> String {
    match notice {
        Notice::ApprovalNeeded { run_id, .. } => format!("/chat/{dot_id}/{run_id}"),
        Notice::RunEnded { run } => format!("/chat/{dot_id}/{}", run.id),
        Notice::ScheduleFailed { .. } | Notice::MessagePosted { .. } => format!("/home/{dot_id}"),
        Notice::WatcherPaused { .. } => format!("/dots/{dot_id}?tab=watchers"),
        Notice::Hold { .. } => "/usage".to_string(),
    }
}

/// Whether a notice about the dot `dot_id` stays quiet because the person is reading that
/// dot's conversation (`viewing`). A notice whose dot is unknown is never quiet.
pub fn quiet(dot_id: Option<&str>, viewing: Option<&str>) -> bool {
    dot_id.is_some() && dot_id == viewing
}

/// Whether a posted message shows a notification: a question always, a note while the setting
/// `notify_on_notes` is on.
pub fn message_notifies(message: &Message, notify_on_notes: bool) -> bool {
    message.kind == MessageKind::Question || notify_on_notes
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
    if let Notice::MessagePosted { message } = notice {
        if !message_notifies(message, rt.settings().await.notify_on_notes) {
            return;
        }
    }
    let dot_id = match notice {
        Notice::ApprovalNeeded { run_id, .. } => rt.get_run(run_id).await.map(|r| r.dot_id).ok(),
        Notice::RunEnded { run } => Some(run.dot_id.clone()),
        Notice::ScheduleFailed { dot_id, .. } => Some(dot_id.clone()),
        Notice::MessagePosted { message } => Some(message.dot_id.clone()),
        Notice::WatcherPaused { dot_id, .. } => Some(dot_id.clone()),
        Notice::Hold { .. } => None,
    };
    // Nothing for the dot whose conversation the person is reading (needs-you spec 4.2).
    if quiet(dot_id.as_deref(), rt.viewing().as_deref()) {
        return;
    }
    let name = match &dot_id {
        Some(id) => rt.get_dot(id).await.map(|d| d.spec.name).ok(),
        None => None,
    };
    let (title, body) = notice_text(notice, name.as_deref().unwrap_or("A dot"));
    // A click opens what the notice is about; without its dot, the home.
    let route = match (&dot_id, notice) {
        (_, Notice::Hold { .. }) => notice_route(notice, ""),
        (Some(id), _) => notice_route(notice, id),
        (None, _) => "/home".to_string(),
    };
    let id = notify_routes::remember(app, route);
    let shown = app
        .notification()
        .builder()
        .id(id)
        .title(title)
        .body(body)
        .show();
    if let Err(e) = shown {
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
            retry_of: None,
            cost_usd: None,
            hold_released: false,
            workflow_step_run_id: None,
            tokens_cache_read: 0,
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
                self_apply: false,
                apply_state: None,
                applied_at: None,
                mcp_fingerprint: None,
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
                tool: "Bash".into(),
                input: json!({}),
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
    fn a_schedule_update_does_not_notify_but_may_change_the_inbox() {
        let mut n = Notifier::default();
        let ev = RuntimeEvent::ScheduleUpdated { dot_id: "d".into() };
        assert_eq!(n.decide(&ev), None);
        // The row holds "Needs attention", which the Inbox lists.
        assert!(affects_inbox(&ev));
    }

    #[test]
    fn a_library_update_does_not_notify_but_may_change_the_inbox() {
        let mut n = Notifier::default();
        let ev = RuntimeEvent::LibraryUpdated {
            item_id: "i".into(),
        };
        assert_eq!(n.decide(&ev), None);
        // A server whose tool list changed waits in the Inbox, and the tray counts it.
        assert!(affects_inbox(&ev));
    }

    #[test]
    fn a_failed_scheduled_run_is_quiet_and_its_attention_notifies() {
        let mut n = Notifier::default();
        let s = TriggerKind::Schedule;
        // Retried, or it ends in "Needs attention", which notifies.
        assert_eq!(
            notified(&mut n, "f", s, &[Queued, Running, Failed]),
            [false, false, false]
        );
        assert_eq!(
            notified(&mut n, "ok", s, &[Queued, Running, Succeeded]),
            [false, false, true]
        );
        // Other runs of a scheduled dot notify their failure as before.
        assert_eq!(
            notified(&mut n, "m", TriggerKind::Manual, &[Running, Failed]),
            [false, true]
        );

        // The core publishes this once, when the flag goes from clear to set.
        let ev = RuntimeEvent::ScheduleAttention {
            dot_id: "d".into(),
            run_id: "f".into(),
            error: Some("model server down\nat 09:00".into()),
        };
        let notice = n.decide(&ev).expect("a notice");
        assert_eq!(
            notice,
            Notice::ScheduleFailed {
                dot_id: "d".into(),
                error: Some("model server down\nat 09:00".into()),
            }
        );
        assert_eq!(
            notice_text(&notice, "nightly"),
            (
                "nightly's schedule needs attention".into(),
                "model server down".into()
            )
        );
        let unknown = Notice::ScheduleFailed {
            dot_id: "d".into(),
            error: None,
        };
        assert_eq!(notice_text(&unknown, "nightly").1, "Its run failed.");
        assert!(affects_inbox(&ev));
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
            notice_text(&ended(AwaitingApproval), "nightly"),
            (
                "nightly is waiting for your approval".into(),
                "All green.".into()
            )
        );
        let ask = Notice::ApprovalNeeded {
            run_id: "r".into(),
            tool: "Bash".into(),
            input: json!({"command": "cargo test\n--all"}),
        };
        assert_eq!(
            notice_text(&ask, "nightly"),
            (
                "nightly needs your approval".into(),
                "Bash: cargo test".into()
            )
        );
        let long = "x".repeat(300);
        let body = first_line(Some(&long), "");
        assert_eq!(body.chars().count(), 200);
        assert!(body.ends_with('…'));
        assert_eq!(first_line(Some("  \n"), "fallback"), "fallback");
    }

    #[test]
    fn a_call_dots_performs_is_not_a_notification() {
        let mut n = Notifier::default();
        let RuntimeEvent::ApprovalRequested { approval } = asked("r") else {
            unreachable!()
        };
        let update = RuntimeEvent::ApprovalUpdated { approval };
        assert!(n.decide(&update).is_none());
        // It is not an announced request either: a park that follows still notifies.
        assert_eq!(
            notified(
                &mut n,
                "r",
                TriggerKind::Schedule,
                &[Queued, Running, AwaitingApproval]
            ),
            [false, false, true]
        );
    }

    fn posted(kind: MessageKind) -> Message {
        Message {
            id: "m1".into(),
            dot_id: "d".into(),
            dot_name: "nightly".into(),
            run_id: Some("r".into()),
            kind,
            text: "\n  Ship the release?\nmore".into(),
            choices: None,
            created_at: "t".into(),
            read_at: None,
            answered_at: None,
            answer_run_id: None,
            dismissed_at: None,
        }
    }

    #[test]
    fn a_question_always_notifies_and_a_note_only_when_switched_on() {
        let mut n = Notifier::default();
        let question = posted(MessageKind::Question);
        let event = RuntimeEvent::MessagePosted {
            message: question.clone(),
        };
        assert_eq!(
            n.decide(&event),
            Some(Notice::MessagePosted {
                message: Box::new(question.clone())
            })
        );
        assert!(message_notifies(&question, false));
        let note = posted(MessageKind::Note);
        assert!(message_notifies(&note, true));
        assert!(!message_notifies(&note, false));
        // An answer or a read changes the Inbox and notifies nothing.
        let updated = RuntimeEvent::MessageUpdated {
            message: note.clone(),
        };
        assert!(n.decide(&updated).is_none());
        assert!(affects_inbox(&updated));
        assert_eq!(
            notice_text(
                &Notice::MessagePosted {
                    message: Box::new(question)
                },
                "nightly"
            ),
            ("nightly asks".into(), "Ship the release?".into())
        );
        assert_eq!(
            notice_text(
                &Notice::MessagePosted {
                    message: Box::new(note)
                },
                "nightly"
            ),
            ("nightly says".into(), "Ship the release?".into())
        );
    }

    #[test]
    fn every_notice_says_what_it_is_about_and_none_sends_to_the_inbox() {
        let m = TriggerKind::Manual;
        let ended = |status| Notice::RunEnded {
            run: Box::new(run("r", status, m)),
        };
        let bare = |status| Notice::RunEnded {
            run: Box::new(Run {
                summary: None,
                error: None,
                ..run("r", status, m)
            }),
        };
        let approval = |input| Notice::ApprovalNeeded {
            run_id: "r".into(),
            tool: "Bash".into(),
            input,
        };
        let schedule = |error: Option<&str>| Notice::ScheduleFailed {
            dot_id: "d".into(),
            error: error.map(str::to_string),
        };
        let message = |kind| Notice::MessagePosted {
            message: Box::new(posted(kind)),
        };
        let cases = [
            (
                message(MessageKind::Question),
                "nightly asks",
                "Ship the release?",
            ),
            (
                approval(json!({"command": "cargo test\n--all"})),
                "nightly needs your approval",
                "Bash: cargo test",
            ),
            (approval(json!({})), "nightly needs your approval", "Bash"),
            (
                ended(AwaitingApproval),
                "nightly is waiting for your approval",
                "All green.",
            ),
            (
                bare(AwaitingApproval),
                "nightly is waiting for your approval",
                "A call waits for your decision.",
            ),
            (ended(Succeeded), "nightly finished", "All green."),
            (bare(Succeeded), "nightly finished", "The run succeeded."),
            (ended(Failed), "nightly failed", "exit 3"),
            (bare(Failed), "nightly failed", "The run failed."),
            (
                schedule(Some("model server down\nat 09:00")),
                "nightly's schedule needs attention",
                "model server down",
            ),
            (
                schedule(None),
                "nightly's schedule needs attention",
                "Its run failed.",
            ),
            (
                message(MessageKind::Note),
                "nightly says",
                "Ship the release?",
            ),
        ];
        for (notice, title, body) in cases {
            let (t, b) = notice_text(&notice, "nightly");
            assert_eq!((t.as_str(), b.as_str()), (title, body), "{notice:?}");
            assert!(!t.contains("Inbox") && !b.contains("Inbox"), "{t} / {b}");
        }
    }

    #[test]
    fn a_call_is_previewed_on_one_line() {
        let preview = |input: Value| call_preview(&input);
        assert_eq!(
            preview(json!({"command": "\n  ls -la\nmore"})).as_deref(),
            Some("ls -la")
        );
        assert_eq!(
            preview(json!({"file_path": "C:/src/main.rs", "content": "x"})).as_deref(),
            Some("C:/src/main.rs")
        );
        assert_eq!(
            preview(json!({"url": "https://example.com/a"})).as_deref(),
            Some("https://example.com/a")
        );
        // Without one of those keys, the input itself, cut to 80 characters.
        assert_eq!(
            preview(json!({"query": "rust"})).as_deref(),
            Some(r#"{"query":"rust"}"#)
        );
        let long = preview(json!({"text": "x".repeat(200)})).unwrap();
        assert_eq!(long.chars().count(), 81);
        assert!(long.ends_with('…'));
        // Nothing to show: the tool alone.
        assert_eq!(preview(json!({})), None);
        assert_eq!(preview(Value::Null), None);
        assert_eq!(preview(json!({"command": "  \n "})), None);
    }

    /// Usage and limits spec section 4: one notice when holding starts, one when it ends,
    /// and none for a reading that changes neither; each opens the Usage page.
    #[test]
    fn a_hold_start_and_end_notify_once_each() {
        use dots_core::limits::{HoldChange, HoldShares, LimitsState};
        let mut notifier = Notifier::default();
        let state = LimitsState {
            windows: vec![],
            five_hour: None,
            weekly: None,
            shares: HoldShares {
                enabled: true,
                five_hour: 60,
                weekly: 70,
            },
            holding: false,
            draining: false,
            rejected_until: None,
            held_run_ids: vec![],
            hold_until: None,
        };
        let update = |change| RuntimeEvent::LimitsUpdated {
            limits: state.clone(),
            change,
        };
        assert_eq!(notifier.decide(&update(None)), None);
        let started = notifier
            .decide(&update(Some(HoldChange::Started {
                window: "five_hour".into(),
                utilization: 0.64,
            })))
            .unwrap();
        assert_eq!(
            notice_text(&started, "A dot"),
            (
                "Claude usage".to_string(),
                "Dots is holding background runs: Claude 5-hour usage is at 64%".to_string()
            )
        );
        assert_eq!(notice_route(&started, ""), "/usage");
        let weekly = notifier
            .decide(&update(Some(HoldChange::Started {
                window: "seven_day".into(),
                utilization: 0.72,
            })))
            .unwrap();
        assert_eq!(
            notice_text(&weekly, "A dot").1,
            "Dots is holding background runs: Claude weekly usage is at 72%"
        );
        let ended = notifier.decide(&update(Some(HoldChange::Ended))).unwrap();
        assert_eq!(
            notice_text(&ended, "A dot"),
            (
                "Claude usage".to_string(),
                "Held runs are starting".to_string()
            )
        );
        assert_eq!(notice_route(&ended, ""), "/usage");
    }

    #[test]
    fn a_watcher_dots_paused_notifies_once_and_opens_the_dots_watchers() {
        use dots_core::model::PausedReason;
        let mut notifier = Notifier::default();
        let paused = |reason, error: Option<&str>| RuntimeEvent::WatcherPaused {
            dot_id: "d".into(),
            watcher_id: "w".into(),
            name: "Invoices folder".into(),
            reason,
            error: error.map(str::to_string),
        };
        let flood = notifier.decide(&paused(PausedReason::Flood, None)).unwrap();
        assert_eq!(
            notice_text(&flood, "filer"),
            (
                "filer's watcher paused".to_string(),
                "Invoices folder paused: 20 runs in an hour".to_string()
            )
        );
        assert_eq!(notice_route(&flood, "d"), "/dots/d?tab=watchers");
        let error = notifier
            .decide(&paused(
                PausedReason::Error,
                Some("The folder C:/in no longer exists.\nmore"),
            ))
            .unwrap();
        assert_eq!(
            notice_text(&error, "filer").1,
            "Invoices folder paused: The folder C:/in no longer exists."
        );
        let updated = RuntimeEvent::WatcherUpdated {
            dot_id: "d".into(),
            watcher_id: "w".into(),
        };
        assert_eq!(notifier.decide(&updated), None);
    }

    #[test]
    fn a_notice_opens_the_conversation_it_is_about() {
        let ended = Notice::RunEnded {
            run: Box::new(run("r1", Succeeded, TriggerKind::Schedule)),
        };
        assert_eq!(notice_route(&ended, "d"), "/chat/d/r1");
        let RuntimeEvent::ApprovalRequested { approval } = asked("r2") else {
            unreachable!()
        };
        let ask = Notice::ApprovalNeeded {
            run_id: approval.run_id,
            tool: approval.tool,
            input: approval.input,
        };
        assert_eq!(notice_route(&ask, "d"), "/chat/d/r2");
        let failed = Notice::ScheduleFailed {
            dot_id: "d".into(),
            error: None,
        };
        assert_eq!(notice_route(&failed, "d"), "/home/d");
        for kind in [MessageKind::Question, MessageKind::Note] {
            let message = Notice::MessagePosted {
                message: Box::new(posted(kind)),
            };
            assert_eq!(notice_route(&message, "d"), "/home/d");
        }
    }

    #[test]
    fn nothing_notifies_about_the_dot_being_viewed() {
        assert!(quiet(Some("d"), Some("d")));
        assert!(!quiet(Some("d"), Some("other")));
        assert!(!quiet(Some("d"), None));
        // A notice whose dot is unknown (its run is gone) is not about the viewed dot.
        assert!(!quiet(None, Some("d")));
        assert!(!quiet(None, None));
    }
}
