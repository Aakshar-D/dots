//! Sets the wake phrase up for one person's voice, from a few takes of them saying it.
//!
//! The takes are replayed through the spotter at each `WakeTuning`, strictest first, and the
//! strictest that catches enough of them is the one to use. How loud the voice was in the
//! takes is kept too, so the microphone's gain starts where that voice needs it.
//!
//! The takes are also read by the recogniser, as a resting session reads what is said to it
//! (`WakeText`). When it writes this person's phrase as something other than "Hey Sage", and
//! does so more than once, those words are learned: said at the start of a sentence, they
//! wake the session too.
//!
//! No sound of the takes is kept. What is kept is the tuning, the level, the counts, and the
//! words learned for the phrase, as text (two words each, four at most).

use serde::{Deserialize, Serialize};

use crate::engine::{Recognizer, SpeechDetector, WakeSpotter, WakeWord, FRAME};
use crate::gain::Gain;
use crate::session::{plain_words, sage_like, wake_text_matches, IdleCutter, WAKE_TEXT_QUIET};
use crate::sherpa::WakeTuning;

/// How many takes a setup asks for.
pub const TAKES: u8 = 5;
/// Frames in one take: about three seconds.
pub const TAKE_FRAMES: usize = 94;
/// A tuning that catches this many of `TAKES` is good enough to use.
const ENOUGH: u8 = 4;
/// Frames of silence fed after a take, so the spotter decodes its last block.
const TAIL_FRAMES: usize = 24;
/// A voice whose level in the takes is below this (about 100 in 32768) reaches the
/// microphone faintly: the person is told so.
pub const FAINT: f32 = 0.003;
/// A frame this many times louder than the quietest frames of its take is the voice.
const OVER_NOISE: f32 = 3.0;
/// A take needs this many voice frames (about a quarter of a second) for its level to count.
const MIN_VOICE_FRAMES: usize = 8;
/// A take read as more words than this is not an attempt at the phrase: nothing is learned
/// from it.
const PHRASE_TAKE_WORDS: usize = 4;
/// Words are learned for the phrase only when this many takes were read as them: one take
/// misheard once must not become a way to wake Sage.
const PHRASE_TAKES: usize = 2;
/// The most phrases learned.
const MAX_PHRASES: usize = 4;

/// What a setup found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    /// How many takes were recorded.
    pub takes: u8,
    /// How many of them were caught: by the chosen tuning or by their words (`by_text`),
    /// whichever caught more. Which takes each caught is not compared, so the two together
    /// may catch more than this, never fewer.
    pub heard: u8,
    pub tuning: WakeTuning,
    /// The level of the voice in the takes (rms, of 1.0 full scale); `None` when no take had
    /// a voice in it.
    pub level: Option<f32>,
    /// How many takes were caught by their words: read as "Hey Sage", or as one of
    /// `phrases`. 0 in a setup saved before the words were read.
    #[serde(default)]
    pub by_text: u8,
    /// What this person's phrase was read as, when not as "Hey Sage": lower-case words with
    /// one space between. These wake the session too.
    #[serde(default)]
    pub phrases: Vec<String>,
}

impl Calibration {
    /// Whether the setup is worth keeping: the phrase was caught at least once.
    pub fn usable(&self) -> bool {
        self.heard > 0
    }

    pub fn faint(&self) -> bool {
        self.level.is_some_and(|level| level < FAINT)
    }

    /// This setup with what the takes were read as (`take_texts`): the words learned for
    /// the phrase, and the takes their words catch counted in. The tuning stays the one the
    /// spotter alone was best at.
    pub fn with_words(mut self, texts: &[String]) -> Self {
        let (by_text, phrases) = learn_phrases(texts);
        self.by_text = by_text.min(self.takes);
        self.phrases = phrases;
        self.heard = self.heard.max(self.by_text);
        self
    }
}

