use tokio::io::{AsyncRead, AsyncReadExt};

/// Windows `CREATE_NO_WINDOW` process creation flag.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Prevents a console window from flashing up when a GUI process spawns a console program.
pub fn hide_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Kills a process and all of its descendants. Best effort; errors are ignored.
pub async fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut cmd = tokio::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        hide_window(&mut cmd);
        let _ = cmd.status().await;
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .await;
    }
}

/// Reads `r` to the end and returns its last `max` bytes, lossily decoded as UTF-8.
pub async fn read_tail(mut r: impl AsyncRead + Unpin, max: usize) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > max {
                    let excess = buf.len() - max;
                    buf.drain(..excess);
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}
