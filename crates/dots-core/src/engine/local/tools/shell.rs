use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::{cap_tail, str_arg, ToolOutput, MAX_OUTPUT};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;

/// The shell process for `script`. On Windows the script goes through `-EncodedCommand`, which
/// avoids every quoting problem of passing it as an argument, and prints UTF-8. The trailer
/// turns a failed last command into a nonzero exit code (native exit code when there is one).
#[cfg(windows)]
fn command_for(script: &str) -> Command {
    use base64::Engine as _;
    let full = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         {script}\n\
         if ($?) {{ exit 0 }} elseif ($LASTEXITCODE) {{ exit $LASTEXITCODE }} else {{ exit 1 }}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
    ])
    .arg(base64::engine::general_purpose::STANDARD.encode(utf16));
    cmd
}

#[cfg(not(windows))]
fn command_for(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
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
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_task = tokio::spawn(crate::proc::read_tail(stdout, 2 * MAX_OUTPUT));
    let err_task = tokio::spawn(crate::proc::read_tail(stderr, 2 * MAX_OUTPUT));

    let end = tokio::select! {
        s = child.wait() => End::Exited(s.ok().and_then(|s| s.code())),
        _ = tokio::time::sleep(Duration::from_secs(secs)) => End::TimedOut,
        _ = cancel.cancelled() => End::Cancelled,
    };
    if !matches!(end, End::Exited(_)) {
        if let Some(pid) = pid {
            crate::proc::kill_tree(pid).await;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    // A grandchild that survived the kill may hold the pipes open; never wait on it for long.
    let cap = Duration::from_secs(5);
    let out = tokio::time::timeout(cap, out_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let err = tokio::time::timeout(cap, err_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut text = out;
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
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
