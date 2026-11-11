//! Runs the real engines against the real models. Ignored by default: set
//! `DOTS_VOICE_MODELS` to a models folder in the manifest layout and run with `--ignored`.

use std::time::Instant;

use dots_voice::{
    resample, Clip, ModelPaths, Recognizer, SherpaDetector, SherpaRecognizer, SherpaSpeaker,
    SherpaWake, Speaker, SpeechDetector, WakeSpotter, WakeWord, EAR_RATE, FRAME,
};

fn paths() -> ModelPaths {
    let root = std::env::var_os("DOTS_VOICE_MODELS")
        .expect("set DOTS_VOICE_MODELS to the models folder to run these tests");
    ModelPaths { root: root.into() }
}

fn ears(clip: &Clip) -> Vec<f32> {
    resample(clip, EAR_RATE).samples
}

/// Feed `samples` frame by frame, then half a second of silence; the first word heard.
fn first_wake(wake: &mut SherpaWake, samples: &[f32]) -> Option<WakeWord> {
    wake.reset();
    let tail = vec![0.0f32; EAR_RATE as usize / 2];
    samples
        .chunks(FRAME)
        .chain(tail.chunks(FRAME))
        .map(|chunk| {
            let mut frame = chunk.to_vec();
            frame.resize(FRAME, 0.0);
            frame
        })
        .find_map(|frame| wake.heard(&frame))
}

#[test]
#[ignore]
fn it_hears_what_it_says() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let recognizer = SherpaRecognizer::load(&paths).unwrap();
    assert_eq!(speaker.voices(), 10);
    for voice in [0u8, 5] {
        let clip = speaker
            .speak("I created a dot called Morning Brief.", voice, 1.0)
            .unwrap();
        assert_eq!(clip.rate, 44_100);
        let heard = recognizer.transcribe(&clip).unwrap().to_lowercase();
        assert!(heard.contains("morning brief"), "voice {voice}: {heard}");
    }
}

#[test]
#[ignore]
fn hey_dot_is_caught_and_near_misses_are_not() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut wake = SherpaWake::load(&paths).unwrap();
    let cases = [
        ("Hey Dot, what needs me?", Some(WakeWord::HeyDot)),
        (
            "Hey Dad, I connected the dots on that report.",
            None::<WakeWord>,
        ),
        ("Dot, stop.", Some(WakeWord::DotStop)),
    ];
    let mut wrong = Vec::new();
    for voice in [0u8, 3, 7] {
        for (sentence, want) in cases {
            let samples = ears(&speaker.speak(sentence, voice, 1.0).unwrap());
            let got = first_wake(&mut wake, &samples);
            if got != want {
                wrong.push(format!(
                    "voice {voice}: {sentence:?} wanted {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
#[ignore]
fn silence_is_not_speech_and_speech_is() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut detector = SherpaDetector::load(&paths).unwrap();

    let silent = vec![0.0f32; EAR_RATE as usize];
    let in_silence = silent
        .chunks_exact(FRAME)
        .filter(|f| detector.is_speech(f))
        .count();
    assert_eq!(in_silence, 0);

    detector.reset();
    let samples = ears(
        &speaker
            .speak("Please read me the first message.", 0, 1.0)
            .unwrap(),
    );
    let in_speech = samples
        .chunks_exact(FRAME)
        .filter(|f| detector.is_speech(f))
        .count();
    assert!(in_speech >= 10, "only {in_speech} speech frames");
}

#[test]
#[ignore]
fn a_short_sentence_is_ready_fast() {
    let speaker = SherpaSpeaker::load(&paths(), 2).unwrap();
    speaker.speak("Warming up.", 0, 1.0).unwrap();
    let start = Instant::now();
    let clip = speaker.speak("Okay, I'm on it.", 0, 1.0).unwrap();
    let took = start.elapsed();
    assert!(!clip.samples.is_empty());
    println!("short sentence ready in {} ms", took.as_millis());
    assert!(took.as_millis() < 1500, "took {} ms", took.as_millis());
}

/// Not a pass/fail gate for the hit rate, but near-misses must never wake it. Prints the rates.
#[test]
#[ignore]
fn wake_hit_rates_over_all_voices() {
    let paths = paths();
    let speaker = SherpaSpeaker::load(&paths, 2).unwrap();
    let mut wake = SherpaWake::load(&paths).unwrap();
    let mut rates = Vec::new();
    for (sentence, want) in [
        ("Hey Dot, what needs me?", Some(WakeWord::HeyDot)),
        ("Dot, stop.", Some(WakeWord::DotStop)),
        ("Hey Dad, I connected the dots on that report.", None),
        (
            "Please stop the music before the dog gets a hot donut.",
            None,
        ),
    ] {
        let hits = (0..speaker.voices())
            .filter(|&v| {
                first_wake(&mut wake, &ears(&speaker.speak(sentence, v, 1.0).unwrap())) == want
            })
            .count();
        println!("{hits}/{} right for {sentence:?}", speaker.voices());
        rates.push((want, hits));
    }
    for (want, hits) in rates {
        if want.is_none() {
            assert_eq!(hits, usize::from(speaker.voices()), "a near miss woke it");
        }
    }
}

#[test]
#[ignore]
fn an_empty_clip_hears_nothing() {
    let recognizer = SherpaRecognizer::load(&paths()).unwrap();
    let empty = Clip {
        samples: Vec::new(),
        rate: 16_000,
    };
    assert_eq!(recognizer.transcribe(&empty).unwrap(), "");
    let short = Clip {
        samples: vec![0.0; 800],
        rate: 16_000,
    };
    assert_eq!(recognizer.transcribe(&short).unwrap(), "");
}
