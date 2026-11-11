//! The voice service: owns the voice session, builds it from the speech files when voice is
//! on, downloads those files when they are missing, and runs the bridge between the session
//! and Open Dot's mind.
//!
//! Off until turned on: with `voice_enabled` false nothing is downloaded, no model is loaded,
//! no microphone is opened and no session thread exists. Logs carry timings and state names
//! only, never what was heard or said.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dots_core::open_dot::picture::View;
use dots_core::settings::Settings;
use dots_core::Runtime;
use dots_voice::{
    CpalMic, CpalSink, EarSettings, Ears, Manifest, Mic, ModelError, ModelPaths, Parts, Recognizer,
    Session, SessionEvent, SessionSettings, SherpaDetector, SherpaRecognizer, SherpaSpeaker,
    SherpaWake, Sink, Speaker, State, Voice, WakeSpotter,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::bridge::{Bridge, MindPort, SessionPort};
use super::download;
use super::events::{VoiceDevice, VoiceDevices, VoiceEvent, VoiceStatus};
use crate::error::CmdError;

/// Said once after the first install, and by the voice preview.
pub const GREETING: &str = "Hi, I'm Dot.";
pub const OFF: &str = "Open Dot is off. Turn it on in Settings, under Voice.";
pub const GETTING_READY: &str = "Open Dot is still getting ready.";
pub const NOT_READY: &str =
    "Open Dot is not ready yet. Turn it on in Settings, under Voice, and wait until it is ready.";
pub const IN_USE: &str = "The speech files are in use. Close Dots and try again.";
pub const MODELS_BROKEN: &str = "The speech files could not be loaded. Remove them in Settings, \
     under Voice, and turn Open Dot on again.";
pub const NO_SPEAKER: &str = "The speaker could not be opened. Check your sound settings.";

/// How long `shutdown` waits for the session to let go of its devices.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(500);
/// How long a removal or a rebuild waits for the old session to end.
const END_WAIT: Duration = Duration::from_secs(5);
/// The previewed voice is put back after the greeting, or after this long at most.
const PREVIEW_LIMIT: Duration = Duration::from_secs(5);

/// What the service needs of a running session beyond what the bridge needs.
pub trait SessionControl: SessionPort {
    fn settings(&self, settings: SessionSettings);
    fn set_voice(&self, voice: u8, speed: f32);
    fn notice(&self, text: &str);
    /// Returns at once; the session has ended when its event channel closes.
    fn shutdown(&self);
}

impl SessionControl for Session {
    fn settings(&self, settings: SessionSettings) {
        Session::settings(self, settings);
    }
    fn set_voice(&self, voice: u8, speed: f32) {
        Session::set_voice(self, voice, speed);
    }
    fn notice(&self, text: &str) {
        Session::notice(self, text);
    }
    fn shutdown(&self) {
        Session::shutdown(self);
    }
}

/// What a session is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildSpec {
    pub root: PathBuf,
    pub mic: Option<String>,
    pub speaker: Option<String>,
    pub settings: SessionSettings,
    pub voice: u8,
    pub speed: f32,
}

/// A built session and its events.
pub struct Built {
    pub session: Arc<dyn SessionControl>,
    pub events: UnboundedReceiver<SessionEvent>,
}

/// Makes sessions. The real one loads the models and opens the speaker; tests use a fake.
pub trait SessionFactory: Send + Sync + 'static {
    /// Blocking (models load, devices open). A failure is a sentence for people.
    fn build(&self, spec: &BuildSpec) -> Result<Built, String>;
    fn microphones(&self) -> Vec<VoiceDevice>;
    fn speakers(&self) -> Vec<VoiceDevice>;
}

/// The settings and the mind: the runtime in the app, a fake in tests.
#[async_trait]
pub trait Host: Send + Sync + 'static {
    async fn settings(&self) -> Settings;
    /// Stores the settings; what was stored.
    async fn save(&self, settings: Settings) -> Result<Settings, CmdError>;
    fn mind(&self) -> Arc<dyn MindPort>;
}

