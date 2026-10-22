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

/// The processes of one spawned child: the child and everything it starts.
///
/// On Windows this is a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. A process started
/// by a member is a member too, and stays one when its parent exits, which is where `kill_tree`
/// loses it. `terminate` kills every member. So does dropping the `Job`, and so does the end of
/// Dots itself, because the `Job` holds the only handle to the job object.
///
/// On other systems, and when Windows refuses the job object, a `Job` is only the child's
/// process id: `terminate` is `kill_tree` and dropping it does nothing.
pub struct Job {
    #[cfg(windows)]
    object: Option<std::os::windows::io::OwnedHandle>,
    pid: Option<u32>,
}

impl Job {
    /// Puts `child` into a new job. To be called right after the spawn: what the child starts
    /// before this call is outside the job. An error means that Windows gave no job object;
    /// `fallback` is what is left then.
    pub fn assign(child: &tokio::process::Child) -> std::io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                object: Some(job_object::holding(child)?),
                pid: child.id(),
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self::fallback(child))
        }
    }

    /// A job without a job object.
    pub fn fallback(child: &tokio::process::Child) -> Self {
        Self {
            #[cfg(windows)]
            object: None,
            pid: child.id(),
        }
    }

    /// Kills every process of the job. Without a job object that is `kill_tree` on the child's
    /// process id, so the child must not have been dropped yet.
    pub async fn terminate(&self) {
        #[cfg(windows)]
        if let Some(object) = &self.object {
            match job_object::terminate(object) {
                Ok(()) => return,
                Err(e) => tracing::warn!("terminating the job object failed: {e}"),
            }
        }
        if let Some(pid) = self.pid {
            kill_tree(pid).await;
        }
    }

    /// Ends the job at a chosen point, which kills what is still in it. This is dropping it,
    /// under a name that says so; without a job object nothing happens.
    pub fn close(self) {}
}

/// The Win32 calls behind `Job`.
#[cfg(windows)]
mod job_object {
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// A new job object with `child` in it. The handle returned is the only one there is: no
    /// process inherits it, so closing it ends the job.
    pub(super) fn holding(child: &tokio::process::Child) -> io::Result<OwnedHandle> {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the process has already exited"))?;
        // SAFETY: both pointers may be null, for the default security (a handle that child
        // processes do not inherit) and no name.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is an open handle that nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
                LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: `limits` is the structure this information class takes, it outlives the call
        // and the length given is its size.
        let set = unsafe {
            SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of_val(&limits) as u32,
            )
        };
        if set == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: both handles are open: `job` owns the first, and tokio keeps the second for
        // as long as `child` has not been waited for.
        if unsafe { AssignProcessToJobObject(raw, process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Ends every process in the job; each exits with code 1.
    pub(super) fn terminate(job: &OwnedHandle) -> io::Result<()> {
        // SAFETY: the handle is open for as long as `job` is borrowed.
        if unsafe { TerminateJobObject(job.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Reads `r` to the end and returns its last `max` bytes, lossily decoded as UTF-8, with the
/// secret values scrubbed out. The tail is cut after the scrub: a secret across the cut would
/// leave a piece that no scrub matches.
pub async fn read_scrubbed_tail(r: impl AsyncRead + Unpin, max: usize) -> String {
    let keep = max + crate::vault::redact::global().longest();
    let text = read_tail(r, keep).await;
    let mut text = if text.len() >= keep {
        crate::vault::redact::global().scrub_clipped_head(&text)
    } else {
        crate::vault::redact::global().scrub(&text).into_owned()
    };
    if text.len() > max {
        let mut start = text.len() - max;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text.drain(..start);
    }
    text
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
