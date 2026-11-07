//! The audit queue and History in the store (env vault spec §10): queued records get the next
//! `seq` and chain on the line before, a full queue drops its oldest records with an `expired`
//! record naming them, a record of an account without `audit` is only listed, and History
//! keeps 30 days and at most its row limit.

use dots_core::store::{AuditLimits, HistoryRow, HistoryState, NewAudit, Store};
use dots_core::vault::audit::record::REASON_QUEUE_FULL;
use dots_core::vault::audit::{
    line_hash, verify_lines, AuditDot, AuditRecord, AuditUser, Dropped, Event, Finding,
};
use tempfile::TempDir;

async fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    (dir, store)
}

fn record(org_id: &str, at: &str, name: &str) -> AuditRecord {
    AuditRecord {
        v: 1,
        id: dots_core::util::new_id(),
        seq: 0,
        prev: None,
        at: at.into(),
        org_id: org_id.into(),
        install: "01J9INSTALL".into(),
        user: AuditUser::default(),
        dot: Some(AuditDot {
            id: "01J9DOT".into(),
            name: "Triage".into(),
        }),
        run_id: Some("01J9RUN".into()),
        event: Event::Resolved,
        name: Some(name.into()),
        consumer: Some("shell".into()),
        source: Some("openbao".into()),
        reason: None,
        dropped: None,
    }
}

const AT: &str = "2026-10-22T09:00:00Z";

async fn queue(store: &Store, limits: &AuditLimits, name: &str) -> Vec<AuditRecord> {
    let new = NewAudit {
        record: record("acme-eng", AT, name),
        queue: true,
        history: true,
    };
    store.append_audit(new, limits).await.unwrap()
}

fn lines(batch: &[(u64, String)]) -> String {
    batch.iter().map(|(_, l)| format!("{l}\n")).collect()
}

#[tokio::test]
async fn queued_records_count_up_and_chain_on_the_line_before() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    let first = queue(&store, &limits, "A").await;
    let second = queue(&store, &limits, "B").await;
    assert_eq!(first.len(), 1);
    assert_eq!((first[0].seq, first[0].prev.clone()), (1, None));
    assert_eq!(second[0].seq, 2);
    assert_eq!(second[0].prev, Some(line_hash(&first[0].line())));

    let batch = store.audit_batch("acme-eng", 500).await.unwrap();
    assert_eq!(batch, vec![(1, first[0].line()), (2, second[0].line())]);
    assert!(verify_lines(&lines(&batch)).findings.is_empty());
    assert_eq!(store.audit_queued("acme-eng").await.unwrap(), 2);
    assert_eq!(
        store.audit_waiting().await.unwrap(),
        vec![("acme-eng".to_string(), 2)]
    );
    // Each org has its own chain.
    let new = NewAudit {
        record: record("beta", AT, "A"),
        queue: true,
        history: true,
    };
    assert_eq!(store.append_audit(new, &limits).await.unwrap()[0].seq, 1);
}

#[tokio::test]
async fn a_record_that_is_not_queued_is_only_listed_and_spends_no_seq() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    let new = NewAudit {
        record: record("acme-eng", AT, "A"),
        queue: false,
        history: true,
    };
    let written = store.append_audit(new, &limits).await.unwrap();
    assert_eq!((written[0].seq, written[0].prev.clone()), (0, None));
    assert_eq!(store.audit_queued("acme-eng").await.unwrap(), 0);
    let history = store.audit_history("acme-eng", 100).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].state, HistoryState::Local);
    // Once the account sends records, the first is seq 1.
    assert_eq!(queue(&store, &limits, "B").await[0].seq, 1);
}

#[tokio::test]
async fn history_lists_newest_first_and_follows_what_was_sent() {
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    for name in ["A", "B", "C"] {
        queue(&store, &limits, name).await;
    }
    let fetched = NewAudit {
        record: AuditRecord {
            event: Event::Fetched,
            name: None,
            consumer: None,
            dot: None,
            run_id: None,
            ..record("acme-eng", AT, "")
        },
        queue: true,
        history: false,
    };
    store.append_audit(fetched, &limits).await.unwrap();
    store.audit_sent("acme-eng", 2).await.unwrap();

    let history = store.audit_history("acme-eng", 100).await.unwrap();
    let shown: Vec<(Option<&str>, HistoryState)> = history
        .iter()
        .map(|row: &HistoryRow| (row.name.as_deref(), row.state))
        .collect();
    assert_eq!(
        shown,
        vec![
            (Some("C"), HistoryState::Queued),
            (Some("B"), HistoryState::Sent),
            (Some("A"), HistoryState::Sent),
        ]
    );
    assert_eq!(history[0].dot_name.as_deref(), Some("Triage"));
    assert_eq!(history[0].consumer.as_deref(), Some("shell"));
    // The fetch is queued but not listed.
    let left: Vec<u64> = store
        .audit_batch("acme-eng", 500)
        .await
        .unwrap()
        .iter()
        .map(|(seq, _)| *seq)
        .collect();
    assert_eq!(left, vec![3, 4]);
}