/// The global hotkey: the plugin in the app (`hotkey::PluginHotkey`), a fake in tests.
pub trait HotkeyPort: Send + Sync + 'static {
    /// Registers `shortcut`; a failure is a sentence for people.
    fn register(&self, shortcut: &str) -> Result<(), String>;
    /// Lets go of the shortcut registered last, if any.
    fn unregister(&self);
}

/// Told when the microphone opens or closes and when voice becomes ready or stops being
/// ready, once per change (the tray in the app). Must not block: it is called from the
/// service's own calls.
pub trait Listener: Send + Sync + 'static {
    fn mic_open(&self, open: bool);
    fn ready(&self, ready: bool);
}

/// The session's wake phrase and interruption settings, from Dots' settings.
pub fn session_settings(s: &Settings) -> SessionSettings {
    SessionSettings {
        wake_phrase: s.voice_wake_phrase,
        interrupt_by_talking: s.voice_interrupt_by_talking,
        follow_up_secs: 6,
        reopen_after_ms: 250,
    }
}

/// Set once a session's event channel has closed: its worker is gone, its devices let go.
struct Ended {
    done: Mutex<bool>,
    cv: Condvar,
    tx: watch::Sender<bool>,
}

impl Ended {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            done: Mutex::new(false),
            cv: Condvar::new(),
            tx: watch::channel(false).0,
        })
    }

    fn set(&self) {
        *self.done.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
        self.tx.send_replace(true);
    }

    /// Blocks up to `limit`; whether it ended.
    fn wait(&self, limit: Duration) -> bool {
        let done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        let (done, _) = self
            .cv
            .wait_timeout_while(done, limit, |d| !*d)
            .unwrap_or_else(|e| e.into_inner());
        *done
    }

    async fn ended(&self, limit: Duration) -> bool {
        let mut rx = self.tx.subscribe();
        let ended = tokio::time::timeout(limit, rx.wait_for(|d| *d)).await;
        ended.is_ok()
    }
}

/// The running session.
struct Live {
    gen: u64,
    spec: BuildSpec,
    session: Arc<dyn SessionControl>,
    bridge: Arc<Bridge>,
    ended: Arc<Ended>,
}

/// A download in flight; `done` holds its result once it ended.
struct Download {
    cancel: CancellationToken,
    done: watch::Receiver<Option<Result<(), CmdError>>>,
}

struct Inner {
    settings: Settings,
    view: View,
    download: Option<Download>,
    building: bool,
    live: Option<Live>,
    /// Bumped whenever the session is torn down: a build that finishes after it is dropped.
    gen: u64,
    trouble: Option<String>,
    state: State,
    mic_open: bool,
    hotkey_error: Option<String>,
    last_sent: Option<VoiceStatus>,
}

pub struct Service {
    me: Weak<Service>,
    host: Arc<dyn Host>,
    factory: Arc<dyn SessionFactory>,
    root: PathBuf,
    manifest: Manifest,
    ui: UnboundedSender<VoiceEvent>,
    inner: Mutex<Inner>,
    /// How many states the session went through, and the last.
    state_tx: watch::Sender<(u64, State)>,
    hotkey_port: Mutex<Option<Arc<dyn HotkeyPort>>>,
    /// The shortcut registered now. Held while (un)registering, so changes go in order.
    registered: Mutex<Option<String>>,
    listener: Mutex<Option<Arc<dyn Listener>>>,
    /// What the listener was told last: (mic open, ready). Held while telling it.
    told: Mutex<(bool, bool)>,
}

