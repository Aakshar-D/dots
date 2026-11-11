//! The service against a fake session factory and a local file server: no microphone, no
//! speaker, no models.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, State as AxState};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use dots_core::open_dot::MindEvent;
use dots_voice::ModelFile;
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use super::*;
use crate::error::CmdErrorKind;

// ---- the file server ------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    /// Waits for the gate before it answers.
    Gated,
    /// Sends half the body, then nothing, never ending.
    Hold,
}

struct Server {
    data: Vec<u8>,
    mode: Mode,
    gate: Notify,
    requests: AtomicUsize,
}

async fn serve(AxState(s): AxState<Arc<Server>>, AxPath(_name): AxPath<String>) -> Response {
    s.requests.fetch_add(1, Ordering::SeqCst);
    if s.mode == Mode::Gated {
        s.gate.notified().await;
    }
    let all = Bytes::copy_from_slice(&s.data);
    let builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_LENGTH, all.len());
    if s.mode == Mode::Hold {
        let half = all.slice(..all.len() / 2);
        let stream = futures_util::stream::once(async move { Ok::<_, std::io::Error>(half) })
            .chain(futures_util::stream::pending());
        return builder.body(Body::from_stream(stream)).unwrap();
    }
    builder.body(Body::from(all)).unwrap()
}

use futures_util::StreamExt;