/// What the takes' words teach. `texts` holds what the recogniser wrote for each take.
/// Returns how many takes their words catch, and the phrases learned.
///
/// A take read as "Hey Sage" (`wake_text_matches`) is caught as it is. From any other take
/// of two to four words that is plainly an attempt at the phrase, the first two words are a
/// candidate: "hey" and any word ("Hey said"), or any word and one that sounds like "Sage"
/// ("Hate sage", "Pay sage"). Neither half alone is enough when the other is a
/// loose guess: "He said" and "Say it" were seen for the phrase too, and people begin
/// sentences with them all day. A candidate that at least `PHRASE_TAKES` takes gave is
/// learned, the commonest first, `MAX_PHRASES` at most. Takes read as a learned phrase are
/// caught too.
pub fn learn_phrases(texts: &[String]) -> (u8, Vec<String>) {
    let mut candidates: Vec<(String, usize)> = Vec::new();
    for text in texts {
        if wake_text_matches(text, &[]) {
            continue;
        }
        let words = plain_words(text);
        if !(2..=PHRASE_TAKE_WORDS).contains(&words.len()) {
            continue;
        }
        if !matches!(words[0].as_str(), "hey" | "hay") && !sage_like(&words[1]) {
            continue;
        }
        let phrase = format!("{} {}", words[0], words[1]);
        match candidates.iter_mut().find(|(known, _)| *known == phrase) {
            Some((_, times)) => *times += 1,
            None => candidates.push((phrase, 1)),
        }
    }
    candidates.retain(|(_, times)| *times >= PHRASE_TAKES);
    // Stable: of two as common, the one heard first comes first.
    candidates.sort_by_key(|(_, times)| std::cmp::Reverse(*times));
    let phrases: Vec<String> = candidates
        .into_iter()
        .take(MAX_PHRASES)
        .map(|(phrase, _)| phrase)
        .collect();
    let caught = texts
        .iter()
        .filter(|text| wake_text_matches(text, &phrases))
        .count();
    (u8::try_from(caught).unwrap_or(u8::MAX), phrases)
}

/// What `recognizer` writes for each take, heard as a resting session would hear it: cut by
/// `detector` into the clip that session would read (from just before the first words to
/// the quiet after them, or two seconds), and made as loud as a session that knows the
/// voice's `level` would make it. A take with nothing to read in it, or that cannot be
/// read, gives an empty text. The texts are for `Calibration::with_words` and are not to
/// be kept.
pub fn take_texts(
    detector: Box<dyn SpeechDetector>,
    recognizer: &dyn Recognizer,
    takes: &[Vec<Vec<f32>>],
    level: Option<f32>,
) -> Vec<String> {
    let mut gain = level.map_or_else(Gain::new, Gain::with_voice);
    let mut cutter = IdleCutter::new(detector);
    let quiet = vec![0.0; FRAME];
    takes
        .iter()
        .map(|take| {
            // Each take is said after a pause: it is heard from quiet.
            cutter.forget();
            // Quiet after the take ends the words of a person who spoke late.
            let tail = std::iter::repeat_n(&quiet, WAKE_TEXT_QUIET);
            let mut frames = take.iter().filter(|f| f.len() == FRAME).chain(tail);
            let ready = frames.any(|frame| cutter.hear(&gain.apply(frame), frame));
            // The gain hears the whole take, as it would have.
            let clip = ready.then(|| cutter.clip(gain.factor()));
            for frame in frames {
                gain.apply(frame);
            }
            clip.and_then(|clip| recognizer.transcribe(&clip).ok())
                .unwrap_or_default()
        })
        .collect()
}

