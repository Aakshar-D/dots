//! The phone view acts through `Runtime`, so a phone's decision is the app's decision: the
//! account's role checks, the private-folder guard and the resume of a parked run all apply.

use async_trait::async_trait;

use super::PhoneBackend;
use crate::model::{Approval, Inbox, Message, Run};
use crate::{Result, Runtime};

#[async_trait]
impl PhoneBackend for Runtime {
    async fn inbox(&self) -> Result<Inbox> {
        Runtime::inbox(self).await
    }

    async fn waiting_messages(&self) -> Result<Vec<Message>> {
        self.store().waiting_messages().await
    }

    async fn decide_approval(&self, id: &str, approved: bool) -> Result<Approval> {
        // No rule: "always allow" is never offered on a phone.
        Runtime::decide_approval(self, id, approved, None, None).await
    }

    async fn note_decided_via(&self, id: &str, via: &str) -> Result<()> {
        Runtime::note_decided_via(self, id, via).await
    }

    async fn reply(&self, id: &str, text: &str) -> Result<Run> {
        Runtime::reply(self, id, text).await
    }

    async fn run_release_hold(&self, run_id: &str) -> Result<Run> {
        Runtime::run_release_hold(self, run_id).await
    }
}
