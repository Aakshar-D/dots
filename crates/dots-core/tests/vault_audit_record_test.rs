//! Audit records (env vault spec §10, §13): the line is canonical JSON with every key, `prev`
//! chains the lines, and `verify_lines` finds what a receiver must see: gaps, changed
//! records, a seq out of order, and the gaps a full queue explains itself.

use dots_core::vault::audit::record::REASON_QUEUE_FULL;
use dots_core::vault::audit::{
    canonical, line_hash, verify_lines, AuditDot, AuditRecord, AuditUser, ChainSummary, Dropped,
    Event, Finding,
};
use serde_json::json;

const INSTALL: &str = "01J9ZQ3V5X8K2M4N6P8R0T2V4W";

fn record(org_id: &str, seq: u64, prev: Option<String>) -> AuditRecord {
    AuditRecord {
        v: 1,
        id: format!("01J9ZQ3V5X8K2M4N6P8R0T{seq:04}"),
        seq,
        prev,
        at: "2026-10-21T09:12:03Z".into(),
        org_id: org_id.into(),
        install: INSTALL.into(),
        user: AuditUser {
            windows: Some("ACME\\jdoe".into()),
            machine: Some("LT-0423".into()),
            source_identity: Some("jdoe".into()),
            dots_id: Some("B255 6546 0D3C 9A11 7F20 44E1 C0DE 9B7A".into()),
        },
        dot: Some(AuditDot {
            id: "01J9DOT".into(),
            name: "Triage".into(),
        }),
        run_id: Some("01J9RUN".into()),
        event: Event::Resolved,
        name: Some("GITHUB_TOKEN".into()),
        consumer: Some("mcp:github".into()),
        source: Some("openbao".into()),
        reason: None,
        dropped: None,
    }
}

/// `n` chained records of `org_id`, seq 1 to `n`, as lines.
fn chain(org_id: &str, n: u64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for seq in 1..=n {
        let prev = lines.last().map(|line| line_hash(line));
        lines.push(record(org_id, seq, prev).line());
    }
    lines
}