fn rms(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// The level of the voice in one take: the mean level of the frames that stand out of the
/// take's quietest tenth. `None` when too few do.
fn take_level(take: &[Vec<f32>]) -> Option<f32> {
    let mut levels: Vec<f32> = take.iter().map(|frame| rms(frame)).collect();
    if levels.is_empty() {
        return None;
    }
    let mut sorted = levels.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let noise = sorted[sorted.len() / 10];
    levels.retain(|level| *level > noise * OVER_NOISE && *level > 0.0);
    (levels.len() >= MIN_VOICE_FRAMES).then(|| levels.iter().sum::<f32>() / levels.len() as f32)
}

/// The level of the voice over the takes: the middle one of those that had a voice.
pub fn voice_level(takes: &[Vec<Vec<f32>>]) -> Option<f32> {
    let mut levels: Vec<f32> = takes.iter().filter_map(|take| take_level(take)).collect();
    levels.sort_by(|a, b| a.total_cmp(b));
    levels.get(levels.len() / 2).copied()
}

/// How many times `wake` catches the wake phrase in `takes`, made as loud as a session that
/// knows the voice's `level` would make them. The takes are heard one after the other as one
/// sound, as they were recorded: a phrase said across the end of one take and the start of
/// the next is still caught. Never more than there are takes.
pub fn heard_times(wake: &mut dyn WakeSpotter, takes: &[Vec<Vec<f32>>], level: Option<f32>) -> u8 {
    let mut gain = level.map_or_else(Gain::new, Gain::with_voice);
    wake.reset();
    let quiet = vec![0.0; FRAME];
    let mut caught = 0usize;
    for frame in takes.iter().flatten() {
        if frame.len() != FRAME {
            continue;
        }
        if wake.heard(&gain.apply(frame)) == Some(WakeWord::HeyDot) {
            caught += 1;
        }
    }
    for _ in 0..TAIL_FRAMES {
        if wake.heard(&quiet) == Some(WakeWord::HeyDot) {
            caught += 1;
        }
    }
    wake.reset();
    u8::try_from(caught.min(takes.len())).unwrap_or(u8::MAX)
}

/// The tuning for these takes. `caught(tuning)` says how many times the spotter at `tuning`
/// catches the phrase in them. The strictest tuning that catches `ENOUGH` wins; when none
/// does, the one that catches the most, and the keener of two that tie: a voice no tuning
/// catches well is hard to hear, and the takes are too few to show what the keener one adds.
/// When none catches anything, nothing is known, and the tuning is the standard one.
pub fn calibrate(takes: &[Vec<Vec<f32>>], mut caught: impl FnMut(WakeTuning) -> u8) -> Calibration {
    let level = voice_level(takes);
    let count = u8::try_from(takes.len()).unwrap_or(u8::MAX);
    let enough = ENOUGH.min(count);
    let mut best = (WakeTuning::Standard, 0u8);
    for tuning in WakeTuning::STRICTEST_FIRST {
        let heard = caught(tuning).min(count);
        if heard > 0 && heard >= best.1 {
            best = (tuning, heard);
        }
        if heard >= enough && heard > 0 {
            break;
        }
    }
    Calibration {
        takes: count,
        heard: best.1,
        tuning: best.0,
        level,
        by_text: 0,
        phrases: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineError;

    fn tone(level: f32) -> Vec<f32> {
        (0..FRAME)
            .map(|i| level * std::f32::consts::SQRT_2 * (i as f32 * 0.3).sin())
            .collect()
    }

    /// A take: quiet, then `voice_frames` at `level`, then quiet.
    fn take(level: f32, voice_frames: usize) -> Vec<Vec<f32>> {
        let mut frames = vec![tone(0.000_1); 30];
        frames.extend(vec![tone(level); voice_frames]);
        frames.extend(vec![tone(0.000_1); TAKE_FRAMES - 30 - voice_frames]);
        frames
    }

    fn five_of(take: Vec<Vec<f32>>) -> Vec<Vec<Vec<f32>>> {
        vec![take; 5]
    }

    fn five(level: f32) -> Vec<Vec<Vec<f32>>> {
        five_of(take(level, 20))
    }

    #[test]
    fn the_strictest_tuning_that_catches_four_of_five_is_used() {
        let mut asked = Vec::new();
        let got = calibrate(&five(0.05), |tuning| {
            asked.push(tuning);
            5
        });
        assert_eq!(
            (got.tuning, got.heard, got.takes),
            (WakeTuning::Strict, 5, 5)
        );
        // Nothing looser was tried.
        assert!(asked.iter().all(|t| *t == WakeTuning::Strict));
    }

    #[test]
    fn a_looser_tuning_is_used_when_the_strict_one_misses_too_many() {
        let got = calibrate(&five(0.001), |tuning| match tuning {
            WakeTuning::Strict => 1,
            WakeTuning::Standard => 4,
            WakeTuning::Sensitive => 5,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 4));
    }

    #[test]
    fn when_none_catches_enough_the_one_that_catches_most_is_used() {
        let got = calibrate(&five(0.001), |tuning| match tuning {
            WakeTuning::Strict => 0,
            WakeTuning::Standard => 1,
            WakeTuning::Sensitive => 3,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Sensitive, 3));
        assert!(got.usable());
    }

    #[test]
    fn two_that_tie_below_enough_give_the_keener() {
        let got = calibrate(&five(0.001), |_| 0);
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 0));
        assert!(!got.usable());
        let got = calibrate(&five(0.001), |_| 2);
        assert_eq!((got.tuning, got.heard), (WakeTuning::Sensitive, 2));
        // What a hard voice gave: the standard and the keen tuning both caught three.
        let got = calibrate(&five(0.017), |tuning| match tuning {
            WakeTuning::Strict => 1,
            WakeTuning::Standard | WakeTuning::Sensitive => 3,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Sensitive, 3));
        // A keener tuning that catches fewer does not win.
        let got = calibrate(&five(0.017), |tuning| match tuning {
            WakeTuning::Strict => 1,
            WakeTuning::Standard => 3,
            WakeTuning::Sensitive => 2,
        });
        assert_eq!((got.tuning, got.heard), (WakeTuning::Standard, 3));
    }

    #[test]
    fn the_level_is_that_of_the_voice_not_of_the_quiet_around_it() {
        let level = voice_level(&five(0.002)).unwrap();
        assert!((level - 0.002).abs() < 0.000_2, "{level}");
        let got = calibrate(&five(0.002), |_| 5);
        assert!(got.faint());
        assert!(!calibrate(&five(0.05), |_| 5).faint());
        // A spotter that fires more often than there are takes caught them all, no more.
        assert_eq!(calibrate(&five(0.05), |_| 9).heard, 5);
    }

    #[test]
    fn takes_with_no_voice_have_no_level() {
        let silent = five_of(vec![tone(0.000_1); TAKE_FRAMES]);
        assert_eq!(voice_level(&silent), None);
        // A click of three frames is not a voice.
        assert_eq!(voice_level(&five_of(take(0.05, 3))), None);
        let got = calibrate(&silent, |_| 0);
        assert_eq!((got.level, got.heard), (None, 0));
        assert!(!got.faint());
    }

    #[test]
    fn a_calibration_is_stored_as_plain_json() {
        let c = Calibration {
            takes: 5,
            heard: 4,
            tuning: WakeTuning::Sensitive,
            level: Some(0.5),
            by_text: 4,
            phrases: vec!["hate sage".into()],
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(
            json,
            r#"{"takes":5,"heard":4,"tuning":"sensitive","level":0.5,"by_text":4,"phrases":["hate sage"]}"#
        );
        assert_eq!(serde_json::from_str::<Calibration>(&json).unwrap(), c);
    }

    /// A setup saved before the takes' words were read has neither of the new fields.
    #[test]
    fn a_setup_saved_before_the_words_were_read_still_loads() {
        let old = r#"{"takes":5,"heard":3,"tuning":"sensitive","level":0.019}"#;
        let c: Calibration = serde_json::from_str(old).unwrap();
        assert_eq!(
            c,
            Calibration {
                takes: 5,
                heard: 3,
                tuning: WakeTuning::Sensitive,
                level: Some(0.019),
                by_text: 0,
                phrases: Vec::new(),
            }
        );
        assert!(c.usable());
    }

    fn texts(of: &[&str]) -> Vec<String> {
        of.iter().map(|text| text.to_string()).collect()
    }

    #[test]
    fn takes_read_as_the_phrase_are_caught_and_teach_nothing() {
        let heard = texts(&["Hey Sage.", "Hey, Saige.", "hey sedge", "Hay stage.", ""]);
        assert_eq!(learn_phrases(&heard), (4, Vec::new()));
        assert_eq!(learn_phrases(&[]), (0, Vec::new()));
    }

    #[test]
    fn words_two_takes_were_read_as_are_learned_and_then_caught() {
        // "Hate sage" twice: learned. "Pay sage" once: a mishearing of one take, not learned.
        let heard = texts(&["Hey Sage.", "Hate sage.", "Hate, Sage!", "Pay sage.", ""]);
        assert_eq!(learn_phrases(&heard), (3, vec!["hate sage".to_string()]));
        // A first word "hey" is misheard as, with any second: "Hey said" is learned.
        let heard = texts(&["Hey said.", "Hey said", "Hey, said it.", "Hey Sage.", "Hm."]);
        assert_eq!(learn_phrases(&heard), (4, vec!["hey said".to_string()]));
        // A second word that sounds like "Sage", with any first.
        let heard = texts(&[
            "His siege.",
            "His siege.",
            "Sage.",
            "This age.",
            "This age.",
        ]);
        assert_eq!(learn_phrases(&heard), (2, vec!["his siege".to_string()]));
    }

    #[test]
    fn nothing_is_learned_from_takes_that_are_not_the_phrase() {
        // Too long to be the phrase, one word only, or neither word anything like it.
        let heard = texts(&[
            "Hey, what time is the meeting today?",
            "Hey, what time is the meeting today?",
            "Sage.",
            "Sage.",
            "This age.",
        ]);
        assert_eq!(learn_phrases(&heard), (0, Vec::new()));
        // Each candidate came once.
        let heard = texts(&[
            "Hate sage.",
            "Pay sage.",
            "He stage.",
            "Hi siege.",
            "Eh, cage.",
        ]);
        assert_eq!(learn_phrases(&heard), (0, Vec::new()));
        // Seen for the phrase, but what people say all day: never learned, however often.
        let heard = texts(&["He said.", "He said.", "Say it.", "Say it.", "Okay, so."]);
        assert_eq!(learn_phrases(&heard), (0, Vec::new()));
    }

    #[test]
    fn the_commonest_words_come_first_and_four_phrases_are_the_most() {
        let heard = texts(&[
            "Pay sage.",
            "Hate sage.",
            "Hate sage.",
            "Pay sage.",
            "Hate sage.",
            "He sage",
            "He sage",
            "Hi sage",
            "Hi sage",
            "Eh sage",
            "Eh sage",
        ]);
        let (caught, phrases) = learn_phrases(&heard);
        assert_eq!(phrases, ["hate sage", "pay sage", "he sage", "hi sage"]);
        assert_eq!(caught, 9);
    }

    #[test]
    fn heard_is_what_the_spotter_or_the_words_catch_and_the_tuning_is_the_spotters() {
        let spotted = calibrate(&five(0.02), |tuning| match tuning {
            WakeTuning::Strict => 1,
            WakeTuning::Standard | WakeTuning::Sensitive => 3,
        });
        let heard = texts(&["Hey Sage.", "Hate sage.", "Hate sage.", "Hey Sage.", "Hey."]);
        let found = spotted.clone().with_words(&heard);
        assert_eq!((found.heard, found.by_text), (4, 4));
        assert_eq!(found.phrases, ["hate sage"]);
        assert_eq!(found.tuning, WakeTuning::Sensitive);
        // Words that catch fewer takes than the spotter take nothing away.
        let found = spotted.with_words(&texts(&["Hey Sage.", "", "", "", ""]));
        assert_eq!((found.heard, found.by_text), (3, 1));
        // Nothing caught either way is still not usable.
        let none = calibrate(&five(0.02), |_| 0).with_words(&texts(&["", "", "", "", ""]));
        assert!(!none.usable());
        // The words alone make a setup usable, and never count more takes than there were.
        let words = calibrate(&five(0.02), |_| 0).with_words(&texts(&["Hey Sage."; 7]));
        assert_eq!((words.heard, words.by_text), (5, 5));
        assert!(words.usable());
    }

    /// Calls a frame speech when it is at least `over` loud.
    struct Loud {
        over: f32,
    }

    impl SpeechDetector for Loud {
        fn is_speech(&mut self, frame: &[f32]) -> bool {
            rms(frame) >= self.over
        }
        fn reset(&mut self) {}
    }

    /// Writes "Hey Sage." for a clip and keeps each clip's length, in frames.
    #[derive(Default)]
    struct Reads {
        frames: std::sync::Mutex<Vec<usize>>,
        fails: bool,
    }

    impl Recognizer for Reads {
        fn transcribe(&self, clip: &crate::engine::Clip) -> Result<String, EngineError> {
            self.frames.lock().unwrap().push(clip.samples.len() / FRAME);
            if self.fails {
                return Err(EngineError::Failed("broken".into()));
            }
            Ok("Hey Sage.".into())
        }
    }

    #[test]
    fn each_take_is_read_as_a_resting_session_would_read_it() {
        let reads = Reads::default();
        let loud = || Box::new(Loud { over: 0.02 });
        // A faint voice is made loud first, as the session makes it: 0.001 becomes 0.03.
        let mut takes = five(0.001);
        // A click is not read; a take of quiet has nothing to read.
        takes[1] = take(0.001, 3);
        takes[2] = vec![tone(0.000_1); TAKE_FRAMES];
        // Said late: the take ends before the quiet that would end the words.
        takes[3] = vec![tone(0.000_1); TAKE_FRAMES - 12];
        takes[3].extend(vec![tone(0.001); 12]);
        let got = take_texts(loud(), &reads, &takes, Some(0.001));
        assert_eq!(got, ["Hey Sage.", "", "", "Hey Sage.", "Hey Sage."]);
        // The words with ten frames before them and the ten quiet ones that ended them.
        assert_eq!(*reads.frames.lock().unwrap(), [40, 32, 40]);
        // A recogniser that cannot read gives empty texts, not an error.
        let broken = Reads {
            fails: true,
            ..Reads::default()
        };
        let got = take_texts(loud(), &broken, &five(0.05), Some(0.05));
        assert_eq!(got, ["", "", "", "", ""]);
    }

    /// Catches the phrase when a frame at least `over` loud comes after a quieter one.
    struct Spots {
        over: f32,
        loud: bool,
    }

    impl WakeSpotter for Spots {
        fn heard(&mut self, frame: &[f32]) -> Option<WakeWord> {
            let loud = rms(frame) >= self.over;
            let began = loud && !self.loud;
            self.loud = loud;
            began.then_some(WakeWord::HeyDot)
        }
        fn reset(&mut self) {
            self.loud = false;
        }
    }

    #[test]
    fn takes_are_heard_as_loud_as_the_session_would_make_them() {
        let mut wake = Spots {
            over: 0.02,
            loud: false,
        };
        let faint = five(0.001);
        // Unknown level: the gain finds the voice as it goes.
        assert_eq!(heard_times(&mut wake, &faint, None), 5);
        // Known level: loud from the first frame.
        assert_eq!(heard_times(&mut wake, &faint, Some(0.001)), 5);
        // Silence is never the phrase.
        let silent = five_of(vec![tone(0.000_1); TAKE_FRAMES]);
        assert_eq!(heard_times(&mut wake, &silent, Some(0.001)), 0);
    }

    /// The person spoke late: the phrase began at the end of one take and ended in the next.
    #[test]
    fn a_phrase_said_across_two_takes_is_caught_once() {
        let mut wake = Spots {
            over: 0.02,
            loud: false,
        };
        let mut first = vec![tone(0.000_1); TAKE_FRAMES - 10];
        first.extend(vec![tone(0.05); 10]);
        let mut second = vec![tone(0.05); 10];
        second.extend(vec![tone(0.000_1); TAKE_FRAMES - 10]);
        assert_eq!(heard_times(&mut wake, &[first, second], Some(0.05)), 1);
    }
}