#[tokio::test]
async fn a_full_queue_drops_its_oldest_records_and_says_which() {
    let (_dir, store) = store().await;
    let limits = AuditLimits {
        max_queued: 5,
        drop_chunk: 2,
        ..AuditLimits::default()
    };
    let mut received = String::new();
    let first = queue(&store, &limits, "A").await;
    received.push_str(&format!("{}\n", first[0].line()));
    store.audit_sent("acme-eng", 1).await.unwrap();
    for name in ["B", "C", "D", "E", "F"] {
        assert_eq!(queue(&store, &limits, name).await.len(), 1);
    }
    // Seq 2 to 6 are queued: the next record drops 2 and 3.
    let written = queue(&store, &limits, "G").await;
    assert_eq!(written.len(), 2);
    let expired = &written[0];
    assert_eq!(expired.seq, 7);
    assert_eq!(expired.event, Event::Expired);
    assert_eq!(expired.reason.as_deref(), Some(REASON_QUEUE_FULL));
    assert_eq!(expired.dropped, Some(Dropped { from: 2, to: 3 }));
    assert_eq!((expired.dot.clone(), expired.name.clone()), (None, None));
    assert_eq!(written[1].seq, 8);

    let batch = store.audit_batch("acme-eng", 500).await.unwrap();
    let seqs: Vec<u64> = batch.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs, vec![4, 5, 6, 7, 8]);
    received.push_str(&lines(&batch));
    let report = verify_lines(&received);
    assert!(report.ok());
    assert_eq!(
        report.findings,
        vec![Finding::Gap {
            org_id: "acme-eng".into(),
            install: "01J9INSTALL".into(),
            line: 2,
            from: 2,
            to: 3,
            dropped: true,
        }]
    );
    let states: Vec<(Option<String>, HistoryState)> = store
        .audit_history("acme-eng", 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.name, row.state))
        .collect();
    assert!(states.contains(&(Some("B".into()), HistoryState::Dropped)));
    assert!(states.contains(&(Some("C".into()), HistoryState::Dropped)));
    assert!(states.contains(&(None, HistoryState::Queued)));
}

#[tokio::test]
async fn two_drops_in_a_row_name_the_whole_gap_between_them() {
    let (_dir, store) = store().await;
    let limits = AuditLimits {
        max_queued: 5,
        drop_chunk: 2,
        ..AuditLimits::default()
    };
    let mut received = String::new();
    let first = queue(&store, &limits, "A").await;
    received.push_str(&format!("{}\n", first[0].line()));
    store.audit_sent("acme-eng", 1).await.unwrap();
    for name in ["B", "C", "D", "E", "F"] {
        queue(&store, &limits, name).await;
    }
    // G drops 2 and 3 (expired seq 7); H drops 4 and 5 (expired seq 9).
    let g = queue(&store, &limits, "G").await;
    assert_eq!(g[0].dropped, Some(Dropped { from: 2, to: 3 }));
    let h = queue(&store, &limits, "H").await;
    assert_eq!(h[0].dropped, Some(Dropped { from: 4, to: 5 }));

    let batch = store.audit_batch("acme-eng", 500).await.unwrap();
    let seqs: Vec<u64> = batch.iter().map(|(seq, _)| *seq).collect();
    assert_eq!(seqs, vec![6, 7, 8, 9, 10]);
    received.push_str(&lines(&batch));
    let report = verify_lines(&received);
    assert!(report.ok(), "{:?}", report.findings);
    assert_eq!(
        report.findings,
        vec![Finding::Gap {
            org_id: "acme-eng".into(),
            install: "01J9INSTALL".into(),
            line: 2,
            from: 2,
            to: 5,
            dropped: true,
        }]
    );
}

#[tokio::test]
async fn history_keeps_thirty_days_and_at_most_its_row_limit() {
    let (_dir, store) = store().await;
    let limits = AuditLimits {
        history_rows: 3,
        ..AuditLimits::default()
    };
    for (at, name) in [
        ("2026-08-31T09:00:00Z", "OLD"),
        ("2026-10-01T09:00:00Z", "A"),
        ("2026-10-01T09:00:01Z", "B"),
    ] {
        let new = NewAudit {
            record: record("acme-eng", at, name),
            queue: false,
            history: true,
        };
        store.append_audit(new, &limits).await.unwrap();
    }
    let names = |rows: Vec<HistoryRow>| -> Vec<String> {
        rows.into_iter().filter_map(|r| r.name).collect()
    };
    // 2026-08-31 is more than 30 days before 2026-10-01.
    assert_eq!(
        names(store.audit_history("acme-eng", 100).await.unwrap()),
        vec!["B", "A"]
    );
    for name in ["C", "D"] {
        let new = NewAudit {
            record: record("acme-eng", "2026-10-01T09:00:02Z", name),
            queue: false,
            history: true,
        };
        store.append_audit(new, &limits).await.unwrap();
    }
    assert_eq!(
        names(store.audit_history("acme-eng", 100).await.unwrap()),
        vec!["D", "C", "B"]
    );
}

#[tokio::test]
async fn history_read_after_time_passes_without_an_append_drops_the_old_rows() {
    use chrono::{TimeZone, Utc};
    let (_dir, store) = store().await;
    let limits = AuditLimits::default();
    for (at, name) in [
        ("2026-10-01T09:00:00Z", "OLD"),
        ("2026-10-20T09:00:00Z", "NEW"),
    ] {
        let new = NewAudit {
            record: record("acme-eng", at, name),
            queue: false,
            history: true,
        };
        store.append_audit(new, &limits).await.unwrap();
    }
    let names = |rows: Vec<HistoryRow>| -> Vec<String> {
        rows.into_iter().filter_map(|r| r.name).collect()
    };
    // 21 days after the first: both are within 30 days.
    let day21 = Utc.with_ymd_and_hms(2026, 10, 22, 9, 0, 0).unwrap();
    assert_eq!(
        names(
            store
                .audit_history_as_of("acme-eng", day21, &limits)
                .await
                .unwrap()
        ),
        vec!["NEW", "OLD"]
    );
    // 40 days after the first, with no append since: the old row is gone from the read.
    let day40 = Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap();
    assert_eq!(
        names(
            store
                .audit_history_as_of("acme-eng", day40, &limits)
                .await
                .unwrap()
        ),
        vec!["NEW"]
    );
}
