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
use super::notices::{Needs, Notices};
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
    /// Blocking (models load, devices open). A failure is a sentence for people. `wanted` is
    /// asked just before the session starts (and the microphone may open): `Ok(None)` when
    /// it is no longer wanted (voice turned off, devices changed, Dots quitting).
    fn build(&self, spec: &BuildSpec, wanted: &dyn Fn() -> bool) -> Result<Option<Built>, String>;
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
    /// Everything in the Inbox now, and how many of its approvals are private; `None` when
    /// it could not be read (nothing is said then, and what waits keeps waiting).
    async fn waiting(&self) -> Option<(usize, usize)>;
}

/// The global hotkey: the plugin in the app (`hotkey::PluginHotkey`), a fake in tests.
pub trait HotkeyPort: Send + Sync + 'static {
    /// Registers `shortcut` in place of the one registered before, which is let go only once
    /// the new one works: on a failure (a sentence for people) the one before still works.
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
    /// The person asked for it (`enable`): its failure turns voice off. A resume (at start or
    /// on `open`) that fails leaves voice on, says why, and is tried again on the next `open`
    /// or `enable`.
    asked: bool,
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
    /// Whether the speech files are in place: read at the start, after a download ends and
    /// after a removal, not on every status.
    installed: bool,
    /// Dots is quitting or restarting: a download that ends now changes nothing.
    shutting_down: bool,
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
    /// Whether a build (or a rebuild) is under way, to wait on.
    building_tx: watch::Sender<bool>,
    /// Spoken notices: what waits to be said, and when one was said last.
    notices: Mutex<Notices>,
    /// A wake-up is set for when what waits may be said.
    notice_timer: Mutex<bool>,
}

/// Where the session stands for a notice.
enum Rest {
    /// Notices off, or voice off or not ready: nothing waits.
    Off,
    /// The person speaks, Dot thinks or speaks, or a question or card shows.
    Busy,
    /// Idle with nothing showing: a notice may be said through this session.
    Resting(Arc<dyn SessionControl>),
}

/// The longest a `View` field may be, in bytes.
const VIEW_FIELD_MAX: usize = 256;
const VIEW_TOO_LONG: &str = "That view is too long.";
const NO_SUCH_VOICE: &str = "Pick one of the ten voices.";
const BAD_SPEED: &str = "Speaking speed must be between 0.8 and 1.4.";

/// The same microphone and speaker: a session built for one serves the other.
fn same_devices(a: &BuildSpec, b: &BuildSpec) -> bool {
    a.mic == b.mic && a.speaker == b.speaker
}