impl Service {
    /// A service with voice off; `apply` with the stored settings starts what they turn on.
    pub fn new(
        host: Arc<dyn Host>,
        factory: Arc<dyn SessionFactory>,
        root: PathBuf,
        manifest: Manifest,
        ui: UnboundedSender<VoiceEvent>,
    ) -> Arc<Service> {
        Arc::new_cyclic(|me| Service {
            me: me.clone(),
            host,
            factory,
            root,
            manifest,
            ui,
            inner: Mutex::new(Inner {
                settings: Settings::default(),
                view: View::default(),
                download: None,
                building: false,
                live: None,
                gen: 0,
                trouble: None,
                state: State::Off,
                mic_open: false,
                hotkey_error: None,
                last_sent: None,
            }),
            state_tx: watch::channel((0, State::Off)).0,
            hotkey_port: Mutex::new(None),
            registered: Mutex::new(None),
            listener: Mutex::new(None),
            told: Mutex::new((false, false)),
        })
    }

    /// Where the hotkey is registered; set before the stored settings are applied.
    pub fn set_hotkey_port(&self, port: Arc<dyn HotkeyPort>) {
        *self.hotkey_port.lock().unwrap_or_else(|e| e.into_inner()) = Some(port);
    }

    /// Who hears of the microphone and readiness changing (the tray).
    pub fn set_listener(&self, listener: Arc<dyn Listener>) {
        *self.listener.lock().unwrap_or_else(|e| e.into_inner()) = Some(listener);
    }

    /// Registers the hotkey while voice is on and a shortcut is set, re-registers it when the
    /// shortcut changed, and lets it go otherwise. A failure only sets `hotkey_error`.
    fn sync_hotkey(&self) {
        let Some(port) = self
            .hotkey_port
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        let mut registered = self.registered.lock().unwrap_or_else(|e| e.into_inner());
        let wanted = {
            let s = self.lock();
            let key = s.settings.voice_hotkey.trim();
            (s.settings.voice_enabled && !key.is_empty()).then(|| key.to_string())
        };
        // A shortcut that failed is not registered: it is tried again on the next sync.
        if *registered == wanted {
            drop(registered);
            if wanted.is_none() {
                self.set_hotkey_error(None);
            }
            return;
        }
        if registered.take().is_some() {
            port.unregister();
        }
        let error = match &wanted {
            Some(key) => match port.register(key) {
                Ok(()) => {
                    *registered = Some(key.clone());
                    None
                }
                Err(sentence) => Some(sentence),
            },
            None => None,
        };
        drop(registered);
        self.set_hotkey_error(error);
    }

    /// Tells the listener what changed since it was told last.
    fn tell(&self) {
        let Some(listener) = self
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        let mut told = self.told.lock().unwrap_or_else(|e| e.into_inner());
        let now = {
            let s = self.lock();
            (s.mic_open, s.live.is_some())
        };
        if now.0 != told.0 {
            listener.mic_open(now.0);
        }
        if now.1 != told.1 {
            listener.ready(now.1);
        }
        *told = now;
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, event: VoiceEvent) {
        let _ = self.ui.send(event);
    }

    fn installed(&self) -> bool {
        dots_voice::installed(&self.root, &self.manifest)
    }

    fn status_of(&self, s: &Inner) -> VoiceStatus {
        VoiceStatus {
            enabled: s.settings.voice_enabled,
            installed: self.installed(),
            downloading: s.download.is_some(),
            ready: s.live.is_some(),
            state: s.state,
            mic_open: s.mic_open,
            download_bytes: self.manifest.total_bytes(),
            hotkey_error: s.hotkey_error.clone(),
            trouble: s.trouble.clone(),
        }
    }

    fn status_now(&self) -> VoiceStatus {
        let s = self.lock();
        self.status_of(&s)
    }

    /// Sends `Status` when the status differs from the last one sent, and tells the listener.
    fn changed(&self) {
        let mut s = self.lock();
        let status = self.status_of(&s);
        if s.last_sent.as_ref() != Some(&status) {
            s.last_sent = Some(status.clone());
            drop(s);
            self.send(VoiceEvent::Status { status });
        } else {
            drop(s);
        }
        self.tell();
    }

    pub async fn status(&self) -> VoiceStatus {
        self.status_now()
    }