async fn server(mode: Mode) -> (Manifest, Arc<Server>) {
    let data: Vec<u8> = (0..50_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let state = Arc::new(Server {
        data: data.clone(),
        mode,
        gate: Notify::new(),
        requests: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/{name}", get(serve))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let sha: String = Sha256::digest(&data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let manifest = Manifest {
        files: vec![ModelFile {
            name: "vad".into(),
            url: format!("{base}/silero_vad.onnx"),
            bytes: data.len() as u64,
            sha256: sha,
            unpack: false,
        }],
    };
    (manifest, state)
}

// ---- fakes ----------------------------------------------------------------------------------

#[derive(Default)]
struct FakeSession {
    calls: Mutex<Vec<String>>,
    events: Mutex<Option<UnboundedSender<SessionEvent>>>,
}

impl FakeSession {
    fn push(&self, call: impl Into<String>) {
        self.calls.lock().unwrap().push(call.into());
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn emit(&self, ev: SessionEvent) {
        if let Some(tx) = self.events.lock().unwrap().as_ref() {
            let _ = tx.send(ev);
        }
    }
}

impl SessionPort for FakeSession {
    fn open(&self) {
        self.push("open");
    }
    fn close(&self) {
        self.push("close");
    }
    fn interrupt(&self) {
        self.push("interrupt");
    }
    fn thinking(&self) {}
    fn say(&self, _turn: u64, _text: &str, _then: dots_voice::After) {}
    fn say_delta(&self, _turn: u64, _delta: &str) {}
    fn end_of_reply(&self, _turn: u64, _then: dots_voice::After) {}
    fn filler(&self, _turn: u64, _text: &str) {}
    fn answered(&self, _text: &str) -> Option<u64> {
        None
    }
    fn state(&self) -> State {
        State::Idle
    }
}

impl SessionControl for FakeSession {
    fn settings(&self, s: SessionSettings) {
        self.push(format!("settings wake={}", s.wake_phrase));
    }
    fn set_voice(&self, voice: u8, speed: f32) {
        self.push(format!("voice {voice} {speed}"));
    }
    fn notice(&self, text: &str) {
        self.push(format!("notice {text}"));
    }
    fn shutdown(&self) {
        self.push("shutdown");
        // The worker ends: its end of the event channel goes.
        self.events.lock().unwrap().take();
    }
}

#[derive(Default)]
struct FakeFactory {
    specs: Mutex<Vec<BuildSpec>>,
    sessions: Mutex<Vec<Arc<FakeSession>>>,
    /// Builds that fail before one succeeds.
    fail: AtomicUsize,
}

impl FakeFactory {
    fn builds(&self) -> usize {
        self.specs.lock().unwrap().len()
    }
    fn session(&self, n: usize) -> Arc<FakeSession> {
        self.sessions.lock().unwrap()[n].clone()
    }
}

impl SessionFactory for Arc<FakeFactory> {
    fn build(&self, spec: &BuildSpec) -> Result<Built, String> {
        self.specs.lock().unwrap().push(spec.clone());
        if self
            .fail
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(NO_SPEAKER.into());
        }
        let (tx, events) = unbounded_channel();
        let session = Arc::new(FakeSession::default());
        *session.events.lock().unwrap() = Some(tx);
        session.emit(SessionEvent::State {
            state: State::Idle,
            mic_open: false,
        });
        self.sessions.lock().unwrap().push(session.clone());
        Ok(Built { session, events })
    }
    fn microphones(&self) -> Vec<VoiceDevice> {
        vec![VoiceDevice {
            id: "mic-1".into(),
            name: "Desk mic".into(),
            default: true,
        }]
    }
    fn speakers(&self) -> Vec<VoiceDevice> {
        vec![]
    }
}

struct QuietMind;

#[async_trait]
impl MindPort for QuietMind {
    async fn hear(&self, _: &str, _: &View, _: &UnboundedSender<MindEvent>) {}
    async fn pick(&self, _: usize, _: &View, _: &UnboundedSender<MindEvent>) {}
    fn lapse(&self, _: &UnboundedSender<MindEvent>) {}
    fn cancel(&self) {}
    fn reset(&self) -> bool {
        false
    }
    fn pending(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct FakeHost {
    settings: Mutex<Settings>,
}

#[async_trait]
impl Host for FakeHost {
    async fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }
    async fn save(&self, settings: Settings) -> Result<Settings, CmdError> {
        *self.settings.lock().unwrap() = settings.clone();
        Ok(settings)
    }
    fn mind(&self) -> Arc<dyn MindPort> {
        Arc::new(QuietMind)
    }
}

// ---- the rig --------------------------------------------------------------------------------

struct Rig {
    _dir: tempfile::TempDir,
    root: PathBuf,
    svc: Arc<Service>,
    host: Arc<FakeHost>,
    factory: Arc<FakeFactory>,
    server: Arc<Server>,
    ui: UnboundedReceiver<VoiceEvent>,
}

async fn rig(mode: Mode) -> Rig {
    let (manifest, server) = server(mode).await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("voice").join("models");
    let host = Arc::new(FakeHost::default());
    let factory = Arc::new(FakeFactory::default());
    let (tx, ui) = unbounded_channel();
    let svc = Service::new(
        host.clone(),
        Arc::new(factory.clone()),
        root.clone(),
        manifest,
        tx,
    );
    Rig {
        _dir: dir,
        root,
        svc,
        host,
        factory,
        server,
        ui,
    }
}

impl Rig {
    fn ui(&mut self) -> Vec<VoiceEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.ui.try_recv() {
            out.push(ev);
        }
        out
    }
    fn stored(&self) -> Settings {
        self.host.settings.lock().unwrap().clone()
    }
    fn part(&self) -> PathBuf {
        self.root.join("vad.part")
    }
}

/// Waits up to five seconds for `f`.
async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < end, "never: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn on() -> Settings {
    Settings {
        voice_enabled: true,
        ..Settings::default()
    }
}

// ---- tests ----------------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_builds_nothing() {
    let mut r = rig(Mode::Normal).await;
    r.svc.apply(&Settings::default()).await;
    r.svc.open();
    r.svc.close();
    r.svc.stop();
    r.svc.pick(0);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.factory.builds(), 0, "no session");
    assert_eq!(r.server.requests.load(Ordering::SeqCst), 0, "no download");
    assert!(!r.root.exists(), "nothing on disk");
    let status = r.svc.status().await;
    assert!(!status.enabled && !status.ready && !status.downloading);
    assert_eq!(status.state, State::Off);
    assert!(r.ui().contains(&VoiceEvent::Trouble {
        message: OFF.into()
    }));
    assert!(matches!(
        r.svc.preview(1, 1.0).await,
        Err(CmdError {
            kind: CmdErrorKind::Invalid,
            ..
        })
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enable_downloads_then_builds_and_greets() {
    let mut r = rig(Mode::Normal).await;
    let status = r.svc.enable(true).await.unwrap();
    assert!(
        status.enabled && status.installed && status.ready,
        "{status:?}"
    );
    assert!(!status.downloading);
    assert!(r.stored().voice_enabled, "the setting is saved");
    assert_eq!(r.factory.builds(), 1);
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&format!("notice {GREETING}")));
    let events = r.ui();
    let last_download = events.iter().rev().find_map(|e| match e {
        VoiceEvent::Download {
            done_bytes,
            total_bytes,
        } => Some((*done_bytes, *total_bytes)),
        _ => None,
    });
    assert_eq!(last_download, Some((50_000, 50_000)));
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::Status { status } if status.downloading
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        VoiceEvent::Status { status } if status.ready
    )));
    // Turned on again: no second greeting, no second build.
    r.svc.enable(true).await.unwrap();
    assert_eq!(r.factory.builds(), 1);
    // Opening reaches the session.
    r.svc.open();
    assert!(r.factory.session(0).calls().contains(&"open".to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_enables_one_download() {
    let r = rig(Mode::Gated).await;
    let first = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.enable(true).await })
    };
    let svc = r.svc.clone();
    until("downloading", || svc.status_now().downloading).await;
    let second = r.svc.enable(true).await.unwrap();
    assert!(second.downloading && !second.ready);
    r.svc.open();
    r.server.gate.notify_one();
    let first = first.await.unwrap().unwrap();
    assert!(first.ready, "{first:?}");
    assert_eq!(r.server.requests.load(Ordering::SeqCst), 1);
    assert_eq!(r.factory.builds(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_leaves_a_resumable_part_and_status_off() {
    let r = rig(Mode::Hold).await;
    let enabling = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.enable(true).await })
    };
    let part = r.part();
    until("half the file", || {
        std::fs::metadata(&part).is_ok_and(|m| m.len() >= 25_000)
    })
    .await;
    r.svc.download_cancel();
    let err = enabling.await.unwrap().unwrap_err();
    assert_eq!(err.kind, CmdErrorKind::Cancelled);
    assert_eq!(err.message, download::CANCELLED);
    let status = r.svc.status().await;
    assert!(!status.enabled && !status.downloading && !status.ready);
    assert_eq!(status.trouble, None);
    assert!(!r.stored().voice_enabled, "stored off");
    assert_eq!(std::fs::metadata(r.part()).unwrap().len(), 25_000);
    assert_eq!(r.factory.builds(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_turns_off_and_deletes() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    let status = r.svc.remove().await.unwrap();
    assert!(
        !status.enabled && !status.installed && !status.ready,
        "{status:?}"
    );
    assert_eq!(status.state, State::Off);
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&"shutdown".to_string()));
    assert!(!r.root.exists(), "the files are gone");
    assert!(!r.stored().voice_enabled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_change_rebuilds() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    let usb = Settings {
        voice_mic: Some("usb".into()),
        ..on()
    };
    r.svc.apply(&usb).await;
    assert_eq!(r.factory.builds(), 2);
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&"shutdown".to_string()));
    assert_eq!(
        r.factory.specs.lock().unwrap()[1].mic,
        Some("usb".to_string())
    );
    assert!(r.svc.status().await.ready);
    let speaker = Settings {
        voice_speaker: Some("headset".into()),
        ..usb
    };
    r.svc.apply(&speaker).await;
    assert_eq!(r.factory.builds(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn voice_change_does_not_rebuild() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    r.svc
        .apply(&Settings {
            voice_id: 3,
            voice_speed: 1.2,
            voice_wake_phrase: true,
            ..on()
        })
        .await;
    assert_eq!(r.factory.builds(), 1);
    let calls = r.factory.session(0).calls();
    assert!(calls.contains(&"voice 3 1.2".to_string()), "{calls:?}");
    assert!(
        calls.contains(&"settings wake=true".to_string()),
        "{calls:?}"
    );
    // Turned off: the session goes and the microphone with it.
    r.svc.apply(&Settings::default()).await;
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&"shutdown".to_string()));
    let status = r.svc.status().await;
    assert!(!status.ready && !status.mic_open);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn build_failure_is_a_sentence_and_retried_on_open() {
    let r = rig(Mode::Normal).await;
    r.factory.fail.store(1, Ordering::SeqCst);
    let status = r.svc.enable(true).await.unwrap();
    assert!(status.enabled && status.installed && !status.ready);
    assert_eq!(status.trouble.as_deref(), Some(NO_SPEAKER));
    r.svc.open();
    let svc = r.svc.clone();
    until("ready", || svc.status_now().ready).await;
    assert_eq!(r.factory.builds(), 2);
    assert_eq!(r.svc.status().await.trouble, None);
    let f = r.factory.clone();
    until("listening", || {
        f.session(0).calls().contains(&"open".to_string())
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_releases() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    let svc = r.svc.clone();
    let began = Instant::now();
    tokio::task::spawn_blocking(move || svc.shutdown())
        .await
        .unwrap();
    // The event channel closed well within the wait: the session let go of its devices.
    assert!(
        began.elapsed() < Duration::from_millis(450),
        "{:?}",
        began.elapsed()
    );
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&"shutdown".to_string()));
    let status = r.svc.status().await;
    assert!(!status.ready);
    assert_eq!(status.state, State::Off);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_session_that_ends_by_itself_is_built_again_on_open() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    // The worker dies: its channel closes.
    r.factory.session(0).events.lock().unwrap().take();
    let svc = r.svc.clone();
    until("not ready", || !svc.status_now().ready).await;
    r.svc.open();
    until("ready again", || svc.status_now().ready).await;
    assert_eq!(r.factory.builds(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_speaks_then_restores_the_saved_voice() {
    let r = rig(Mode::Normal).await;
    r.svc
        .apply(&Settings {
            voice_id: 2,
            voice_speed: 1.0,
            ..Settings::default()
        })
        .await;
    *r.host.settings.lock().unwrap() = Settings {
        voice_id: 2,
        ..Settings::default()
    };
    r.svc.enable(true).await.unwrap();
    let session = r.factory.session(0);
    until("idle", || r.svc.status_now().state == State::Idle).await;
    r.svc.preview(5, 1.3).await.unwrap();
    session.emit(SessionEvent::State {
        state: State::Speaking,
        mic_open: false,
    });
    session.emit(SessionEvent::State {
        state: State::Idle,
        mic_open: false,
    });
    until("restored", || {
        session.calls().contains(&"voice 2 1".to_string())
    })
    .await;
    let calls = session.calls();
    let spoke = calls.iter().position(|c| c == "voice 5 1.3").unwrap();
    assert_eq!(calls[spoke + 1], format!("notice {GREETING}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn devices_are_listed() {
    let r = rig(Mode::Normal).await;
    let d = r.svc.devices();
    assert_eq!(d.microphones.len(), 1);
    assert_eq!(d.microphones[0].name, "Desk mic");
    assert!(d.speakers.is_empty());
}
