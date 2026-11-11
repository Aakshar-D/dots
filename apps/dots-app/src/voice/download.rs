//! The speech files' download: progress at most ten times a second, and every failure as a
//! sentence for people. Only sizes and state names move; nothing is logged here.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dots_voice::{install, Manifest, ModelError, Progress};
use tokio_util::sync::CancellationToken;

use crate::error::CmdError;

/// The UI hears of progress at most this often (and once at the end).
pub const PROGRESS_EVERY: Duration = Duration::from_millis(100);

pub const CANCELLED: &str = "The download was cancelled.";
pub const RESUMES: &str =
    "The download stopped. It picks up where it left off when you turn Open Dot on again.";
pub const DAMAGED: &str = "The download was damaged. Try again.";
pub const NO_ROOM: &str = "There is not enough room, or the folder cannot be written.";
pub const BUSY: &str = "The speech files are busy. Try again in a moment.";

/// What a failed download tells the person.
pub fn sentence(e: &ModelError) -> CmdError {
    match e {
        ModelError::Cancelled => CmdError::cancelled(CANCELLED),
        ModelError::Stalled | ModelError::Network(_) => CmdError::internal(RESUMES),
        ModelError::WrongHash { .. } => CmdError::internal(DAMAGED),
        ModelError::Disk(_) => CmdError::internal(NO_ROOM),
        ModelError::Busy => CmdError::conflict(BUSY),
    }
}

/// Installs `manifest` into `root`. `progress(done, total)` is called at most every
/// [`PROGRESS_EVERY`], and always for the last report.
pub async fn run(
    root: &Path,
    manifest: &Manifest,
    cancel: CancellationToken,
    progress: impl Fn(u64, u64) + Send + Sync,
) -> Result<(), ModelError> {
    let last: Mutex<Option<Instant>> = Mutex::new(None);
    let report = |p: Progress| {
        let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
        let done = p.done_bytes >= p.total_bytes;
        if done || last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            *last = Some(Instant::now());
            progress(p.done_bytes, p.total_bytes);
        }
    };
    install(root, manifest, report, cancel).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CmdErrorKind;

    #[test]
    fn every_failure_is_a_sentence() {
        let cases = [
            (ModelError::Cancelled, CmdErrorKind::Cancelled, CANCELLED),
            (ModelError::Stalled, CmdErrorKind::Internal, RESUMES),
            (
                ModelError::Network("reset".into()),
                CmdErrorKind::Internal,
                RESUMES,
            ),
            (
                ModelError::WrongHash {
                    file: "speak".into(),
                },
                CmdErrorKind::Internal,
                DAMAGED,
            ),
            (
                ModelError::Disk("full".into()),
                CmdErrorKind::Internal,
                NO_ROOM,
            ),
            (ModelError::Busy, CmdErrorKind::Conflict, BUSY),
        ];
        for (e, kind, message) in cases {
            let c = sentence(&e);
            assert_eq!((c.kind, c.message.as_str()), (kind, message), "{e:?}");
        }
    }
}