    /// Turns voice on or off and stores it. On: downloads the speech files if they are
    /// missing (one download at a time: a second call while one runs answers at once), then
    /// builds the session. Off: stops the session and lets go of the microphone.
    pub async fn enable(&self, on: bool) -> Result<VoiceStatus, CmdError> {
        let mut settings = self.host.settings().await;
        if settings.voice_enabled != on {
            settings.voice_enabled = on;
            settings = self.host.save(settings).await?;
        }
        if !on {
            self.apply(&settings).await;
            return Ok(self.status_now());
        }
        let done = {
            let mut s = self.lock();
            s.settings = settings;
            if s.download.is_some() {
                drop(s);
                self.changed();
                return Ok(self.status_now());
            }
            (!self.installed()).then(|| self.start_download(&mut s))
        };
        self.changed();
        match done {
            Some(mut done) => {
                let result = done
                    .wait_for(|r| r.is_some())
                    .await
                    .map_err(|_| CmdError::internal(download::RESUMES))?
                    .clone();
                result.expect("waited for a result")?;
            }
            None => self.build(false).await,
        }
        self.sync_hotkey();
        self.changed();
        Ok(self.status_now())
    }

    /// Starts the download; on success the session is built and greets once.
    fn start_download(&self, s: &mut Inner) -> watch::Receiver<Option<Result<(), CmdError>>> {
        let cancel = CancellationToken::new();
        let (tx, rx) = watch::channel(None);
        s.download = Some(Download {
            cancel: cancel.clone(),
            done: rx.clone(),
        });
        s.trouble = None;
        let (me, root, manifest, ui) = (
            self.me.clone(),
            self.root.clone(),
            self.manifest.clone(),
            self.ui.clone(),
        );
        tauri::async_runtime::spawn(async move {
            let began = Instant::now();
            let progress = |done_bytes, total_bytes| {
                let _ = ui.send(VoiceEvent::Download {
                    done_bytes,
                    total_bytes,
                });
            };
            let result = download::run(&root, &manifest, cancel, progress).await;
            match &result {
                Ok(()) => tracing::info!(
                    "voice: speech files installed in {} s",
                    began.elapsed().as_secs()
                ),
                Err(e) => tracing::warn!("voice: download ended: {}", kind_of(e)),
            }
            let result = result.map_err(|e| download::sentence(&e));
            if let Some(me) = me.upgrade() {
                me.download_ended(&result).await;
            }
            let _ = tx.send(Some(result));
        });
        rx
    }

    /// A failed or cancelled download turns voice off (a cut one resumes when turned on
    /// again); a finished one builds the session, which greets.
    async fn download_ended(&self, result: &Result<(), CmdError>) {
        match result {
            Ok(()) => {
                self.lock().download = None;
                self.build(true).await;
            }
            Err(e) => {
                let mut settings = self.host.settings().await;
                if settings.voice_enabled {
                    settings.voice_enabled = false;
                    if let Ok(saved) = self.host.save(settings.clone()).await {
                        settings = saved;
                    }
                }
                let mut s = self.lock();
                s.download = None;
                s.settings = settings;
                s.trouble =
                    (e.kind != crate::error::CmdErrorKind::Cancelled).then(|| e.message.clone());
                drop(s);
                self.sync_hotkey();
            }
        }
        self.changed();
    }

