//! Keeping this PC awake (always-on workflows spec section 3): while it is asked to, the PC
//! does not go to sleep on its own for being idle. It still sleeps when the person says so or
//! closes the lid, and the screen may still turn off.
//!
//! On Windows the request belongs to the thread that made it, so one thread of its own makes
//! and clears it. On macOS a `caffeinate -i` runs for as long as it is wanted. Elsewhere
//! nothing is done.

use std::sync::mpsc::{channel, Sender};

/// Asks the PC to stay awake, or lets it sleep again.
pub struct Awake {
    tx: Sender<bool>,
}

impl Default for Awake {
    fn default() -> Self {
        Self::new()
    }
}

impl Awake {
    /// Starts the thread that holds the request. The PC may sleep until `set(true)`.
    pub fn new() -> Awake {
        let (tx, rx) = channel::<bool>();
        let spawned = std::thread::Builder::new()
            .name("sage-awake".into())
            .spawn(move || {
                let mut holder = Holder::default();
                let mut on = false;
                // Ends when the `Awake` is dropped: the request goes with the thread.
                while let Ok(want) = rx.recv() {
                    if want != on {
                        holder.hold(want);
                        on = want;
                    }
                }
                holder.hold(false);
            });
        if let Err(e) = spawned {
            tracing::warn!("the keep-awake thread could not start: {e}");
        }
        Awake { tx }
    }

    /// Keeps the PC awake (`true`) or lets it sleep on its own again (`false`). Asking for what
    /// is already so does nothing.
    pub fn set(&self, on: bool) {
        let _ = self.tx.send(on);
    }
}

#[cfg(windows)]
#[derive(Default)]
struct Holder;

#[cfg(windows)]
impl Holder {
    fn hold(&mut self, on: bool) {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };
        let state = if on {
            ES_CONTINUOUS | ES_SYSTEM_REQUIRED
        } else {
            ES_CONTINUOUS
        };
        // SAFETY: a plain call with flag constants; it changes only this thread's request.
        if unsafe { SetThreadExecutionState(state) } == 0 {
            tracing::warn!("Windows refused the keep-awake request");
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Default)]
struct Holder {
    child: Option<std::process::Child>,
}

#[cfg(target_os = "macos")]
impl Holder {
    fn hold(&mut self, on: bool) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if on {
            match std::process::Command::new("caffeinate").arg("-i").spawn() {
                Ok(child) => self.child = Some(child),
                Err(e) => tracing::warn!("caffeinate could not start: {e}"),
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
#[derive(Default)]
struct Holder;

#[cfg(not(any(windows, target_os = "macos")))]
impl Holder {
    fn hold(&mut self, _on: bool) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_and_letting_go_never_fails() {
        // What the PC then does cannot be seen from here: the calls must not panic or block.
        let awake = Awake::new();
        awake.set(true);
        awake.set(true);
        awake.set(false);
        drop(awake);
    }
}
