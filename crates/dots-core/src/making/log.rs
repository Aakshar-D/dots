//! The makers' log (dots-make-dots design sections 10 and 11): what a maker's run did, kept
//! for the cards of the maker's chat topic and for the count of dots one run made.

use serde::{Deserialize, Serialize};
use serde_json::Value;

str_enum!(MakingEventKind {
    // The maker's run made a dot. `detail`: `{"purpose": ..}`.
    Made => "made",
});

/// One thing a maker's run did: one card of the maker's chat topic (design section 11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MakingEvent {
    pub id: String,
    /// The maker.
    pub maker_dot_id: String,
    /// The maker's run that did it.
    pub run_id: String,
    /// The maker's chat topic: the first run of the thread `run_id` belongs to.
    pub root_run_id: String,
    pub kind: MakingEventKind,
    /// The made dot it is about; `None` once that dot is deleted.
    pub dot_id: Option<String>,
    /// The made dot's name when it happened.
    pub dot_name: String,
    /// What the card says besides, by kind (`MakingEventKind`).
    pub detail: Value,
    pub created_at: String,
}

/// Input for `Store::add_making_event`.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMakingEvent<'a> {
    pub maker_dot_id: &'a str,
    pub run_id: &'a str,
    pub root_run_id: &'a str,
    pub kind: MakingEventKind,
    pub dot_id: Option<&'a str>,
    pub dot_name: &'a str,
    pub detail: Value,
}