    /// Builds the session when voice is on, the files are in place and none runs.
    async fn build(&self, greet: bool) {
        let (spec, gen) = {
            let mut s = self.lock();
            if !s.settings.voice_enabled || s.live.is_some() || s.building || !self.installed() {
                return;
            }
            s.building = true;
            s.trouble = None;
            (self.spec_of(&s.settings), s.gen)
        };
        self.changed();
        let began = Instant::now();
        let factory = self.factory.clone();
        let wanted = spec.clone();
        let built = tauri::async_runtime::spawn_blocking(move || factory.build(&wanted))
            .await
            .unwrap_or_else(|_| Err(MODELS_BROKEN.to_string()));
        {
            let mut s = self.lock();
            s.building = false;
            match built {
                Ok(built) if s.gen == gen && s.settings.voice_enabled => {
                    tracing::info!("voice: session built in {} ms", began.elapsed().as_millis());
                    let bridge =
                        Bridge::new(built.session.clone(), self.host.mind(), self.ui.clone());
                    bridge.set_view(s.view.clone());
                    let ended = Ended::new();
                    s.state = built.session.state();
                    self.saw_state(s.state);
                    if greet {
                        built.session.notice(GREETING);
                    }
                    self.listen(gen, bridge.clone(), built.events, ended.clone());
                    s.live = Some(Live {
                        gen,
                        spec,
                        session: built.session,
                        bridge,
                        ended,
                    });
                }
                // Turned off, or torn down, while it was being built.
                Ok(built) => built.session.shutdown(),
                Err(sentence) => {
                    tracing::warn!("voice: the session could not be built");
                    s.trouble = Some(sentence);
                }
            }
        }
        self.changed();
    }

    fn spec_of(&self, s: &Settings) -> BuildSpec {
        BuildSpec {
            root: self.root.clone(),
            mic: s.voice_mic.clone(),
            speaker: s.voice_speaker.clone(),
            settings: session_settings(s),
            voice: s.voice_id,
            speed: s.voice_speed,
        }
    }

