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
    /// What Open Sage is doing now, in a few words ("Opening the Inbox"): shown, never said.
    Step {
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
    /// A voice setup is recording take `take` of `of`: the person says "Hey Sage" now.
    SettingUp {
        take: u8,
        of: u8,
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

/// What a voice setup found, for the settings page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VoiceSetup {
    /// How many takes were recorded, and how many of them Sage heard the phrase in.
    pub takes: u8,
    pub heard: u8,
    /// The voice reaches the microphone faintly.
    pub faint: bool,
    /// The outcome in a sentence or two.
    pub message: String,
}

impl VoiceSetup {
    pub fn of(found: &dots_voice::Calibration) -> Self {
        Self {
            takes: found.takes,
            heard: found.heard,
            faint: found.faint(),
            message: super::service::setup_message(found),
        }
    }
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
    fn a_voice_setup_is_told_in_plain_words() {
        use dots_voice::{Calibration, WakeTuning};
        let found = |heard: u8, level: Option<f32>| Calibration {
            takes: 5,
            heard,
            tuning: WakeTuning::Standard,
            level,
            by_text: 0,
            phrases: Vec::new(),
        };
        let good = VoiceSetup::of(&found(5, Some(0.05)));
        assert_eq!(
            good.message,
            "Sage heard you 5 times out of 5 and is set up for your voice."
        );
        assert!(!good.faint);
        // Four of five is still set up; a faint voice is told how to be heard better.
        let faint = VoiceSetup::of(&found(4, Some(0.001)));
        assert!(faint.faint);
        assert_eq!(
            faint.message,
            "Sage heard you 4 times out of 5 and is set up for your voice. Your voice reaches \
             the microphone faintly: nearer to it, or with a headset, Sage will hear you better."
        );
        let few = VoiceSetup::of(&found(2, Some(0.05)));
        assert_eq!(
            few.message,
            "Sage heard you only 2 times out of 5. Try again a little closer to the microphone, \
             saying it the same way each time."
        );
        // The words Sage learned for the phrase are not part of what the page is given.
        let learned = Calibration {
            by_text: 4,
            phrases: vec!["hate sage".into()],
            ..found(4, Some(0.05))
        };
        let shown = serde_json::to_string(&VoiceSetup::of(&learned)).unwrap();
        assert!(!shown.contains("hate"), "{shown}");
        assert_eq!(
            VoiceSetup::of(&learned).message,
            "Sage heard you 4 times out of 5 and is set up for your voice."
        );
        let none = VoiceSetup::of(&found(0, None));
        assert_eq!(
            none.message,
            "Sage did not hear \"Hey Sage\" in any of the 5 tries, so nothing was changed. Check \
             the microphone, move closer to it and try again."
        );
        assert_eq!(
            serde_json::to_value(VoiceEvent::SettingUp { take: 2, of: 5 }).unwrap(),
            json!({"kind": "setting_up", "take": 2, "of": 5})
        );
    }

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
        assert_eq!(
            serde_json::to_value(VoiceEvent::Step {
                text: "Opening the Inbox".into()
            })
            .unwrap(),
            json!({"kind": "step", "text": "Opening the Inbox"})
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
