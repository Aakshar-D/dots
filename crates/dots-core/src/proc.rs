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