    /// One task takes the session's events in order: the status follows its state, and the
    /// bridge gets every event. The channel closing means the session has ended.
    fn listen(
        &self,
        gen: u64,
        bridge: Arc<Bridge>,
        mut events: UnboundedReceiver<SessionEvent>,
        ended: Arc<Ended>,
    ) {
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(ev) = events.recv().await {
                let state = match &ev {
                    SessionEvent::State { state, mic_open } => Some((*state, *mic_open)),
                    _ => None,
                };
                if let (Some((state, mic_open)), Some(me)) = (state, me.upgrade()) {
                    me.session_state(gen, state, mic_open);
                }
                bridge.on_session(ev);
                if state.is_some() {
                    if let Some(me) = me.upgrade() {
                        me.changed();
                    }
                }
            }
            ended.set();
            if let Some(me) = me.upgrade() {
                me.session_ended(gen);
            }
        });
    }

    fn session_state(&self, gen: u64, state: State, mic_open: bool) {
        let mut s = self.lock();
        if s.live.as_ref().is_some_and(|l| l.gen == gen) {
            s.state = state;
            s.mic_open = mic_open;
            self.saw_state(state);
        }
    }

    /// The session ended by itself (a failure): it is built again on the next `open`.
    fn session_ended(&self, gen: u64) {
        {
            let mut s = self.lock();
            if !s.live.as_ref().is_some_and(|l| l.gen == gen) {
                return;
            }
            tracing::warn!("voice: the session ended");
            s.live = None;
            s.state = State::Off;
            s.mic_open = false;
        }
        self.saw_state(State::Off);
        self.changed();
    }

    /// Takes the running session out; nothing built before this is kept.
    fn take_live(&self) -> Option<Live> {
        let mut s = self.lock();
        s.gen += 1;
        s.state = State::Off;
        s.mic_open = false;
        self.saw_state(State::Off);
        s.live.take()
    }

    /// Closes the stage and shuts the session down; what tells when it has ended.
    fn stop_live(live: Live) -> Arc<Ended> {
        live.bridge.close();
        live.session.shutdown();
        live.ended
    }

    /// Cancels a download in flight; what tells when it has ended.
    fn cancel_download(&self) -> Option<watch::Receiver<Option<Result<(), CmdError>>>> {
        let s = self.lock();
        s.download.as_ref().map(|d| {
            d.cancel.cancel();
            d.done.clone()
        })
    }

    /// Stops the download.
    pub fn download_cancel(&self) {
        self.cancel_download();
    }

    /// Turns voice off, stops the session and waits for it to end, then deletes the speech
    /// files.
    pub async fn remove(&self) -> Result<VoiceStatus, CmdError> {
        let mut settings = self.host.settings().await;
        if settings.voice_enabled {
            settings.voice_enabled = false;
            settings = self.host.save(settings).await?;
        }
        self.lock().settings = settings;
        if let Some(mut done) = self.cancel_download() {
            let _ = done.wait_for(|r| r.is_some()).await;
        }
        if let Some(live) = self.take_live() {
            Self::stop_live(live).ended(END_WAIT).await;
        }
        let root = self.root.clone();
        let removed = tauri::async_runtime::spawn_blocking(move || dots_voice::remove(&root)).await;
        self.lock().trouble = None;
        self.sync_hotkey();
        self.changed();
        match removed {
            Ok(Ok(())) => Ok(self.status_now()),
            Ok(Err(ModelError::Disk(_))) | Err(_) => Err(CmdError::internal(IN_USE)),
            Ok(Err(e)) => Err(download::sentence(&e)),
        }
    }

    fn bridge(&self) -> Option<Arc<Bridge>> {
        self.lock().live.as_ref().map(|l| l.bridge.clone())
    }

    fn trouble(&self, message: &str) {
        self.send(VoiceEvent::Trouble {
            message: message.into(),
        });
    }

    /// Listen (hotkey, rail, tray). Not ready: a sentence says why; a session that failed to
    /// build (or ended) is built again first.
    pub fn open(&self) {
        let (enabled, busy) = {
            let s = self.lock();
            if let Some(live) = &s.live {
                let bridge = live.bridge.clone();
                drop(s);
                bridge.open();
                return;
            }
            (s.settings.voice_enabled, s.download.is_some() || s.building)
        };
        if !enabled {
            self.trouble(OFF);
            return;
        }
        if busy {
            self.trouble(GETTING_READY);
            return;
        }
        if !self.installed() {
            // A download that ended with Dots: it resumes.
            self.trouble(GETTING_READY);
            let mut s = self.lock();
            if s.download.is_none() {
                self.start_download(&mut s);
            }
            drop(s);
            self.changed();
            return;
        }
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            let Some(me) = me.upgrade() else { return };
            me.build(false).await;
            if let Some(bridge) = me.bridge() {
                bridge.open();
            }
        });
    }

    pub fn close(&self) {
        if let Some(b) = self.bridge() {
            b.close();
        }
    }

    pub fn stop(&self) {
        if let Some(b) = self.bridge() {
            b.stop();
        }
    }

    /// A click on option `index`: reaches the mind as `pick(index)`.
    pub fn pick(&self, index: usize) {
        if let Some(b) = self.bridge() {
            b.pick(index);
        }
    }

    pub fn set_view(&self, view: View) {
        let bridge = {
            let mut s = self.lock();
            s.view = view.clone();
            s.live.as_ref().map(|l| l.bridge.clone())
        };
        if let Some(b) = bridge {
            b.set_view(view);
        }
    }

    /// The session's state moved on (or the service set it).
    fn saw_state(&self, state: State) {
        self.state_tx.send_modify(|(n, st)| {
            *n += 1;
            *st = state;
        });
    }

    /// Says the greeting in `voice` at `speed`, then puts the saved voice back once the
    /// session rests again (or after five seconds).
    pub async fn preview(&self, voice: u8, speed: f32) -> Result<(), CmdError> {
        let session = {
            let s = self.lock();
            match &s.live {
                Some(l) if s.state == State::Idle => l.session.clone(),
                _ => return Err(CmdError::invalid(NOT_READY)),
            }
        };
        let mut states = self.state_tx.subscribe();
        let (start, _) = *states.borrow_and_update();
        session.set_voice(voice, speed);
        session.notice(GREETING);
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            // The session never reports one state twice in a row: Idle after any change means
            // it spoke and rests again, however quickly the states went by.
            let rested = states.wait_for(|(n, st)| *n > start && *st == State::Idle);
            let _ = tokio::time::timeout(PREVIEW_LIMIT, rested).await;
            if let Some(me) = me.upgrade() {
                let s = me.lock();
                if let Some(l) = &s.live {
                    l.session
                        .set_voice(s.settings.voice_id, s.settings.voice_speed);
                }
            }
        });
        Ok(())
    }

    pub fn devices(&self) -> VoiceDevices {
        VoiceDevices {
            microphones: self.factory.microphones(),
            speakers: self.factory.speakers(),
        }
    }

    /// Saved settings apply at once: off stops the session, lets go of the microphone and the
    /// hotkey; a new microphone or speaker rebuilds it; voice, speed, wake phrase and
    /// interruption are set on the running session; a new hotkey is registered in place of
    /// the old (a failure only sets `hotkey_error`).
    pub async fn apply(&self, settings: &Settings) {
        let live = {
            let mut s = self.lock();
            s.settings = settings.clone();
            if !settings.voice_enabled {
                s.trouble = None;
            }
            s.live.as_ref().map(|l| (l.spec.clone(), l.session.clone()))
        };
        if !settings.voice_enabled {
            self.sync_hotkey();
            self.cancel_download();
            if let Some(live) = self.take_live() {
                Self::stop_live(live).ended(SHUTDOWN_WAIT).await;
                tracing::info!("voice: off");
            }
            self.changed();
            return;
        }
        // The hotkey before the session: a slow build does not hold it back.
        self.sync_hotkey();
        let wanted = self.spec_of(settings);
        match live {
            Some((had, _)) if had.mic != wanted.mic || had.speaker != wanted.speaker => {
                if let Some(live) = self.take_live() {
                    Self::stop_live(live).ended(END_WAIT).await;
                }
                self.build(false).await;
            }
            Some((had, session)) => {
                if had.voice != wanted.voice || had.speed != wanted.speed {
                    session.set_voice(wanted.voice, wanted.speed);
                }
                if had.settings != wanted.settings {
                    session.settings(wanted.settings);
                }
                let mut s = self.lock();
                if let Some(l) = s.live.as_mut() {
                    l.spec = wanted;
                }
            }
            None => {
                let downloading = self.lock().download.is_some();
                if !downloading && self.installed() {
                    self.build(false).await;
                }
            }
        }
        self.changed();
    }

    /// The hotkey could not be registered (or now could): shown in the status.
    pub fn set_hotkey_error(&self, error: Option<String>) {
        self.lock().hotkey_error = error;
        self.changed();
    }

    /// On exit, tray quit and restart: stops the download and the session, and waits up to
    /// 500 ms for the session to let go of its devices. Blocks.
    pub fn shutdown(&self) {
        self.cancel_download();
        if let Some(live) = self.take_live() {
            let ended = Self::stop_live(live);
            if !ended.wait(SHUTDOWN_WAIT) {
                tracing::warn!("voice: the session did not end in time");
            }
        }
    }
}

