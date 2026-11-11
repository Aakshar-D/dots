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
    mode: Mutex<Mode>,
    gate: Notify,
    requests: AtomicUsize,
    /// The `Range` header of each request.
    ranges: Mutex<Vec<Option<String>>>,
}

impl Server {
    fn mode(&self) -> Mode {
        *self.mode.lock().unwrap()
    }
}

async fn serve(
    AxState(s): AxState<Arc<Server>>,
    AxPath(_name): AxPath<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    s.requests.fetch_add(1, Ordering::SeqCst);
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    s.ranges.lock().unwrap().push(range.clone());
    if s.mode() == Mode::Gated {
        s.gate.notified().await;
    }
    let start = range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(0);
    let total = s.data.len();
    let all = Bytes::copy_from_slice(&s.data[start..]);
    let mut builder = Response::builder().header(header::CONTENT_LENGTH, all.len());
    builder = if start > 0 {
        builder.status(StatusCode::PARTIAL_CONTENT).header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{}/{total}", total - 1),
        )
    } else {
        builder.status(StatusCode::OK)
    };
    if s.mode() == Mode::Hold {
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
        mode: Mutex::new(mode),
        gate: Notify::new(),
        requests: AtomicUsize::new(0),
        ranges: Mutex::default(),
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
    fn notice(&self, text: &str) {
        self.push(format!("notice {text}"));
    }
}

impl SessionControl for FakeSession {
    fn settings(&self, s: SessionSettings) {
        self.push(format!("settings wake={}", s.wake_phrase));
    }
    fn set_voice(&self, voice: u8, speed: f32) {
        self.push(format!("voice {voice} {speed}"));
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
    /// While true, a build waits (the models are loading).
    held: Mutex<bool>,
    let_go: std::sync::Condvar,
    /// Builds that found they were no longer wanted, and started no session.
    unwanted: AtomicUsize,
}

impl FakeFactory {
    fn builds(&self) -> usize {
        self.specs.lock().unwrap().len()
    }
    fn session(&self, n: usize) -> Arc<FakeSession> {
        self.sessions.lock().unwrap()[n].clone()
    }
    fn started(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }
    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.let_go.notify_all();
    }
}

impl SessionFactory for Arc<FakeFactory> {
    fn build(&self, spec: &BuildSpec, wanted: &dyn Fn() -> bool) -> Result<Option<Built>, String> {
        self.specs.lock().unwrap().push(spec.clone());
        let held = self.held.lock().unwrap();
        drop(self.let_go.wait_while(held, |h| *h).unwrap());
        if self
            .fail
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(NO_SPEAKER.into());
        }
        // As the real factory does just before the session starts.
        if !wanted() {
            self.unwanted.fetch_add(1, Ordering::SeqCst);
            return Ok(None);
        }
        let (tx, events) = unbounded_channel();
        let session = Arc::new(FakeSession::default());
        *session.events.lock().unwrap() = Some(tx);
        session.emit(SessionEvent::State {
            state: State::Idle,
            mic_open: false,
        });
        self.sessions.lock().unwrap().push(session.clone());
        Ok(Some(Built { session, events }))
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

/// Says nothing; records the view each turn saw.
#[derive(Default)]
struct QuietMind {
    views: Mutex<Vec<View>>,
}

#[async_trait]
impl MindPort for QuietMind {
    async fn hear(&self, _: &str, view: &View, _: &UnboundedSender<MindEvent>) {
        self.views.lock().unwrap().push(view.clone());
    }
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
    mind: Arc<QuietMind>,
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
        self.mind.clone()
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
    // Turned on again, it picks up where it left off rather than starting over.
    *r.server.mode.lock().unwrap() = Mode::Normal;
    let status = r.svc.enable(true).await.unwrap();
    assert!(status.ready && status.installed, "{status:?}");
    assert_eq!(
        r.server.ranges.lock().unwrap().last().cloned().flatten(),
        Some("bytes=25000-".to_string())
    );
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

// ---- the hotkey and the listener ------------------------------------------------------------

/// Records what was registered; refuses the shortcuts in `taken`. As the plugin one does, a
/// registration takes the place of the shortcut before only when it succeeds.
#[derive(Default)]
struct FakeHotkey {
    calls: Mutex<Vec<String>>,
    taken: Mutex<Vec<String>>,
    /// The shortcut that works now.
    current: Mutex<Option<String>>,
}

impl FakeHotkey {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn current(&self) -> Option<String> {
        self.current.lock().unwrap().clone()
    }
}

impl HotkeyPort for FakeHotkey {
    fn register(&self, shortcut: &str) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("register {shortcut}"));
        if self.taken.lock().unwrap().iter().any(|t| t == shortcut) {
            return Err(super::super::hotkey::TAKEN.to_string());
        }
        *self.current.lock().unwrap() = Some(shortcut.to_string());
        Ok(())
    }
    fn unregister(&self) {
        self.calls.lock().unwrap().push("unregister".into());
        *self.current.lock().unwrap() = None;
    }
}

