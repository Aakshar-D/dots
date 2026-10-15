//! A clock the test moves by hand, for the scheduler (Scheduler v2 spec §5, "Clock"). Time
//! stands still until `set` moves it; a sleep ends when the time reaches its deadline. Each
//! sleep is counted when it begins, which is when the scheduler's pass is over, so a test
//! waits for the `n`th sleep to know that the loop is idle again.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dots_core::scheduler::Clock;
use tokio::sync::watch;

#[derive(Debug, Clone)]
struct State {
    now: DateTime<Utc>,
    /// Sleeps begun so far.
    sleeps: u64,
    /// The deadline of the newest sleep.
    deadline: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct FakeClock {
    state: watch::Sender<State>,
}

impl FakeClock {
    pub fn new(now: DateTime<Utc>) -> Arc<FakeClock> {
        Arc::new(FakeClock {
            state: watch::Sender::new(State {
                now,
                sleeps: 0,
                deadline: None,
            }),
        })
    }

    /// Moves the time to `t`; the sleeps whose deadline it reaches end.
    pub fn set(&self, t: DateTime<Utc>) {
        self.state.send_modify(|s| s.now = t);
    }

    /// Waits (at most 10 real seconds) until `n` sleeps have begun, and returns the deadline
    /// of the newest one.
    pub async fn nth_sleep(&self, n: u64) -> DateTime<Utc> {
        let mut rx = self.state.subscribe();
        let state = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|s| s.sleeps >= n))
            .await
            .unwrap_or_else(|_| panic!("the scheduler did not begin sleep {n}"))
            .unwrap()
            .clone();
        state.deadline.unwrap()
    }

    pub fn sleeps(&self) -> u64 {
        self.state.borrow().sleeps
    }
}

#[async_trait]
impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.state.borrow().now
    }

    async fn sleep_until(&self, deadline: DateTime<Utc>) {
        let mut rx = self.state.subscribe();
        self.state.send_modify(|s| {
            s.sleeps += 1;
            s.deadline = Some(deadline);
        });
        let _ = rx.wait_for(|s| s.now >= deadline).await;
    }
}
