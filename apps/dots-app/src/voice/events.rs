//! What the UI receives on the `voice` event, and the status it reads.

use dots_core::open_dot::act::OpenTarget;
use dots_core::open_dot::Card;
use dots_voice::State;
use serde::Serialize;

/// One `voice` event for the UI.
// Sent once each, by value, and serialised at once: not boxed, as `MindEvent` is not.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    State {
        state: State,
        mic_open: bool,
    },
    Level {
        mic: f32,
        voice: f32,
    },
    /// Live words of the sentence the person is saying.
    Hearing {
        text: String,
    },
    /// A finished sentence (or a clicked option): it went to the mind.
    Heard {
        text: String,
    },
    /// A sentence handed to the speaker.
    Saying {
        text: String,
    },
    /// A question that waits on the person, with its options (none for an open question).
    /// `seq` names this question: a pick carries it back, and one for any other question is
    /// ignored. It starts at 1 per process and grows by one per question, so it stays far
    /// below 2^53 and crosses to the UI as an exact JSON number.
    Ask {
        question: String,
        options: Vec<String>,
        seq: u64,
    },
    Card {
        card: Card,
    },
    Open {
        target: OpenTarget,
    },
    /// The options and the card should go away.
    Cleared,
    /// "Hey Dot" was heard while idle.
    Woke,
    Trouble {
        message: String,
    },
    Download {
        done_bytes: u64,
        total_bytes: u64,
    },
    /// Sent whenever the status changes.
    Status {
        status: VoiceStatus,
    },
}

/// Where voice stands, for the settings page and the rail.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceStatus {
    pub enabled: bool,
    pub installed: bool,
    pub downloading: bool,
    /// The session is built and running.
    pub ready: bool,
    pub state: State,
    pub mic_open: bool,
    /// `Manifest::pinned().total_bytes()`.
    pub download_bytes: u64,
    pub hotkey_error: Option<String>,
    pub trouble: Option<String>,
}

/// A microphone or a speaker the person can choose.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceDevice {
    pub id: String,
    pub name: String,
    /// The system's default.
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct VoiceDevices {
    pub microphones: Vec<VoiceDevice>,
    pub speakers: Vec<VoiceDevice>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_serialise_with_a_kind_tag() {
        let ev = VoiceEvent::State {
            state: State::Asking,
            mic_open: true,
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({"kind": "state", "state": "asking", "mic_open": true})
        );
        assert_eq!(
            serde_json::to_value(VoiceEvent::Cleared).unwrap(),
            json!({"kind": "cleared"})
        );
        let ev = VoiceEvent::Open {
            target: OpenTarget::Inbox,
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({"kind": "open", "target": {"kind": "inbox"}})
        );
    }
}
