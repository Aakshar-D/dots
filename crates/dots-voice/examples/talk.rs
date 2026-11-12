//! Hand check for the microphone and speaker: it says it is listening, then repeats back each
//! sentence it hears. It prints state words only, never what was heard. Ctrl+C ends it.
//!
//! `DOTS_VOICE_MODELS=<models folder> cargo run -p dots-voice --example talk`

use dots_voice::{
    CpalMic, CpalSink, EarEvent, EarSettings, Ears, Mic, MicFrame, ModelPaths, Recognizer,
    SherpaDetector, SherpaRecognizer, SherpaSpeaker, Sink, Speaker, Voice,
};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("DOTS_VOICE_MODELS")
        .ok_or("set DOTS_VOICE_MODELS to the models folder")?;
    let paths = ModelPaths { root: root.into() };

    println!("loading models");
    let speaker: Arc<dyn Speaker> = Arc::new(SherpaSpeaker::load(&paths, 2)?);
    let recognizer: Arc<dyn Recognizer> = Arc::new(SherpaRecognizer::load(&paths)?);
    let detector = SherpaDetector::load(&paths)?;

    let mut mic = CpalMic::open(None)?;
    let sink = Arc::new(CpalSink::open(None)?);
    let (events, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    // Nothing here needs the voice events; read them so the channel does not fill up.
    std::thread::spawn(move || while events_rx.blocking_recv().is_some() {});
    let voice = Voice::new(speaker, sink.clone() as Arc<dyn Sink>, events);
    let mut ears = Ears::new(Box::new(detector), recognizer, EarSettings::default());

    let frames = mic.frames();
    voice.say("I'm listening.");
    println!("speaking");

    let mut deaf = true;
    while let Ok(frame) = frames.recv() {
        let frame = match frame {
            MicFrame::Audio(frame) => frame,
            MicFrame::Lost(_) => {
                println!("the microphone was lost");
                break;
            }
        };
        // Do not listen while speaking, or it would hear itself.
        if voice.speaking() || !sink.idle() {
            deaf = true;
            continue;
        }
        if deaf {
            deaf = false;
            ears.reset();
            println!("listening");
            continue;
        }
        for event in ears.push(&frame) {
            match event {
                EarEvent::Started => println!("hearing speech"),
                EarEvent::Final(text) => {
                    println!("heard a sentence");
                    voice.say(&format!("You said: {text}"));
                    println!("speaking");
                }
                EarEvent::Partial(_) | EarEvent::Dropped => {}
            }
        }
    }
    Ok(())
}