#[derive(Default)]
struct FakeListener {
    mic: Mutex<Vec<bool>>,
    ready: Mutex<Vec<bool>>,
}

impl Listener for FakeListener {
    fn mic_open(&self, open: bool) {
        self.mic.lock().unwrap().push(open);
    }
    fn ready(&self, ready: bool) {
        self.ready.lock().unwrap().push(ready);
    }
}

fn hotkey(r: &Rig) -> Arc<FakeHotkey> {
    let fake = Arc::new(FakeHotkey::default());
    r.svc.set_hotkey_port(fake.clone());
    fake
}

/// Saves `settings` the way `update_settings` does, then applies them.
async fn save(r: &Rig, settings: Settings) {
    r.host.save(settings.clone()).await.unwrap();
    r.svc.apply(&settings).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hotkey_registered_only_when_enabled() {
    let r = rig(Mode::Normal).await;
    let keys = hotkey(&r);
    // Off at start: nothing registered.
    r.svc.apply(&Settings::default()).await;
    assert!(keys.calls().is_empty(), "{:?}", keys.calls());
    // On, with no shortcut: still nothing.
    save(
        &r,
        Settings {
            voice_hotkey: String::new(),
            ..on()
        },
    )
    .await;
    assert!(keys.calls().is_empty(), "{:?}", keys.calls());
    // Turned on (downloads, builds): the default shortcut is registered once.
    *r.host.settings.lock().unwrap() = Settings::default();
    r.svc.enable(true).await.unwrap();
    assert_eq!(keys.calls(), ["register CmdOrCtrl+Shift+Space"]);
    // Applying the same settings again changes nothing.
    r.svc.apply(&r.stored()).await;
    assert_eq!(keys.calls(), ["register CmdOrCtrl+Shift+Space"]);
    assert_eq!(r.svc.status().await.hotkey_error, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_voice_off_unregisters() {
    let r = rig(Mode::Normal).await;
    let keys = hotkey(&r);
    r.svc.enable(true).await.unwrap();
    r.svc.enable(false).await.unwrap();
    assert_eq!(
        keys.calls(),
        ["register CmdOrCtrl+Shift+Space", "unregister"]
    );
    // Removing the files after turning on again unregisters too.
    r.svc.enable(true).await.unwrap();
    r.svc.remove().await.unwrap();
    assert_eq!(
        keys.calls(),
        [
            "register CmdOrCtrl+Shift+Space",
            "unregister",
            "register CmdOrCtrl+Shift+Space",
            "unregister"
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changing_the_hotkey_reregisters() {
    let r = rig(Mode::Normal).await;
    let keys = hotkey(&r);
    r.svc.enable(true).await.unwrap();
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F9".into(),
            ..on()
        },
    )
    .await;
    // The new one is registered in place of the old: no unregister first.
    assert_eq!(
        keys.calls(),
        ["register CmdOrCtrl+Shift+Space", "register Alt+F9"]
    );
    assert_eq!(keys.current().as_deref(), Some("Alt+F9"));
    // Cleared: unregistered, nothing in its place.
    save(
        &r,
        Settings {
            voice_hotkey: String::new(),
            ..on()
        },
    )
    .await;
    assert_eq!(keys.calls().last().map(String::as_str), Some("unregister"));
    assert_eq!(keys.calls().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_taken_hotkey_keeps_the_old_one_working() {
    let r = rig(Mode::Normal).await;
    let keys = hotkey(&r);
    keys.taken.lock().unwrap().push("Alt+F9".into());
    r.svc.enable(true).await.unwrap();
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F9".into(),
            ..on()
        },
    )
    .await;
    assert_eq!(
        keys.current().as_deref(),
        Some("CmdOrCtrl+Shift+Space"),
        "{:?}",
        keys.calls()
    );
    assert!(!keys.calls().contains(&"unregister".to_string()));
    assert_eq!(
        r.svc.status().await.hotkey_error.as_deref(),
        Some(super::super::hotkey::TAKEN)
    );
    // Freed later, it is tried again on the next save.
    keys.taken.lock().unwrap().clear();
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F9".into(),
            voice_id: 1,
            ..on()
        },
    )
    .await;
    assert_eq!(keys.current().as_deref(), Some("Alt+F9"));
    assert_eq!(r.svc.status().await.hotkey_error, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_taken_hotkey_sets_the_error_and_saves_anyway() {
    let mut r = rig(Mode::Normal).await;
    let keys = hotkey(&r);
    keys.taken.lock().unwrap().push("Alt+F9".into());
    r.svc.enable(true).await.unwrap();
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F9".into(),
            ..on()
        },
    )
    .await;
    assert_eq!(r.stored().voice_hotkey, "Alt+F9", "the setting is saved");
    let status = r.svc.status().await;
    assert_eq!(
        status.hotkey_error.as_deref(),
        Some(super::super::hotkey::TAKEN)
    );
    // Nothing else changed: voice is still on and ready.
    assert!(status.enabled && status.ready, "{status:?}");
    assert!(r.ui().iter().any(|e| matches!(
        e,
        VoiceEvent::Status { status } if status.hotkey_error.is_some()
    )));
    // A free one clears the error.
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F10".into(),
            ..on()
        },
    )
    .await;
    assert_eq!(r.svc.status().await.hotkey_error, None);
    assert_eq!(
        keys.calls().last().map(String::as_str),
        Some("register Alt+F10")
    );
    // Turning off clears it too.
    keys.taken.lock().unwrap().push("Alt+F11".into());
    save(
        &r,
        Settings {
            voice_hotkey: "Alt+F11".into(),
            ..on()
        },
    )
    .await;
    assert!(r.svc.status().await.hotkey_error.is_some());
    r.svc.enable(false).await.unwrap();
    assert_eq!(r.svc.status().await.hotkey_error, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mic_open_changes_reach_the_listener_once_each() {
    let r = rig(Mode::Normal).await;
    let heard = Arc::new(FakeListener::default());
    r.svc.set_listener(heard.clone());
    r.svc.enable(true).await.unwrap();
    assert_eq!(*heard.ready.lock().unwrap(), [true]);
    assert!(
        heard.mic.lock().unwrap().is_empty(),
        "the mic starts closed"
    );
    let session = r.factory.session(0);
    let steps = [
        (State::Listening, true),
        (State::Thinking, false),
        (State::Speaking, false),
        (State::Listening, true),
        (State::Asking, true),
        (State::Idle, false),
    ];
    for (state, mic_open) in steps {
        session.emit(SessionEvent::State { state, mic_open });
    }
    let h = heard.clone();
    until("four changes", || h.mic.lock().unwrap().len() >= 4).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(*heard.mic.lock().unwrap(), [true, false, true, false]);
    // Opened, then turned off: closed and not ready.
    session.emit(SessionEvent::State {
        state: State::Listening,
        mic_open: true,
    });
    let h = heard.clone();
    until("open again", || h.mic.lock().unwrap().len() == 5).await;
    r.svc.enable(false).await.unwrap();
    assert_eq!(
        *heard.mic.lock().unwrap(),
        [true, false, true, false, true, false]
    );
    assert_eq!(*heard.ready.lock().unwrap(), [true, false]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn devices_are_listed() {
    let r = rig(Mode::Normal).await;
    let d = r.svc.devices();
    assert_eq!(d.microphones.len(), 1);
    assert_eq!(d.microphones[0].name, "Desk mic");
    assert!(d.speakers.is_empty());
}

// ---- the service ends where the settings ask ------------------------------------------------

/// Downloads and builds once, then turns voice off: the files are in place, nothing runs.
async fn installed_and_off(r: &Rig) {
    r.svc.enable(true).await.unwrap();
    r.svc.enable(false).await.unwrap();
    assert_eq!(r.factory.builds(), 1);
}

/// Turns voice on in the background, and waits until the build is loading (held).
async fn enabling_held(r: &Rig) -> tokio::task::JoinHandle<Result<VoiceStatus, CmdError>> {
    r.factory.hold();
    let enabling = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.enable(true).await })
    };
    let f = r.factory.clone();
    until("the build began", || f.builds() == 2).await;
    enabling
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_then_on_during_a_build_ends_with_a_session() {
    let r = rig(Mode::Normal).await;
    installed_and_off(&r).await;
    let enabling = enabling_held(&r).await;
    r.svc.enable(false).await.unwrap();
    r.svc.enable(true).await.unwrap();
    r.factory.release();
    enabling.await.unwrap().unwrap();
    let svc = r.svc.clone();
    until("ready", || svc.status_now().ready).await;
    let status = r.svc.status().await;
    assert!(status.enabled && status.ready, "{status:?}");
    // The dropped build started no session; the one after it did.
    assert_eq!(r.factory.unwanted.load(Ordering::SeqCst), 1);
    assert_eq!(r.factory.started(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn device_change_during_a_build_ends_on_the_new_device() {
    let r = rig(Mode::Normal).await;
    installed_and_off(&r).await;
    let enabling = enabling_held(&r).await;
    let usb = Settings {
        voice_mic: Some("usb".into()),
        ..on()
    };
    save(&r, usb).await;
    r.factory.release();
    enabling.await.unwrap().unwrap();
    let svc = r.svc.clone();
    let f = r.factory.clone();
    until("ready on usb", || {
        svc.status_now().ready
            && f.specs.lock().unwrap().last().unwrap().mic.as_deref() == Some("usb")
    })
    .await;
    // The old device was never opened: that build found it was no longer wanted.
    assert_eq!(r.factory.unwanted.load(Ordering::SeqCst), 1);
    assert_eq!(r.factory.started(), 2);
    let live_mic = r.svc.lock().live.as_ref().map(|l| l.spec.mic.clone());
    assert_eq!(live_mic, Some(Some("usb".to_string())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn off_during_a_build_leaves_nothing_running() {
    let r = rig(Mode::Normal).await;
    installed_and_off(&r).await;
    let enabling = enabling_held(&r).await;
    r.svc.enable(false).await.unwrap();
    r.factory.release();
    enabling.await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let status = r.svc.status().await;
    assert!(
        !status.enabled && !status.ready && !status.mic_open,
        "{status:?}"
    );
    // No session was started for it, so no microphone was opened.
    assert_eq!(r.factory.started(), 1);
    assert_eq!(r.factory.unwanted.load(Ordering::SeqCst), 1);
    assert_eq!(r.factory.builds(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_build_that_fails_after_off_sets_no_trouble() {
    let r = rig(Mode::Normal).await;
    installed_and_off(&r).await;
    r.factory.fail.store(1, Ordering::SeqCst);
    let enabling = enabling_held(&r).await;
    r.svc.enable(false).await.unwrap();
    r.factory.release();
    enabling.await.unwrap().unwrap();
    let status = r.svc.status().await;
    assert!(!status.enabled && !status.ready, "{status:?}");
    assert_eq!(status.trouble, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_during_a_build_waits_for_it() {
    let r = rig(Mode::Normal).await;
    installed_and_off(&r).await;
    let enabling = enabling_held(&r).await;
    let removing = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.remove().await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!removing.is_finished(), "removed while the models load");
    assert!(r.root.exists());
    r.factory.release();
    let status = removing.await.unwrap().unwrap();
    enabling.await.unwrap().unwrap();
    assert!(
        !status.enabled && !status.installed && !status.ready,
        "{status:?}"
    );
    assert!(!r.root.exists());
    assert_eq!(r.factory.started(), 1, "no session for the dropped build");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quitting_mid_download_keeps_voice_on() {
    let mut r = rig(Mode::Hold).await;
    let enabling = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.enable(true).await })
    };
    let part = r.part();
    until("half the file", || {
        std::fs::metadata(&part).is_ok_and(|m| m.len() >= 25_000)
    })
    .await;
    r.ui();
    let svc = r.svc.clone();
    tokio::task::spawn_blocking(move || svc.shutdown())
        .await
        .unwrap();
    // The download ends (cancelled) and its end is handled.
    let _ = enabling.await.unwrap();
    assert!(r.stored().voice_enabled, "still on for the next start");
    assert_eq!(std::fs::metadata(r.part()).unwrap().len(), 25_000);
    assert!(
        !r.ui()
            .iter()
            .any(|e| matches!(e, VoiceEvent::Status { .. })),
        "nothing is told while quitting"
    );
    assert_eq!(r.factory.builds(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_resumes_an_unfinished_download() {
    let r = rig(Mode::Normal).await;
    // A download that ended with Dots: half the file, voice stored on.
    std::fs::create_dir_all(&r.root).unwrap();
    std::fs::write(r.part(), &r.server.data[..25_000]).unwrap();
    *r.host.settings.lock().unwrap() = on();
    r.svc.start(&on()).await;
    let svc = r.svc.clone();
    until("ready", || svc.status_now().ready).await;
    let status = r.svc.status().await;
    assert!(status.enabled && status.installed, "{status:?}");
    assert_eq!(
        *r.server.ranges.lock().unwrap(),
        [Some("bytes=25000-".to_string())]
    );
    // Greets once, as after any install.
    assert!(r
        .factory
        .session(0)
        .calls()
        .contains(&format!("notice {GREETING}")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_with_voice_off_downloads_nothing() {
    let r = rig(Mode::Normal).await;
    r.svc.start(&Settings::default()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.server.requests.load(Ordering::SeqCst), 0);
    assert_eq!(r.factory.builds(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn apply_on_with_files_missing_downloads_nothing() {
    let r = rig(Mode::Normal).await;
    save(&r, on()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let status = r.svc.status().await;
    assert!(
        status.enabled && !status.installed && !status.downloading && !status.ready,
        "{status:?}"
    );
    assert_eq!(r.server.requests.load(Ordering::SeqCst), 0);
    assert_eq!(r.factory.builds(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remove_during_a_download_cancels_it_and_deletes() {
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
    let status = r.svc.remove().await.unwrap();
    assert!(
        !status.enabled && !status.installed && !status.downloading,
        "{status:?}"
    );
    assert!(!r.root.exists(), "the part is gone too");
    assert!(!r.stored().voice_enabled);
    let err = enabling.await.unwrap().unwrap_err();
    assert_eq!(err.kind, CmdErrorKind::Cancelled);
    assert_eq!(r.factory.builds(), 0);
}

// ---- small corrections ----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saving_settings_never_changes_voice_enabled() {
    // Off: a draft that says on does not turn it on.
    let r = rig(Mode::Hold).await;
    let draft = r.svc.settings_to_save(on()).await;
    assert!(!draft.voice_enabled);
    save(&r, draft).await;
    assert!(!r.svc.status().await.enabled);
    assert_eq!(r.server.requests.load(Ordering::SeqCst), 0);
    // Turning on (still downloading): a stale draft that says off neither turns it off nor
    // stops the download.
    let enabling = {
        let svc = r.svc.clone();
        tokio::spawn(async move { svc.enable(true).await })
    };
    let part = r.part();
    until("half the file", || {
        std::fs::metadata(&part).is_ok_and(|m| m.len() >= 25_000)
    })
    .await;
    let stale = Settings {
        voice_id: 4,
        ..Settings::default()
    };
    let draft = r.svc.settings_to_save(stale).await;
    assert!(draft.voice_enabled && draft.voice_id == 4);
    save(&r, draft).await;
    let status = r.svc.status().await;
    assert!(status.enabled && status.downloading, "{status:?}");
    assert!(r.stored().voice_enabled);
    assert!(!enabling.is_finished());
    r.svc.download_cancel();
    let _ = enabling.await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_view_is_kept_while_off_and_used_once_on() {
    let r = rig(Mode::Normal).await;
    let view = View {
        account: "personal".into(),
        workspace: "default".into(),
        screen: "inbox".into(),
        dot_id: None,
        topic_root: None,
    };
    r.svc.set_view(view.clone()).unwrap();
    assert_eq!(r.factory.builds(), 0, "nothing starts for a view");
    r.svc.enable(true).await.unwrap();
    r.factory.session(0).emit(SessionEvent::Heard {
        text: "what needs me".into(),
        turn: 1,
    });
    let mind = r.host.mind.clone();
    until("the mind heard", || !mind.views.lock().unwrap().is_empty()).await;
    assert_eq!(mind.views.lock().unwrap()[0], view);
    // A field longer than 256 bytes is refused.
    let long = View {
        screen: "x".repeat(257),
        ..view
    };
    let err = r.svc.set_view(long).unwrap_err();
    assert_eq!(err.kind, CmdErrorKind::Invalid);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preview_refuses_a_voice_or_speed_out_of_range() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    for (voice, speed) in [
        (10, 1.0),
        (1, 0.7),
        (1, 1.5),
        (1, f32::NAN),
        (1, f32::INFINITY),
    ] {
        let err = r.svc.preview(voice, speed).await.unwrap_err();
        assert_eq!(err.kind, CmdErrorKind::Invalid, "{voice} {speed}");
        assert_ne!(err.message, NOT_READY, "{voice} {speed}");
    }
    // An option index far out of range is ignored.
    r.svc.pick(u32::MAX as usize);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_microphone_failure_stays_in_the_status_until_it_listens() {
    let r = rig(Mode::Normal).await;
    r.svc.enable(true).await.unwrap();
    let session = r.factory.session(0);
    session.emit(SessionEvent::Trouble {
        message: "The microphone could not be opened: no microphone was found".into(),
    });
    let svc = r.svc.clone();
    until("kept", || svc.status_now().trouble.is_some()).await;
    session.emit(SessionEvent::State {
        state: State::Listening,
        mic_open: true,
    });
    until("cleared", || svc.status_now().trouble.is_none()).await;
    // Other trouble is only told, not kept.
    session.emit(SessionEvent::Trouble {
        message: "No reply came back.".into(),
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(r.svc.status().await.trouble, None);
}