/// A download error's state name, for the log.
fn kind_of(e: &ModelError) -> &'static str {
    match e {
        ModelError::Cancelled => "cancelled",
        ModelError::Network(_) => "network",
        ModelError::WrongHash { .. } => "wrong hash",
        ModelError::Disk(_) => "disk",
        ModelError::Stalled => "stalled",
        ModelError::Busy => "busy",
    }
}

// ---- the real parts -------------------------------------------------------------------------

/// Sessions on the real models, microphone and speaker (`dots-voice`'s session example).
pub struct RealFactory;

fn device(d: dots_voice::Device) -> VoiceDevice {
    VoiceDevice {
        id: d.id,
        name: d.name,
        default: d.default,
    }
}

impl SessionFactory for RealFactory {
    fn build(&self, spec: &BuildSpec) -> Result<Built, String> {
        let broken = |_| MODELS_BROKEN.to_string();
        let paths = ModelPaths {
            root: spec.root.clone(),
        };
        let speaker: Arc<dyn Speaker> = Arc::new(SherpaSpeaker::load(&paths, 2).map_err(broken)?);
        let recognizer: Arc<dyn Recognizer> =
            Arc::new(SherpaRecognizer::load(&paths).map_err(broken)?);
        let ears = Ears::new(
            Box::new(SherpaDetector::load(&paths).map_err(broken)?),
            recognizer.clone(),
            EarSettings::default(),
        );
        let short_ears = Ears::new(
            Box::new(SherpaDetector::load(&paths).map_err(broken)?),
            recognizer,
            EarSettings::short_answer(),
        );
        let wake: Option<Box<dyn WakeSpotter>> = match SherpaWake::load(&paths) {
            Ok(w) => Some(Box::new(w)),
            Err(_) => None,
        };
        let out = spec.speaker.clone();
        let sink: Arc<dyn Sink> =
            Arc::new(CpalSink::open(out.as_deref()).map_err(|_| NO_SPEAKER.to_string())?);
        let (voice_tx, voice_events) = unbounded_channel();
        let voice = Voice::new(speaker, sink.clone(), voice_tx);
        let mic = spec.mic.clone();
        let parts = Parts {
            ears,
            short_ears,
            voice,
            voice_events,
            sink,
            open_sink: Box::new(move || {
                CpalSink::open(out.as_deref()).map(|s| Arc::new(s) as Arc<dyn Sink>)
            }),
            wake,
            open_mic: Box::new(move || {
                CpalMic::open(mic.as_deref()).map(|m| Box::new(m) as Box<dyn Mic>)
            }),
        };
        let (events_tx, events) = unbounded_channel();
        let session = Session::start(parts, spec.settings, events_tx);
        session.set_voice(spec.voice, spec.speed);
        Ok(Built {
            session: Arc::new(session),
            events,
        })
    }

