//! Which dots are acting (watchers spec section 4.1): a dot acts while the runner holds one of
//! its runs (resume runs and the calls Dots performs inside them included) and for
//! `ACTING_GRACE` after the last one ends. A folder event reported while its dot acts is the
//! dot's own doing and wakes nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};

use crate::scheduler::Clock;

/// How long a dot still counts as acting after its last run ends.
pub const ACTING_GRACE: TimeDelta = TimeDelta::seconds(5);

#[derive(Debug, Default)]
struct Span {
    held: u32,
    last_end: Option<DateTime<Utc>>,
}

/// Sync on purpose: the folder source asks it from the OS's callback.
#[derive(Debug)]
pub struct ActingTracker {
    clock: Arc<dyn Clock>,
    dots: Mutex<HashMap<String, Span>>,
}

impl ActingTracker {
    pub fn new(clock: Arc<dyn Clock>) -> Arc<ActingTracker> {
        Arc::new(ActingTracker {
            clock,
            dots: Mutex::new(HashMap::new()),
        })
    }

    /// The runner took a run of the dot.
    pub fn begin(&self, dot_id: &str) {
        let mut dots = self.dots.lock().unwrap();
        dots.entry(dot_id.to_string()).or_default().held += 1;
    }

    /// The runner let a run of the dot go; the last one stamps the time.
    pub fn end(&self, dot_id: &str) {
        let now = self.clock.now();
        let mut dots = self.dots.lock().unwrap();
        let span = dots.entry(dot_id.to_string()).or_default();
        span.held = span.held.saturating_sub(1);
        if span.held == 0 {
            span.last_end = Some(now);
        }
    }

    /// Whether the dot acts at `at`: a run is held, or the last ended less than
    /// `ACTING_GRACE` before.
    pub fn is_acting(&self, dot_id: &str, at: DateTime<Utc>) -> bool {
        let dots = self.dots.lock().unwrap();
        match dots.get(dot_id) {
            None => false,
            Some(span) => span.held > 0 || span.last_end.is_some_and(|end| at < end + ACTING_GRACE),
        }
    }
}
