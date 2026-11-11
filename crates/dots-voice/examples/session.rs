//! Hand check for the whole `Session` on the real microphone, speaker and models. It prints
//! every session event (levels only with `--levels`) and says back each sentence it hears,
//! then listens for a follow-up.
//!
//! `DOTS_VOICE_MODELS=<models folder> cargo run -p dots-voice --example session [-- --levels]`
//!
//! Type a line and press Enter:
//!
//! - nothing: `open` (listen now)
//! - `c`: `close`, `i`: `interrupt`
//! - `w`: wake phrase on or off ("Hey Dot", "Dot, stop")
//! - `t`: "Interrupt by just talking" on or off
//! - `a <words>`: `answered(<words>)`, as if an option were clicked
//! - `ask`: the next reply asks a question, so the short-answer ears are used
//! - `q`: `shutdown`, then wait for the worker to end
//!
//! What is heard is printed on this terminal only; nothing is written anywhere.

use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use dots_voice::{
    After, CpalMic, CpalSink, EarSettings, Ears, Mic, ModelPaths, Parts, Recognizer, Session,
    SessionEvent, SessionSettings, SherpaDetector, SherpaRecognizer, SherpaSpeaker, SherpaWake,
    Sink, Speaker, Voice, WakeSpotter,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::var_os("DOTS_VOICE_MODELS")
        .ok_or("set DOTS_VOICE_MODELS to the models folder")?;
    let paths = ModelPaths { root: root.into() };
    let levels = std::env::args().any(|a| a == "--levels");

    println!("loading models");
    let speaker: Arc<dyn Speaker> = Arc::new(SherpaSpeaker::load(&paths, 2)?);
    let recognizer: Arc<dyn Recognizer> = Arc::new(SherpaRecognizer::load(&paths)?);
    let ears = Ears::new(
        Box::new(SherpaDetector::load(&paths)?),
        recognizer.clone(),
        EarSettings::default(),
    );
    let short_ears = Ears::new(
        Box::new(SherpaDetector::load(&paths)?),
        recognizer,
        EarSettings::short_answer(),
    );
    let wake: Option<Box<dyn WakeSpotter>> = match SherpaWake::load(&paths) {
        Ok(w) => Some(Box::new(w)),
        Err(e) => {
            println!("no wake phrase: {e}");
            None
        }
    };

    let sink: Arc<dyn Sink> = Arc::new(CpalSink::open(None)?);
    let (voice_tx, voice_events) = tokio::sync::mpsc::unbounded_channel();
    let voice = Voice::new(speaker, sink.clone(), voice_tx);
    let parts = Parts {
        ears,
        short_ears,
        voice,
        voice_events,
        sink,
        open_sink: Box::new(|| CpalSink::open(None).map(|s| Arc::new(s) as Arc<dyn Sink>)),
        wake,
        open_mic: Box::new(|| CpalMic::open(None).map(|m| Box::new(m) as Box<dyn Mic>)),
    };

    let mut settings = SessionSettings::default();
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let session = Session::start(parts, settings, events_tx);

    // Events: print each one, and answer every finished sentence by saying it back.
    let ask_next = Arc::new(AtomicBool::new(false));
    let printer = {
        let session = session.clone();
        let ask_next = ask_next.clone();
        std::thread::spawn(move || {
            while let Some(event) = events.blocking_recv() {
                if matches!(event, SessionEvent::Level { .. }) && !levels {
                    continue;
                }
                println!("{event:?}");
                if let SessionEvent::Heard { text, turn } = event {
                    session.thinking();
                    if ask_next.swap(false, Ordering::SeqCst) {
                        session.say(
                            turn,
                            &format!("You said: {text}. Is that right?"),
                            After::Ask,
                        );
                    } else {
                        session.say(turn, &format!("You said: {text}"), After::FollowUp);
                    }
                }
            }
            println!("the session has ended");
        })
    };

    println!("press Enter to talk; q to quit");
    session.open();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let line = line.trim();
        match line {
            "" => session.open(),
            "c" => session.close(),
            "i" => session.interrupt(),
            "w" => {
                settings.wake_phrase = !settings.wake_phrase;
                println!("wake phrase {}", on_off(settings.wake_phrase));
                session.settings(settings);
            }
            "t" => {
                settings.interrupt_by_talking = !settings.interrupt_by_talking;
                println!(
                    "interrupt by talking {}",
                    on_off(settings.interrupt_by_talking)
                );
                session.settings(settings);
            }
            "ask" => {
                ask_next.store(true, Ordering::SeqCst);
                println!("the next reply asks a question");
            }
            "q" => break,
            other => match other.strip_prefix("a ") {
                Some(words) => println!("answered as turn {:?}", session.answered(words)),
                None => println!("unknown command {other:?}"),
            },
        }
    }
    session.shutdown();
    drop(session);
    let _ = printer.join(); // ends when the worker drops its end of the event channel
    Ok(())
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}
