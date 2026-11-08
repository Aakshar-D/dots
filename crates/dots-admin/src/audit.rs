//! `dots-admin audit verify` (env vault spec §12): checks a file of audit records as a receiver
//! holds them, with `dots-core`'s own checker, so the admin and project 2 read the chain the way
//! Dots writes it (`docs/vault-audit-format.md`).

use dots_core::vault::audit::verify_lines;

use crate::{failed, read_file, say, usage, Io, Result};

/// `audit verify <records.ndjson>`.
pub(crate) fn audit_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    match args {
        [sub, file] if sub == "verify" => verify_command(file, io),
        [sub, ..] if sub == "verify" => Err(usage("audit verify takes one file of records")),
        [other, ..] => Err(usage(format!("unknown audit command '{other}'"))),
        [] => Err(usage("audit needs a command: verify")),
    }
}

/// Prints each chain (an org on one install) with its records and seq range, then every
/// finding. A gap the install's own `expired` record explains is printed but is no failure.
/// It fails when the file holds no record, or when any chain is not whole.
fn verify_command(file: &str, io: &mut Io<'_>) -> Result<()> {
    let text = read_file(file, "the records file")?;
    let report = verify_lines(&text);
    if report.chains.is_empty() && report.findings.is_empty() {
        return Err(failed(format!("{file} holds no records")));
    }
    for chain in &report.chains {
        say(
            io.out,
            &format!(
                "{} ({}): {} records, seq {} to {}",
                chain.org_id, chain.install, chain.records, chain.first_seq, chain.last_seq
            ),
        )?;
    }
    if report.repeated > 0 {
        say(
            io.out,
            &format!(
                "{} lines repeat a record already read (a batch sent twice); they are not \
                 counted",
                report.repeated
            ),
        )?;
    }
    for finding in &report.findings {
        say(io.out, &finding.text())?;
    }
    let problems = report.findings.iter().filter(|f| f.is_failure()).count();
    if problems > 0 {
        let s = if problems == 1 { "" } else { "s" };
        return Err(failed(format!(
            "the records are not whole: {problems} problem{s}"
        )));
    }
    say(io.out, "ok: every chain is whole")
}
