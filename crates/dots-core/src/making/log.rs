//! The makers' log (dots-make-dots design sections 10 and 11): what a maker's run did, kept
//! for the cards of the maker's chat topic and for the count of dots one run made.

use serde::{Deserialize, Serialize};
use serde_json::Value;

str_enum!(MakingEventKind {
    // The maker's run made a dot. `detail`: `{"purpose": ..}`.
    Made => "made",
    // The maker's run asked for a dot that is beyond its reach or over a limit: a make request
    // waits for the person (design section 7.2). `dot_name` is the name asked for.
    // `detail`: `{"request_id": ..}`.
    Asked => "asked",
    // The person answered a make request. `dot_id` is the dot the answer made, if any.
    // `detail`: `{"request_id": .., "decision": "allow" | "without" | "refuse", "dot_name": ..}`.
    Answered => "answered",
    // The maker's run changed a dot it made, or the person's answer to a change request did
    // (design section 9). `dot_name` is the dot's name after the change.
    // `detail`: `{"what": [..]}`, the settings that changed, in plain words.
    Changed => "changed",
    // The maker's run deleted a dot it made (design section 9). `dot_id` is `None`: the dot is
    // gone. `detail`: `{}`.
    Deleted => "deleted",
    // The maker gave a job to a dot it made, or a first job started with the dot (design
    // section 8): "<made dot> is working". `run_id` is the maker's run that gave it.
    // `detail`: `{"job_id": .., "message": .., "run_id": ..}`, where `run_id` is the made dot's
    // run. The loop guard counts these entries of a topic.
    JobStarted => "job_started",
    // The job's run ended with a question waiting for the person: "<made dot> is waiting for
    // your answer". `detail`: `{"job_id": .., "run_id": ..}`, the made dot's run that asked or
    // ended last.
    JobWaiting => "job_waiting",
    // The job ended: "<made dot> finished", failed or was stopped. `dot_id` is `None` when the
    // dot was deleted. `detail`: `{"job_id": .., "run_id": .., "how": ..}`, the made dot's last
    // run and how the job ended (`JobEnd`).
    JobEnded => "job_ended",
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