/// A microphone or speaker failure the session reported: kept in the status until the
/// microphone opens again.
fn device_trouble(message: &str) -> bool {
    message.starts_with("The microphone") || message.starts_with("The speaker")
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
        let installed = dots_voice::installed(&root, &manifest);
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
                installed,
                shutting_down: false,
            }),
            state_tx: watch::channel((0, State::Off)).0,
            hotkey_port: Mutex::new(None),
            registered: Mutex::new(None),
            listener: Mutex::new(None),
            told: Mutex::new((false, false)),
            building_tx: watch::channel(false).0,
            notices: Mutex::new(Notices::new()),
            notice_timer: Mutex::new(false),
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
        let error = match &wanted {
            // In place of the one before, which keeps working when this one fails.
            Some(key) => match port.register(key) {
                Ok(()) => {
                    *registered = Some(key.clone());
                    None
                }
                Err(sentence) => Some(sentence),
            },
            None => {
                if registered.take().is_some() {
                    port.unregister();
                }
                None
            }
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

    /// Reads the disk again (outside the lock) for whether the speech files are in place.
    fn refresh_installed(&self) {
        let installed = dots_voice::installed(&self.root, &self.manifest);
        self.lock().installed = installed;
    }

    fn status_of(&self, s: &Inner) -> VoiceStatus {
        VoiceStatus {
            enabled: s.settings.voice_enabled,
            installed: s.installed,
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
    /// Sent under the lock (the channel never blocks), so statuses arrive in order.
    fn changed(&self) {
        {
            let mut s = self.lock();
            let status = self.status_of(&s);
            if s.last_sent.as_ref() != Some(&status) {
                s.last_sent = Some(status.clone());
                self.send(VoiceEvent::Status { status });
            }
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
            if let Some(d) = s.download.as_mut() {
                // A resume under way is now one the person asked for.
                d.asked = true;
                drop(s);
                self.changed();
                return Ok(self.status_now());
            }
            (!s.installed).then(|| self.start_download(&mut s, true))
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
            None => self.settle(false).await,
        }
        self.sync_hotkey();
        self.changed();
        Ok(self.status_now())
    }

    /// Starts the download (`asked`: by the person, through `enable`); on success the session
    /// is built and greets once.
    fn start_download(
        &self,
        s: &mut Inner,
        asked: bool,
    ) -> watch::Receiver<Option<Result<(), CmdError>>> {
        let cancel = CancellationToken::new();
        let (tx, rx) = watch::channel(None);
        s.download = Some(Download {
            cancel: cancel.clone(),
            done: rx.clone(),
            asked,
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

    /// A cancelled download, or a failed one the person asked for, turns voice off (a cut one
    /// resumes when turned on again); a resume that failed (no network yet at login) leaves
    /// voice on and says why, and the next `open` or `enable` tries again. A finished one
    /// builds the session, which greets. While Dots quits nothing is saved or told: the part
    /// stays, and the next start resumes it.
    async fn download_ended(&self, result: &Result<(), CmdError>) {
        let asked = {
            let mut s = self.lock();
            if s.shutting_down {
                s.download = None;
                return;
            }
            s.download.as_ref().is_some_and(|d| d.asked)
        };
        self.refresh_installed();
        match result {
            Ok(()) => {
                self.lock().download = None;
                self.settle(true).await;
            }
            Err(e) if !asked && e.kind != crate::error::CmdErrorKind::Cancelled => {
                let mut s = self.lock();
                s.download = None;
                s.trouble = Some(if e.message == download::RESUMES {
                    download::RESUMES_LATER.to_string()
                } else {
                    e.message.clone()
                });
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

    fn set_building(&self, s: &mut Inner, building: bool) {
        s.building = building;
        self.building_tx.send_replace(building);
    }

    /// Whether a session for `spec`, built from generation `gen`, is what the settings ask
    /// for now.
    fn wanted_now(&self, s: &Inner, gen: u64, spec: &BuildSpec) -> bool {
        s.gen == gen
            && s.settings.voice_enabled
            && !s.shutting_down
            && same_devices(&self.spec_of(&s.settings), spec)
    }

    /// Brings what runs in line with the settings: builds the session when voice is on, the
    /// files are in place and none runs; rebuilds it on another microphone or speaker; sets a
    /// new voice, speed or wake phrase on it. Loops until what runs is what the settings ask
    /// for, so a change made while a build was under way is never lost. One caller does this
    /// at a time; another returns at once, and the one at work sees its change. A build that
    /// failed is not tried again for the same devices here (`open` and `enable` try again).
    async fn settle(&self, greet: bool) {
        enum Step {
            Done,
            Build(BuildSpec, u64),
            Rebuild,
        }
        let mut greet = greet;
        let mut failed: Option<BuildSpec> = None;
        {
            let mut s = self.lock();
            if s.building {
                return;
            }
            self.set_building(&mut s, true);
        }
        loop {
            let step = {
                let mut s = self.lock();
                let wanted = self.spec_of(&s.settings);
                let on = s.settings.voice_enabled
                    && !s.shutting_down
                    && s.download.is_none()
                    && s.installed;
                let step = if !on {
                    Step::Done
                } else {
                    match s.live.as_mut() {
                        Some(l) if same_devices(&l.spec, &wanted) => {
                            if l.spec.voice != wanted.voice || l.spec.speed != wanted.speed {
                                l.session.set_voice(wanted.voice, wanted.speed);
                            }
                            if l.spec.settings != wanted.settings {
                                l.session.settings(wanted.settings);
                            }
                            l.spec = wanted;
                            Step::Done
                        }
                        Some(_) => Step::Rebuild,
                        None if failed.as_ref().is_some_and(|f| same_devices(f, &wanted)) => {
                            Step::Done
                        }
                        None => {
                            s.trouble = None;
                            Step::Build(wanted, s.gen)
                        }
                    }
                };
                // Decided under the same lock that ends the work: a caller that found this
                // one at work changed the settings before this look.
                if matches!(step, Step::Done) {
                    self.set_building(&mut s, false);
                }
                step
            };
            match step {
                Step::Done => break,
                Step::Rebuild => {
                    if let Some(live) = self.take_live() {
                        Self::stop_live(live).ended(END_WAIT).await;
                    }
                    self.changed();
                }
                Step::Build(spec, gen) => {
                    self.changed();
                    match self.build_one(&spec, gen, greet).await {
                        Ok(true) => greet = false,
                        Ok(false) => {}
                        Err(()) => failed = Some(spec),
                    }
                }
            }
        }
        self.changed();
    }

    /// Builds one session for `spec`. `Ok(true)`: it runs; `Ok(false)`: it was no longer
    /// wanted (nothing runs for it, and no microphone opened unless the settings moved on
    /// after the last check, in which case it is shut down at once); `Err`: it failed while
    /// still wanted, and `trouble` says why.
    async fn build_one(&self, spec: &BuildSpec, gen: u64, greet: bool) -> Result<bool, ()> {
        let began = Instant::now();
        let factory = self.factory.clone();
        let me = self.me.clone();
        let (wanted, asked) = (spec.clone(), spec.clone());
        let built = tauri::async_runtime::spawn_blocking(move || {
            let still = move || {
                me.upgrade()
                    .is_some_and(|me| me.wanted_now(&me.lock(), gen, &asked))
            };
            factory.build(&wanted, &still)
        })
        .await
        .unwrap_or_else(|_| Err(MODELS_BROKEN.to_string()));
        let mut s = self.lock();
        let still = self.wanted_now(&s, gen, spec);
        match built {
            Ok(Some(built)) if still => {
                tracing::info!("voice: session built in {} ms", began.elapsed().as_millis());
                let bridge = Bridge::new(built.session.clone(), self.host.mind(), self.ui.clone());
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
                    spec: spec.clone(),
                    session: built.session,
                    bridge,
                    ended,
                });
                Ok(true)
            }
            // Turned off, torn down or moved to other devices while it was being built.
            Ok(Some(built)) => {
                built.session.shutdown();
                Ok(false)
            }
            Ok(None) => Ok(false),
            Err(sentence) if still => {
                tracing::warn!("voice: the session could not be built");
                s.trouble = Some(sentence);
                Err(())
            }
            Err(_) => Ok(false),
        }
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
                let trouble = match &ev {
                    SessionEvent::Trouble { message } if device_trouble(message) => {
                        Some(message.clone())
                    }
                    _ => None,
                };
                bridge.on_session(ev);
                if let (Some(message), Some(me)) = (trouble, me.upgrade()) {
                    me.session_trouble(gen, message);
                }
                if state.is_some() {
                    if let Some(me) = me.upgrade() {
                        me.changed();
                        // At rest again, after the bridge saw it: what waits may be said.
                        if matches!(state, Some((State::Idle, _))) {
                            me.kick_notices();
                        }
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
            // It listens: a device failure kept in the status is over.
            if mic_open && matches!(state, State::Listening | State::Asking) {
                s.trouble = None;
            }
        }
    }

    /// The running session could not use the microphone or the speaker: the status keeps
    /// the sentence until it listens again.
    fn session_trouble(&self, gen: u64, message: String) {
        {
            let mut s = self.lock();
            if !s.live.as_ref().is_some_and(|l| l.gen == gen) {
                return;
            }
            s.trouble = Some(message);
        }
        self.changed();
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
        self.clear_notices();
        self.saw_state(State::Off);
        self.changed();
    }

    /// Takes the running session out; nothing built before this is kept.
    fn take_live(&self) -> Option<Live> {
        let live = {
            let mut s = self.lock();
            s.gen += 1;
            s.state = State::Off;
            s.mic_open = false;
            self.saw_state(State::Off);
            s.live.take()
        };
        // Not ready: nothing waits to be said.
        self.clear_notices();
        live
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
        // A build under way is loading the files: it ends (finding it is no longer wanted)
        // before they are deleted.
        let mut building = self.building_tx.subscribe();
        let _ = tokio::time::timeout(END_WAIT, building.wait_for(|b| !*b)).await;
        let root = self.root.clone();
        let removed = tauri::async_runtime::spawn_blocking(move || dots_voice::remove(&root)).await;
        self.refresh_installed();
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
        let (enabled, busy, installed) = {
            let s = self.lock();
            // Dots is quitting: nothing starts now, no download either.
            if s.shutting_down {
                return;
            }
            if let Some(live) = &s.live {
                let bridge = live.bridge.clone();
                drop(s);
                bridge.open();
                return;
            }
            (
                s.settings.voice_enabled,
                s.download.is_some() || s.building,
                s.installed,
            )
        };
        if !enabled {
            self.trouble(OFF);
            return;
        }
        if busy {
            self.trouble(GETTING_READY);
            return;
        }
        if !installed {
            // A download that ended with Dots: it resumes.
            self.trouble(GETTING_READY);
            let mut s = self.lock();
            if s.download.is_none() {
                self.start_download(&mut s, false);
            }
            drop(s);
            self.changed();
            return;
        }
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            let Some(me) = me.upgrade() else { return };
            me.settle(false).await;
            if let Some(bridge) = me.bridge() {
                bridge.open();
            }
        });
    }

    pub fn close(&self) {
        if let Some(b) = self.bridge() {
            b.close();
            // The stage closed: what waits may be said.
            self.kick_notices();
        }
    }

    pub fn stop(&self) {
        if let Some(b) = self.bridge() {
            b.stop();
        }
    }

    /// A click on option `index` of question `seq`: reaches the mind as `pick(index)` when
    /// `seq` is the question showing.
    pub fn pick(&self, index: usize, seq: u64) {
        if let Some(b) = self.bridge() {
            b.pick(index, seq);
        }
    }

    /// The account, workspace and screen the person looks at. Cheap, and fine with voice off:
    /// it is only kept, and the next session's turns see it. A field longer than 256 bytes is
    /// refused.
    pub fn set_view(&self, view: View) -> Result<(), CmdError> {
        let fields = [
            Some(&view.account),
            Some(&view.workspace),
            Some(&view.screen),
            view.dot_id.as_ref(),
            view.topic_root.as_ref(),
        ];
        if fields
            .into_iter()
            .flatten()
            .any(|f| f.len() > VIEW_FIELD_MAX)
        {
            return Err(CmdError::invalid(VIEW_TOO_LONG));
        }
        let bridge = {
            let mut s = self.lock();
            s.view = view.clone();
            s.live.as_ref().map(|l| l.bridge.clone())
        };
        if let Some(b) = bridge {
            b.set_view(view);
        }
        Ok(())
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
        if voice > 9 {
            return Err(CmdError::invalid(NO_SUCH_VOICE));
        }
        if !(speed.is_finite() && (0.8..=1.4).contains(&speed)) {
            return Err(CmdError::invalid(BAD_SPEED));
        }
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
        let ended = self.apply_now(settings);
        self.apply_rest(ended).await;
    }

    /// `apply` for a settings save: the quick part now, the session (re)build in the
    /// background, so the save never waits on a model load. Its outcome reaches the UI in
    /// `Status` events; it cannot fail the save.
    pub fn apply_in_background(&self, settings: &Settings) {
        let ended = self.apply_now(settings);
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(me) = me.upgrade() {
                me.apply_rest(ended).await;
            }
        });
    }

    /// The quick part of `apply`: keeps the settings, (un)registers the hotkey, sets voice,
    /// speed and wake phrase on the running session; off also cancels the download and shuts
    /// the session down, without waiting for it (what tells when it ended is returned).
    fn apply_now(&self, settings: &Settings) -> Option<Arc<Ended>> {
        {
            let mut s = self.lock();
            s.settings = settings.clone();
            if !settings.voice_enabled {
                s.trouble = None;
            }
            if !settings.voice_enabled || !settings.voice_notices {
                self.clear_notices();
            }
            let wanted = self.spec_of(settings);
            if let Some(l) = s.live.as_mut().filter(|l| same_devices(&l.spec, &wanted)) {
                if l.spec.voice != wanted.voice || l.spec.speed != wanted.speed {
                    l.session.set_voice(wanted.voice, wanted.speed);
                }
                if l.spec.settings != wanted.settings {
                    l.session.settings(wanted.settings);
                }
                l.spec = wanted;
            }
        }
        // The hotkey before the session: a slow build does not hold it back.
        self.sync_hotkey();
        if settings.voice_enabled {
            self.changed();
            return None;
        }
        self.cancel_download();
        let ended = self.take_live().map(|live| {
            tracing::info!("voice: off");
            Self::stop_live(live)
        });
        self.changed();
        ended
    }

    /// The slow part of `apply`: waits for a session turned off to let go of its devices,
    /// and builds or rebuilds the session the settings ask for.
    async fn apply_rest(&self, ended: Option<Arc<Ended>>) {
        if let Some(ended) = ended {
            ended.ended(SHUTDOWN_WAIT).await;
        }
        self.settle(false).await;
        self.changed();
    }

    /// At start: applies the stored settings, and resumes a download that ended with Dots
    /// (voice on, files missing). With voice off nothing is downloaded or built.
    pub async fn start(&self, settings: &Settings) {
        self.refresh_installed();
        self.apply(settings).await;
        {
            let mut s = self.lock();
            if s.settings.voice_enabled && !s.installed && s.download.is_none() && !s.shutting_down
            {
                tracing::info!("voice: resuming the speech files' download");
                self.start_download(&mut s, false);
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
        // First: the download cancelled below then ends without turning voice off.
        self.lock().shutting_down = true;
        self.cancel_download();
        if let Some(live) = self.take_live() {
            let ended = Self::stop_live(live);
            if !ended.wait(SHUTDOWN_WAIT) {
                tracing::warn!("voice: the session did not end in time");
            }
        }
    }

    // ---- spoken notices -------------------------------------------------------------------

    fn notices(&self) -> MutexGuard<'_, Notices> {
        self.notices.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notices_on(s: &Inner) -> bool {
        s.settings.voice_enabled && s.settings.voice_notices && s.live.is_some() && !s.shutting_down
    }

    /// Notices are on and voice is ready: only then is anything looked up, kept or said.
    pub fn notices_wanted(&self) -> bool {
        Self::notices_on(&self.lock())
    }

    /// Nothing waits to be said.
    pub fn clear_notices(&self) {
        self.notices().clear();
    }

    /// Where the session stands for a notice. Resting is the session Idle with the stage
    /// closed: no turn live and no question or card showing.
    fn rest(&self) -> Rest {
        let (session, bridge) = {
            let s = self.lock();
            let Some(live) = s.live.as_ref().filter(|_| Self::notices_on(&s)) else {
                return Rest::Off;
            };
            if s.state != State::Idle {
                return Rest::Busy;
            }
            (live.session.clone(), live.bridge.clone())
        };
        if bridge.at_rest() && session.state() == State::Idle {
            Rest::Resting(session)
        } else {
            Rest::Busy
        }
    }

    /// Something new needs the person (the notices listener): said now when the session
    /// rests and none was said in the last 30 seconds, else kept for the next rest. With
    /// notices off, or voice off or not ready, it is dropped and nothing waits.
    pub async fn need(&self, need: Needs) {
        let session = match self.rest() {
            Rest::Off => return self.clear_notices(),
            Rest::Busy => None,
            Rest::Resting(session) => Some(session),
        };
        let said = {
            let mut n = self.notices();
            n.arrived(need, session.is_some(), Instant::now())
                .map(|line| (line, n.kind()))
        };
        match (said, session) {
            (Some((line, kind)), Some(session)) => say_notice(session.as_ref(), &line, kind),
            // More waits, or the gap has not passed: counted at rest, or later.
            _ => self.notices_at_rest().await,
        }
    }

    /// Events were missed: what waits is counted again from the Inbox.
    pub async fn missed_notices(&self) {
        if !self.notices_wanted() {
            return self.clear_notices();
        }
        self.notices().missed();
        self.notices_at_rest().await;
    }

    /// Checks for what waits, in the background, when something does.
    fn kick_notices(&self) {
        if !self.notices().pending() {
            return;
        }
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(me) = me.upgrade() {
                me.notices_at_rest().await;
            }
        });
    }

    /// The session rests (or a notice's gap has passed): says what still waits, counted from
    /// the Inbox, once 30 seconds have passed since the last notice; before that, a wake-up is
    /// set for then. Busy, it waits for the next rest.
    async fn notices_at_rest(&self) {
        match self.rest() {
            Rest::Off => return self.clear_notices(),
            Rest::Busy => return,
            Rest::Resting(_) => {}
        }
        match self.notices().wait_for(Instant::now()) {
            None => return,
            Some(wait) if !wait.is_zero() => return self.notice_wake_up(wait),
            Some(_) => {}
        }
        let Some((count, private)) = self.host.waiting().await else {
            return;
        };
        // Things may have moved while the Inbox was read.
        let session = match self.rest() {
            Rest::Off => return self.clear_notices(),
            Rest::Busy => return,
            Rest::Resting(session) => session,
        };
        let said = {
            let mut n = self.notices();
            n.rested(count, private, Instant::now())
                .map(|line| (line, n.kind()))
        };
        if let Some((line, kind)) = said {
            say_notice(session.as_ref(), &line, kind);
        }
    }

    /// One wake-up at a time, `wait` from now, to say what waits once its gap has passed.
    fn notice_wake_up(&self, wait: Duration) {
        {
            let mut set = self.notice_timer.lock().unwrap_or_else(|e| e.into_inner());
            if *set {
                return;
            }
            *set = true;
        }
        let me = self.me.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(wait).await;
            if let Some(me) = me.upgrade() {
                *me.notice_timer.lock().unwrap_or_else(|e| e.into_inner()) = false;
                me.notices_at_rest().await;
            }
        });
    }
}

/// Says a notice; the log names only its kind.
fn say_notice(session: &dyn SessionControl, line: &str, kind: &str) {
    session.notice(line);
    tracing::info!("voice: a notice was spoken ({kind})");
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
    fn build(&self, spec: &BuildSpec, wanted: &dyn Fn() -> bool) -> Result<Option<Built>, String> {
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
        if !wanted() {
            return Ok(None);
        }
        let (events_tx, events) = unbounded_channel();
        let session = Session::start(parts, spec.settings, events_tx);
        session.set_voice(spec.voice, spec.speed);
        Ok(Some(Built {
            session: Arc::new(session),
            events,
        }))
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
    async fn waiting(&self) -> Option<(usize, usize)> {
        let count = self.0.inbox_count().await.ok()?;
        let inbox = self.0.inbox().await.ok()?;
        let mut private = 0;
        for pending in &inbox.approvals {
            // An error counts as private.
            if self
                .0
                .approval_touches_private(&pending.approval.id)
                .await
                .unwrap_or(true)
            {
                private += 1;
            }
        }
        Some((count, private))
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