fn file(lines: &[String]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn a_line_is_canonical_json_with_every_key_and_reads_back() {
    let mut first = record("acme-eng", 1, None);
    first.dot = None;
    first.run_id = None;
    assert_eq!(
        first.line(),
        concat!(
            r#"{"at":"2026-10-21T09:12:03Z","consumer":"mcp:github","dot":null,"dropped":null,"#,
            r#""event":"resolved","id":"01J9ZQ3V5X8K2M4N6P8R0T0001","#,
            r#""install":"01J9ZQ3V5X8K2M4N6P8R0T2V4W","#,
            r#""name":"GITHUB_TOKEN","org_id":"acme-eng","prev":null,"reason":null,"run_id":null,"#,
            r#""seq":1,"source":"openbao","#,
            r#""user":{"dots_id":"B255 6546 0D3C 9A11 7F20 44E1 C0DE 9B7A","#,
            r#""machine":"LT-0423","source_identity":"jdoe","windows":"ACME\\jdoe"},"v":1}"#
        )
    );
    assert_eq!(AuditRecord::parse(&first.line()).unwrap(), first);
}

#[test]
fn canonical_json_sorts_the_keys_of_every_object() {
    let value = json!({"b": 1, "a": {"d": [2, {"z": 1, "y": "é\"\n"}], "c": null}});
    assert_eq!(
        canonical(&value),
        r#"{"a":{"c":null,"d":[2,{"y":"é\"\n","z":1}]},"b":1}"#
    );
}

#[test]
fn a_lines_hash_is_the_sha256_of_its_bytes() {
    assert_eq!(
        line_hash("abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn a_record_with_an_unknown_field_or_another_version_is_refused() {
    let line = record("acme-eng", 1, None).line();
    let extra = line.replacen("{", r#"{"value":"x","#, 1);
    assert!(AuditRecord::parse(&extra)
        .unwrap_err()
        .contains("unknown field `value`"));
    let v2 = line.replace(r#""v":1"#, r#""v":2"#);
    assert_eq!(
        AuditRecord::parse(&v2).unwrap_err(),
        "record version 2 is not supported"
    );
}

#[test]
fn a_whole_chain_has_no_finding() {
    let report = verify_lines(&file(&chain("acme-eng", 5)));
    assert!(report.ok());
    assert_eq!(report.findings, vec![]);
    assert_eq!(
        report.chains,
        vec![ChainSummary {
            org_id: "acme-eng".into(),
            install: INSTALL.into(),
            records: 5,
            first_seq: 1,
            last_seq: 5,
        }]
    );
}

#[test]
fn a_file_may_start_part_way_and_use_crlf_line_ends() {
    let lines = chain("acme-eng", 6);
    let text = lines[3..].join("\r\n") + "\r\n\r\n";
    let report = verify_lines(&text);
    assert_eq!(report.findings, vec![]);
    assert_eq!(report.chains[0].first_seq, 4);
    assert_eq!(report.chains[0].records, 3);
}

#[test]
fn missing_records_are_a_gap() {
    let mut lines = chain("acme-eng", 6);
    lines.drain(1..4);
    let report = verify_lines(&file(&lines));
    assert!(!report.ok());
    let gap = Finding::Gap {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 2,
        from: 2,
        to: 4,
        dropped: false,
    };
    assert_eq!(report.findings, vec![gap.clone()]);
    assert_eq!(
        gap.text(),
        format!("acme-eng ({INSTALL}): records 2 to 4 are missing before line 2")
    );
}

#[test]
fn a_gap_the_queue_dropped_is_named_but_not_a_failure() {
    let mut lines = chain("acme-eng", 5);
    let mut expired = record("acme-eng", 6, Some(line_hash(&lines[4])));
    expired.event = Event::Expired;
    expired.reason = Some(REASON_QUEUE_FULL.into());
    expired.dropped = Some(Dropped { from: 2, to: 3 });
    expired.name = None;
    expired.consumer = None;
    lines.push(expired.line());
    lines.drain(1..3);
    let report = verify_lines(&file(&lines));
    assert!(report.ok());
    let gap = Finding::Gap {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 2,
        from: 2,
        to: 3,
        dropped: true,
    };
    assert_eq!(report.findings, vec![gap.clone()]);
    assert!(gap.text().ends_with(
        "records 2 to 3 are missing before line 2; the PC's queue was full and dropped them, \
         as its expired record says"
    ));
}

#[test]
fn drops_that_meet_end_to_end_explain_one_gap_together() {
    // Three full-queue drops in a row, 1,000 records each: 2 to 1001, 1002 to 2001 and 2002
    // to 3001. Each wrote its own expired record (seq 3002 to 3004). No single range holds
    // the receiver's gap, 2 to 3001, but together they do.
    let drops = [(2, 1001), (1002, 2001), (2002, 3001)];
    let mut lines: Vec<String> = Vec::new();
    for seq in 1..=3004u64 {
        let prev = lines.last().map(|line| line_hash(line));
        let mut next = record("acme-eng", seq, prev);
        if seq > 3001 {
            let (from, to) = drops[(seq - 3002) as usize];
            next.event = Event::Expired;
            next.reason = Some(REASON_QUEUE_FULL.into());
            next.dropped = Some(Dropped { from, to });
            next.name = None;
            next.consumer = None;
        }
        lines.push(next.line());
    }
    lines.drain(1..3001);
    let report = verify_lines(&file(&lines));
    assert!(report.ok(), "{:?}", report.findings);
    assert_eq!(
        report.findings,
        vec![Finding::Gap {
            org_id: "acme-eng".into(),
            install: INSTALL.into(),
            line: 2,
            from: 2,
            to: 3001,
            dropped: true,
        }]
    );

    // A range with a hole in it does not: 1002 to 1003 are named by no record.
    let mut holed: Vec<String> = Vec::new();
    for seq in 1..=1006u64 {
        let prev = holed.last().map(|line| line_hash(line));
        let mut next = record("acme-eng", seq, prev);
        if seq > 1004 {
            let (from, to) = [(2, 1001), (1004, 1004)][(seq - 1005) as usize];
            next.event = Event::Expired;
            next.reason = Some(REASON_QUEUE_FULL.into());
            next.dropped = Some(Dropped { from, to });
            next.name = None;
            next.consumer = None;
        }
        holed.push(next.line());
    }
    holed.drain(1..1004);
    let report = verify_lines(&file(&holed));
    assert!(!report.ok());
    assert_eq!(
        report.findings,
        vec![Finding::Gap {
            org_id: "acme-eng".into(),
            install: INSTALL.into(),
            line: 2,
            from: 2,
            to: 1004,
            dropped: false,
        }]
    );
}

#[test]
fn a_changed_record_breaks_the_link_after_it() {
    let mut lines = chain("acme-eng", 3);
    lines[1] = lines[1].replace("GITHUB_TOKEN", "OTHER_TOKEN");
    let report = verify_lines(&file(&lines));
    let broken = Finding::BrokenLink {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 3,
        seq: 3,
    };
    assert_eq!(report.findings, vec![broken.clone()]);
    assert_eq!(
        broken.text(),
        format!(
            "acme-eng ({INSTALL}): line 3 (seq 3): its prev does not match the record before it"
        )
    );
}

#[test]
fn seq_one_has_no_prev_and_a_later_first_record_has_one() {
    let with_prev = record("acme-eng", 1, Some(line_hash("x"))).line();
    let without = record("acme-eng", 7, None).line();
    let report = verify_lines(&file(&[with_prev]));
    assert!(matches!(
        report.findings[..],
        [Finding::BrokenLink { seq: 1, .. }]
    ));
    let report = verify_lines(&file(&[without]));
    assert!(matches!(
        report.findings[..],
        [Finding::BrokenLink { seq: 7, .. }]
    ));
}

#[test]
fn a_batch_sent_twice_is_counted_and_another_record_with_a_seen_seq_is_a_conflict() {
    let lines = chain("acme-eng", 4);
    let mut twice = lines.clone();
    twice.extend(lines[2..].iter().cloned());
    let report = verify_lines(&file(&twice));
    assert_eq!(report.findings, vec![]);
    assert_eq!(report.repeated, 2);
    assert_eq!(report.chains[0].records, 4);

    let mut other = lines.clone();
    other.push(lines[1].replace("Triage", "Other"));
    let report = verify_lines(&file(&other));
    let conflict = Finding::Conflict {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 5,
        seq: 2,
    };
    assert_eq!(report.findings, vec![conflict.clone()]);
    assert_eq!(
        conflict.text(),
        format!("acme-eng ({INSTALL}): line 5 has seq 2 again, with another record")
    );
}

#[test]
fn a_seq_lower_than_the_one_before_is_out_of_order() {
    let lines = chain("acme-eng", 4);
    let swapped = vec![
        lines[0].clone(),
        lines[2].clone(),
        lines[1].clone(),
        lines[3].clone(),
    ];
    let report = verify_lines(&file(&swapped));
    assert!(report.findings.contains(&Finding::OutOfOrder {
        org_id: "acme-eng".into(),
        install: INSTALL.into(),
        line: 3,
        seq: 2,
        after: 3,
    }));
    assert!(!report.ok());
}

#[test]
fn each_org_and_install_is_its_own_chain_and_a_bad_line_is_named() {
    let acme = chain("acme-eng", 2);
    let beta = chain("beta", 2);
    let mut other_install = record("acme-eng", 1, None);
    other_install.install = "01J9OTHERINSTALL".into();
    let lines = vec![
        acme[0].clone(),
        beta[0].clone(),
        "not json".to_string(),
        acme[1].clone(),
        other_install.line(),
        beta[1].clone(),
    ];
    let report = verify_lines(&file(&lines));
    let keys: Vec<(&str, &str, usize)> = report
        .chains
        .iter()
        .map(|c| (c.org_id.as_str(), c.install.as_str(), c.records))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("acme-eng", INSTALL, 2),
            ("beta", INSTALL, 2),
            ("acme-eng", "01J9OTHERINSTALL", 1)
        ]
    );
    assert!(matches!(
        &report.findings[..],
        [Finding::Unreadable { line: 3, why }] if why.starts_with("not a record: ")
    ));
}