    fn microphones(&self) -> Vec<VoiceDevice> {
        dots_voice::microphones().into_iter().map(device).collect()
    }

    fn speakers(&self) -> Vec<VoiceDevice> {
        dots_voice::speakers().into_iter().map(device).collect()
    }
}

/// Open Dot's mind over the runtime. Each call reaches the mind with no await before it, so
/// a turn has begun at the mind once its call was first polled.
pub struct RuntimeMind(pub Arc<Runtime>);

#[async_trait]
impl MindPort for RuntimeMind {
    async fn hear(
        &self,
        heard: &str,
        view: &View,
        out: &UnboundedSender<dots_core::open_dot::MindEvent>,
    ) {
        self.0.open_dot().hear(&self.0, heard, view, out).await
    }
    async fn pick(
        &self,
        index: usize,
        view: &View,
        out: &UnboundedSender<dots_core::open_dot::MindEvent>,
    ) {
        self.0.open_dot().pick(&self.0, index, view, out).await
    }
    fn lapse(&self, out: &UnboundedSender<dots_core::open_dot::MindEvent>) {
        self.0.open_dot().lapse(out);
    }
    fn cancel(&self) {
        self.0.open_dot().cancel();
    }
    fn reset(&self) -> bool {
        self.0.open_dot().reset()
    }
    fn pending(&self) -> bool {
        self.0.open_dot().pending()
    }
}

/// The runtime's settings (saved the way `update_settings` saves them) and its mind.
pub struct RuntimeHost(pub Arc<Runtime>);

#[async_trait]
impl Host for RuntimeHost {
    async fn settings(&self) -> Settings {
        self.0.settings().await
    }
    async fn save(&self, settings: Settings) -> Result<Settings, CmdError> {
        // Only `voice_enabled` changes here: no program path, so nothing to confirm.
        Ok(self.0.update_settings(settings).await?.settings)
    }
    fn mind(&self) -> Arc<dyn MindPort> {
        Arc::new(RuntimeMind(self.0.clone()))
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
