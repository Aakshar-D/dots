use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::{cap_tail, str_arg, ToolOutput, MAX_OUTPUT};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
const HELD_OPEN: &str = "[output stream still held open by a background process]";

/// Runs the user script, which arrives as base64 UTF-8 in place of `SCRIPT_BASE64`.
///
/// With redirected pipes, `powershell -EncodedCommand` writes errors, warnings and host output to
/// stderr as CLIXML, and an error message quotes the whole command text, wrapper included. So the
/// script runs as a ScriptBlock with every stream merged into the pipeline (`*>&1`), and each
/// record is printed as plain text; errors go to stderr through `ToString()`, which is just the
/// message. Parse errors are reported before anything runs, so the appended status line cannot
/// change how the script parses. That line records whether the last command succeeded:
/// PowerShell 5.1 marks a native command that wrote to the redirected stderr as failed, so its
/// zero exit code wins. The explicit `Out-Default` flushes formatted tables before `exit`.
#[cfg(windows)]
const BOOTSTRAP: &str = r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$ProgressPreference = 'SilentlyContinue'
$__dotsScript = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('SCRIPT_BASE64'))
$__dotsErrors = $null
[void][Management.Automation.Language.Parser]::ParseInput($__dotsScript, [ref]$null, [ref]$__dotsErrors)
if ($__dotsErrors) { $__dotsErrors | ForEach-Object { [Console]::Error.WriteLine("line $($_.Extent.StartLineNumber): $($_.Message)") }; exit 1 }
$__dotsOk = $true
try {
    . ([ScriptBlock]::Create($__dotsScript + "`n" + '$__dotsOk = $? -or ($LASTEXITCODE -eq 0 -and $Error[0].FullyQualifiedErrorId -eq ''NativeCommandError'')')) *>&1 | ForEach-Object {
        if ($_ -is [Management.Automation.ErrorRecord]) { [Console]::Error.WriteLine($_.ToString()) }
        elseif ($_ -is [Management.Automation.WarningRecord]) { "WARNING: $($_.Message)" }
        elseif ($_ -is [Management.Automation.VerboseRecord]) { "VERBOSE: $($_.Message)" }
        elseif ($_ -is [Management.Automation.DebugRecord]) { "DEBUG: $($_.Message)" }
        elseif ($_ -is [Management.Automation.InformationRecord]) { "$_" }
        else { $_ }
    } | Out-Default
} catch { [Console]::Error.WriteLine($_.ToString()); exit 1 }
if ($__dotsOk) { exit 0 } elseif ($LASTEXITCODE) { exit $LASTEXITCODE } else { exit 1 }"#;

/// The shell process for `script`. On Windows the bootstrap goes through `-EncodedCommand`,
/// which avoids every quoting problem of passing it as an argument, and prints UTF-8.
#[cfg(windows)]
fn command_for(script: &str) -> Command {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    let full = BOOTSTRAP.replace("SCRIPT_BASE64", &STANDARD.encode(script));
    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
    ])
    .arg(STANDARD.encode(utf16));
    cmd
}

/// The shell leads its own process group so that `stop` can kill everything it started.
#[cfg(not(windows))]
fn command_for(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script).process_group(0);
    cmd
}

/// Kills the shell and its descendants. Outside Windows `proc::kill_tree` would kill only `sh`,
/// so the shell's process group is killed instead.
async fn stop(pid: u32) {
    #[cfg(windows)]
    crate::proc::kill_tree(pid).await;
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .status()
            .await;
    }
}

type Tail = Arc<Mutex<Vec<u8>>>;

/// Appends what `r` yields to `tail`, keeping the last `2 * MAX_OUTPUT` bytes. The buffer is
/// shared so that what was read survives when the reader is abandoned before EOF.
async fn pump(mut r: impl AsyncRead + Unpin, tail: Tail) {
    let mut chunk = [0u8; 4096];
    while let Ok(n @ 1..) = r.read(&mut chunk).await {
        let mut buf = tail.lock().unwrap();
        buf.extend_from_slice(&chunk[..n]);
        let excess = buf.len().saturating_sub(2 * MAX_OUTPUT);
        buf.drain(..excess);
    }
}

fn take(tail: &Tail) -> String {
    String::from_utf8_lossy(&std::mem::take(&mut *tail.lock().unwrap())).into_owned()
}

/// Appends `more` to `text` on a new line, unless `more` is blank.
fn append_line(text: &mut String, more: &str) {
    if more.trim().is_empty() {
        return;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(more);
}

enum End {
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
}

pub(super) async fn shell(root: &Path, input: &Value, cancel: &CancellationToken) -> ToolOutput {
    let script = str_arg(input, "command");
    let secs = input
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let mut cmd = command_for(script);
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::proc::hide_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("failed to start the shell: {e}")),
    };
    let pid = child.id();
    let (out, err) = (Tail::default(), Tail::default());
    let mut readers = [
        tokio::spawn(pump(
            child.stdout.take().expect("stdout is piped"),
            out.clone(),
        )),
        tokio::spawn(pump(
            child.stderr.take().expect("stderr is piped"),
            err.clone(),
        )),
    ];

    let end = tokio::select! {
        s = child.wait() => End::Exited(s.ok().and_then(|s| s.code())),
        _ = tokio::time::sleep(Duration::from_secs(secs)) => End::TimedOut,
        _ = cancel.cancelled() => End::Cancelled,
    };
    if !matches!(end, End::Exited(_)) {
        if let Some(pid) = pid {
            stop(pid).await;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    // A background process the shell started may hold the pipes open; wait 5 s at most in all,
    // then keep what was read.
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        for r in &mut readers {
            let _ = r.await;
        }
    })
    .await;
    let held = readers.iter().any(|r| !r.is_finished());
    for r in &readers {
        r.abort();
    }
    let mut text = take(&out);
    append_line(&mut text, &take(&err));
    if held {
        append_line(&mut text, HELD_OPEN);
    }
    let text = cap_tail(text);
    match end {
        End::Exited(Some(0)) if text.trim().is_empty() => ToolOutput::ok("(no output)"),
        End::Exited(Some(0)) => ToolOutput::ok(text),
        End::Exited(code) => ToolOutput::err(format!(
            "exit code {}\n{text}",
            code.map_or_else(|| "unknown".to_string(), |c| c.to_string())
        )),
        End::TimedOut => {
            ToolOutput::err(format!("timed out after {secs}s and was stopped\n{text}"))
        }
        End::Cancelled => ToolOutput::err("cancelled"),
    }
}
