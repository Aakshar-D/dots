//! Checking records as a receiver holds them (env vault spec §12, `dots-admin audit verify`):
//! for each org and install, `seq` counts up by one and each `prev` is the hash of the line
//! before. A file may start part way through a chain, and may hold a batch sent twice (a 2xx
//! that never reached Dots): a line seen before is counted, not reported.

use std::collections::{BTreeMap, HashMap};

use super::record::{line_hash, AuditRecord, Event, REASON_QUEUE_FULL};

/// One org's records from one install, as the file holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSummary {
    pub org_id: String,
    pub install: String,
    /// Records read, not counting lines sent again.
    pub records: usize,
    pub first_seq: u64,
    pub last_seq: u64,
}

/// What is wrong with a file of records. Lines count from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// The line is not a record.
    Unreadable { line: usize, why: String },
    /// Records `from` to `to` of the chain are missing before `line`. `dropped`: the chain's
    /// `expired` records, alone or together, say its queue was full and dropped them unsent.
    Gap {
        org_id: String,
        install: String,
        line: usize,
        from: u64,
        to: u64,
        dropped: bool,
    },
    /// The line has a `seq` seen before, with another record.
    Conflict {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
    },
    /// The line's `seq` is lower than the one before it.
    OutOfOrder {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
        after: u64,
    },
    /// The line's `prev` is not the hash of the record before it (or seq 1 has one, or a later
    /// record has none).
    BrokenLink {
        org_id: String,
        install: String,
        line: usize,
        seq: u64,
    },
}

impl Finding {
    /// Whether the finding means the records are not what the PC sent: anything but a gap
    /// that the PC's own `expired` record explains.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Finding::Gap { dropped: true, .. })
    }

    /// The finding in plain words.
    pub fn text(&self) -> String {
        match self {
            Finding::Unreadable { line, why } => format!("line {line}: {why}"),
            Finding::Gap {
                org_id,
                install,
                line,
                from,
                to,
                dropped,
            } => {
                let what = if from == to {
                    format!("record {from} is missing")
                } else {
                    format!("records {from} to {to} are missing")
                };
                let why = if *dropped {
                    "; the PC's queue was full and dropped them, as its expired record says"
                } else {
                    ""
                };
                format!("{org_id} ({install}): {what} before line {line}{why}")
            }
            Finding::Conflict {
                org_id,
                install,
                line,
                seq,
            } => format!(
                "{org_id} ({install}): line {line} has seq {seq} again, with another record"
            ),
            Finding::OutOfOrder {
                org_id,
                install,
                line,
                seq,
                after,
            } => format!("{org_id} ({install}): line {line} has seq {seq}, after seq {after}"),
            Finding::BrokenLink {
                org_id,
                install,
                line,
                seq,
            } => format!(
                "{org_id} ({install}): line {line} (seq {seq}): its prev does not match the record \
                 before it"
            ),
        }
    }
}

/// What `verify_lines` found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainReport {
    /// Each chain, in the order its first record appears.
    pub chains: Vec<ChainSummary>,
    pub findings: Vec<Finding>,
    /// Lines that repeat a record already read (a batch sent again).
    pub repeated: usize,
}

impl ChainReport {
    /// No finding is a failure: every chain is whole, or its gaps are the PC's own drops.
    pub fn ok(&self) -> bool {
        self.findings.iter().all(|f| !f.is_failure())
    }
}

/// Where a chain is: its last record, and every seq read with its line's hash.
struct Chain {
    summary: usize,
    last: Option<(u64, String)>,
    seen: HashMap<u64, String>,
}

/// Checks a file of records, one per line (`\n` or `\r\n`; blank lines are skipped).
pub fn verify_lines(text: &str) -> ChainReport {
    let mut report = ChainReport::default();
    let mut read = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        match AuditRecord::parse(line) {
            Ok(record) => read.push((i + 1, line, record)),
            Err(why) => report
                .findings
                .push(Finding::Unreadable { line: i + 1, why }),
        }
    }
    // What each chain's `expired` records say its queue dropped.
    let mut dropped: BTreeMap<(String, String), Vec<(u64, u64)>> = BTreeMap::new();
    for (_, _, record) in &read {
        if let (Event::Expired, Some(reason), Some(range)) =
            (record.event, record.reason.as_deref(), record.dropped)
        {
            if reason == REASON_QUEUE_FULL {
                dropped
                    .entry((record.org_id.clone(), record.install.clone()))
                    .or_default()
                    .push((range.from, range.to));
            }
        }
    }
    // Drops in a row take the oldest end of the queue each time, so their ranges meet end to
    // end (1-1000, 1001-2000, ...). Join ranges that overlap or meet, so one gap can be
    // explained by several records together.
    for ranges in dropped.values_mut() {
        *ranges = merge_ranges(std::mem::take(ranges));
    }
    let mut chains: HashMap<(String, String), Chain> = HashMap::new();
    for (line, text, record) in read {
        let key = (record.org_id.clone(), record.install.clone());
        let (org_id, install) = key.clone();
        let chain = chains.entry(key.clone()).or_insert_with(|| {
            report.chains.push(ChainSummary {
                org_id: org_id.clone(),
                install: install.clone(),
                records: 0,
                first_seq: record.seq,
                last_seq: record.seq,
            });
            Chain {
                summary: report.chains.len() - 1,
                last: None,
                seen: HashMap::new(),
            }
        });
        let hash = line_hash(text);
        let seq = record.seq;
        if let Some(seen) = chain.seen.get(&seq) {
            if *seen == hash {
                report.repeated += 1;
            } else {
                report.findings.push(Finding::Conflict {
                    org_id,
                    install,
                    line,
                    seq,
                });
            }
            continue;
        }
        chain.seen.insert(seq, hash.clone());
        let summary = &mut report.chains[chain.summary];
        summary.records += 1;
        summary.first_seq = summary.first_seq.min(seq);
        summary.last_seq = summary.last_seq.max(seq);
        let broken = Finding::BrokenLink {
            org_id: org_id.clone(),
            install: install.clone(),
            line,
            seq,
        };
        match &chain.last {
            Some((last, _)) if seq < *last => {
                report.findings.push(Finding::OutOfOrder {
                    org_id,
                    install,
                    line,
                    seq,
                    after: *last,
                });
                continue;
            }
            Some((last, last_hash)) if seq == last + 1 => {
                if record.prev.as_deref() != Some(last_hash.as_str()) {
                    report.findings.push(broken);
                }
            }
            Some((last, _)) => {
                let (from, to) = (last + 1, seq - 1);
                let covered = dropped
                    .get(&key)
                    .is_some_and(|ranges| ranges.iter().any(|(a, b)| *a <= from && to <= *b));
                report.findings.push(Finding::Gap {
                    org_id,
                    install,
                    line,
                    from,
                    to,
                    dropped: covered,
                });
            }
            // The file's first record of the chain: only seq 1 says what its prev must be.
            None => {
                if (seq == 1) != record.prev.is_none() {
                    report.findings.push(broken);
                }
            }
        }
        chain.last = Some((seq, hash));
    }
    report
}

/// `ranges` sorted, with those that overlap or meet end to end joined into one.
fn merge_ranges(mut ranges: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (from, to) in ranges {
        match merged.last_mut() {
            Some((_, end)) if from <= end.saturating_add(1) => *end = (*end).max(to),
            _ => merged.push((from, to)),
        }
    }
    merged
}
