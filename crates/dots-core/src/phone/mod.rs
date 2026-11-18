//! Answer from your phone (phone view spec, docs/superpowers/specs/2026-11-06-phone-view-design.md).
//!
//! Off by default. When Settings turns it on, a listener of its own (the local server on
//! 127.0.0.1 is not touched) binds the PC's private network addresses only, speaks HTTPS with a
//! self-signed certificate, and serves a small page that shows the Inbox's approvals, questions
//! and waiting runs to phones the person paired. Turning it off closes it at once.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::model::{Approval, Inbox, Message, Run};
use crate::settings::Settings;
use crate::store::{PhoneAction, PhoneDevice, Store};
use crate::util::{new_id, new_token, now, timestamp, token_hash};
use crate::vault::org::offline::Protector;
use crate::{Error, Result};

mod backend;
pub mod net;
pub mod pairing;
mod server;
pub mod tls;

use pairing::{ClaimRefusal, Collected, Limiter, Pairings};

/// The port the listener takes unless Settings says otherwise.
pub const DEFAULT_PORT: u16 = 47323;
/// The longest name a phone may give itself, in characters.
pub const MAX_NAME: usize = 40;
/// A paired phone not seen for this many days is unpaired at its next request.
pub const MAX_IDLE_DAYS: i64 = 90;
/// How often a running listener looks for changed addresses (another network, a new lease).
const RESCAN_EVERY: Duration = Duration::from_secs(30);

/// What the phone view does through the app, so a phone's decision goes through exactly the
/// code a decision in the app does, checks and guards included. `Runtime` is the one the app
/// uses.
#[async_trait]
pub trait PhoneBackend: Send + Sync {
    /// The Inbox as the app shows it (snoozed and done items left out).
    async fn inbox(&self) -> Result<Inbox>;
    /// The dots' messages that wait for the person.
    async fn waiting_messages(&self) -> Result<Vec<Message>>;
    /// `Runtime::decide_approval` without a rule: Approve once or Deny.
    async fn decide_approval(&self, id: &str, approved: bool) -> Result<Approval>;
    /// Records where `id` was decided and announces the change.
    async fn note_decided_via(&self, id: &str, via: &str) -> Result<()>;
    /// `Runtime::reply`: answers a dot's question.
    async fn reply(&self, id: &str, text: &str) -> Result<Run>;
    /// `Runtime::run_release_hold`: Run anyway on a run held for Claude usage.
    async fn run_release_hold(&self, run_id: &str) -> Result<Run>;
}

/// A phone that asks to be paired, as the PC's dialog shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairRequest {
    pub name: String,
    pub ip: IpAddr,
    /// The 4 digits the phone shows.
    pub match_code: String,
}

/// Asks the person on the PC whether a phone may pair: a dialog the phone (and the webview)
/// cannot answer. False when declined or closed.
#[async_trait]
pub trait PairApprover: Send + Sync {
    async fn approve(&self, request: PairRequest) -> bool;
}

/// The words of the PC's dialog.
pub fn pair_question(request: &PairRequest) -> (String, String) {
    (
        "Pair a phone with Sage?".to_string(),
        format!(
            "A phone on your network asks to answer your Sage Inbox.\n\n\
             Name it gives: {}\nAddress: {}\nNumber on the phone: {}\n\n\
             Allow only if you just scanned or typed the pairing code yourself and your phone \
             shows {}, and the address is your phone's (on the phone: Wi-Fi settings, this \
             network's details); another address means another device is in between. Once \
             allowed, it can approve or deny the calls your sols ask about, \
             answer their questions and start runs held for usage, until you remove it in \
             Settings > Phone.",
            request.name, request.ip, request.match_code, request.match_code
        ),
    )
}

/// Where the listener stands, for Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneStatus {
    pub running: bool,
    pub port: u16,
    /// The `https://…` addresses it listens on, the one for the QR code first.
    pub addresses: Vec<String>,
    /// SHA-256 of its certificate, as browsers show it.
    pub fingerprint: Option<String>,
    /// Why it is not listening although it is on.
    pub error: Option<String>,
    /// Whether a pairing waits for a phone now.
    pub pairing: bool,
}

/// A pairing as Settings shows it: the QR code's link and the code to type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingView {
    pub url: String,
    /// `ABCD-EFGH`.
    pub code: String,
    /// The `https://ip:port` to open before typing the code.
    pub address: String,
    pub fingerprint: String,
    pub expires_at: String,
    /// The QR code: `qr_size` modules square (quiet zone included), the dark ones as one SVG
    /// path in module units.
    pub qr_size: u32,
    pub qr_path: String,
}

struct Running {
    cancel: CancellationToken,
    port: u16,
    ips: Vec<IpAddr>,
    fingerprint: String,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        for t in &self.tasks {
            t.abort();
        }
    }
}

pub(crate) struct Inner {
    store: Store,
    backend: Arc<dyn PhoneBackend>,
    protector: Arc<dyn Protector>,
    approver: Arc<dyn PairApprover>,
    pairings: StdMutex<Pairings>,
    limiter: StdMutex<Limiter>,
    running: Mutex<Option<Running>>,
    /// The settings last applied: on, and the port.
    wanted: StdMutex<(bool, u16)>,
    error: StdMutex<Option<String>>,
    addresses: Box<dyn Fn() -> Vec<IpAddr> + Send + Sync>,
    /// The addresses found when it was turned on (or Dots started with it on): it never
    /// moves to another network by itself, only drops the addresses that went away.
    home: StdMutex<Option<Vec<IpAddr>>>,
    rescan: StdMutex<Option<JoinHandle<()>>>,
}

/// The phone view's listener and pairings. Cheap to clone.
#[derive(Clone)]
pub struct Phone {
    inner: Arc<Inner>,
}

impl Phone {
    pub fn new(
        store: Store,
        backend: Arc<dyn PhoneBackend>,
        protector: Arc<dyn Protector>,
        approver: Arc<dyn PairApprover>,
    ) -> Self {
        Self::build(
            store,
            backend,
            protector,
            approver,
            Box::new(net::lan_addresses),
        )
    }

    /// Like `new`, listening on `ips` instead of the private addresses found. For tests only,
    /// which listen on 127.0.0.1.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn with_addresses(
        store: Store,
        backend: Arc<dyn PhoneBackend>,
        protector: Arc<dyn Protector>,
        approver: Arc<dyn PairApprover>,
        ips: Vec<IpAddr>,
    ) -> Self {
        Self::build(
            store,
            backend,
            protector,
            approver,
            Box::new(move || ips.clone()),
        )
    }

    fn build(
        store: Store,
        backend: Arc<dyn PhoneBackend>,
        protector: Arc<dyn Protector>,
        approver: Arc<dyn PairApprover>,
        addresses: Box<dyn Fn() -> Vec<IpAddr> + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                store,
                backend,
                protector,
                approver,
                pairings: StdMutex::new(Pairings::default()),
                limiter: StdMutex::new(Limiter::default()),
                running: Mutex::new(None),
                wanted: StdMutex::new((false, DEFAULT_PORT)),
                error: StdMutex::new(None),
                addresses,
                home: StdMutex::new(None),
                rescan: StdMutex::new(None),
            }),
        }
    }

    /// Starts, restarts or stops the listener to match `settings`. Off closes it and every
    /// connection at once, and forgets a pairing in progress.
    pub async fn apply(&self, settings: &Settings) {
        {
            let mut wanted = self.inner.wanted.lock().unwrap();
            // Turned on (or on at start): the network the PC is on now is the one it serves.
            if settings.phone_enabled && !wanted.0 {
                *self.inner.home.lock().unwrap() = None;
            }
            *wanted = (settings.phone_enabled, settings.phone_port);
        }
        self.inner.reconcile().await;
        let mut rescan = self.inner.rescan.lock().unwrap();
        if settings.phone_enabled && rescan.is_none() {
            let weak = Arc::downgrade(&self.inner);
            *rescan = Some(tokio::spawn(rescan_loop(weak)));
        } else if !settings.phone_enabled {
            if let Some(task) = rescan.take() {
                task.abort();
            }
        }
    }

    /// Stops the listener (the app is closing).
    pub async fn stop(&self) {
        *self.inner.wanted.lock().unwrap() = (false, DEFAULT_PORT);
        if let Some(task) = self.inner.rescan.lock().unwrap().take() {
            task.abort();
        }
        self.inner.reconcile().await;
    }

    pub async fn status(&self) -> PhoneStatus {
        let running = self.inner.running.lock().await;
        let (_, wanted_port) = *self.inner.wanted.lock().unwrap();
        let pairing = self.inner.pairings.lock().unwrap().waiting(Instant::now());
        match running.as_ref() {
            Some(r) => PhoneStatus {
                running: true,
                port: r.port,
                addresses: r.ips.iter().map(|ip| origin(*ip, r.port)).collect(),
                fingerprint: Some(r.fingerprint.clone()),
                error: self.inner.error.lock().unwrap().clone(),
                pairing,
            },
            None => PhoneStatus {
                running: false,
                port: wanted_port,
                addresses: Vec::new(),
                fingerprint: None,
                error: self.inner.error.lock().unwrap().clone(),
                pairing: false,
            },
        }
    }

    /// Starts a pairing on `address` (one of the status's addresses; the first when `None`).
    pub async fn start_pairing(&self, address: Option<&str>) -> Result<PairingView> {
        let running = self.inner.running.lock().await;
        let Some(r) = running.as_ref() else {
            return Err(Error::Conflict(
                "Answer from my phone is off or not listening. Turn it on and save first.".into(),
            ));
        };
        let origins: Vec<String> = r.ips.iter().map(|ip| origin(*ip, r.port)).collect();
        let address = match address {
            None => origins.first().cloned(),
            Some(a) => origins.iter().find(|o| o.as_str() == a).cloned(),
        }
        .ok_or_else(|| Error::Invalid("that address is not one Sage listens on".into()))?;
        let now = Instant::now();
        let pairing = self.inner.pairings.lock().unwrap().start(now);
        let url = format!(
            "{address}/#t={}&fp={}",
            pairing.token,
            r.fingerprint.replace(':', "")
        );
        let (qr_size, qr_path) = qr(&url)?;
        let expires_at = timestamp(
            chrono::Utc::now()
                + chrono::TimeDelta::from_std(pairing::PAIRING_LIFETIME).unwrap_or_default(),
        );
        Ok(PairingView {
            url,
            code: pairing::show_code(&pairing.code),
            address,
            fingerprint: r.fingerprint.clone(),
            expires_at,
            qr_size,
            qr_path,
        })
    }

    /// Ends the pairing in progress, and a phone's claim the person has not answered yet.
    pub fn cancel_pairing(&self) {
        self.inner.pairings.lock().unwrap().cancel();
    }

    pub async fn devices(&self) -> Result<Vec<PhoneDevice>> {
        self.inner.store.list_phone_devices().await
    }

    /// Removes a phone: its next request is refused.
    pub async fn remove_device(&self, id: &str) -> Result<()> {
        self.inner.store.remove_phone_device(id).await
    }

    /// The newest phone actions, newest first.
    pub async fn actions(&self, limit: u32) -> Result<Vec<PhoneAction>> {
        self.inner.store.recent_phone_actions(limit.min(200)).await
    }
}

async fn rescan_loop(inner: Weak<Inner>) {
    loop {
        tokio::time::sleep(RESCAN_EVERY).await;
        let Some(inner) = inner.upgrade() else {
            return;
        };
        inner.reconcile().await;
    }
}

/// `https://ip:port`.
fn origin(ip: IpAddr, port: u16) -> String {
    format!("https://{}:{port}", net::url_host(ip))
}

/// The QR code of `text`: its size with a 4-module quiet zone, and its dark modules as one SVG
/// path.
fn qr(text: &str) -> Result<(u32, String)> {
    let code = qrcode::QrCode::with_error_correction_level(text, qrcode::EcLevel::M)
        .map_err(|e| Error::Other(format!("making the QR code failed: {e}")))?;
    let width = code.width();
    let colors = code.to_colors();
    let mut path = String::new();
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = (i % width + 4, i / width + 4);
            path.push_str(&format!("M{x} {y}h1v1h-1z"));
        }
    }
    Ok((width as u32 + 8, path))
}

/// A phone's name as stored: trimmed, "Phone" when empty, at most 40 characters, nothing that
/// would show it as something else in the PC's dialog.
pub fn clean_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok("Phone".to_string());
    }
    if name.chars().count() > MAX_NAME {
        return Err(Error::Invalid(format!(
            "a phone's name is at most {MAX_NAME} characters"
        )));
    }
    crate::settings::check_plain_text("the phone's name", name)?;
    Ok(name.to_string())
}

impl Inner {
    /// Makes the listener match what is wanted: closed when off; when on, listening on the
    /// private addresses found now, rebuilt when they or the port changed.
    async fn reconcile(self: &Arc<Self>) {
        let (enabled, port) = *self.wanted.lock().unwrap();
        let mut running = self.running.lock().await;
        if !enabled {
            if running.take().is_some() {
                tracing::info!("phone view: stopped listening");
            }
            self.pairings.lock().unwrap().clear();
            *self.error.lock().unwrap() = None;
            return;
        }
        let mut ips = (self.addresses)();
        {
            let mut home = self.home.lock().unwrap();
            match home.as_ref() {
                Some(home) => ips.retain(|ip| home.contains(ip)),
                None if !ips.is_empty() => *home = Some(ips.clone()),
                None => {}
            }
            if ips.is_empty() && home.is_some() {
                drop(home);
                if running.take().is_some() {
                    tracing::info!("phone view: the PC left its network; stopped listening");
                }
                self.pairings.lock().unwrap().clear();
                *self.error.lock().unwrap() = Some(
                    "This PC left the network it was on when this was turned on. To answer from                      your phone on this network, turn the setting off, save, then on again."
                        .into(),
                );
                return;
            }
        }
        if running
            .as_ref()
            .is_some_and(|r| r.port == port && r.ips == ips)
        {
            return;
        }
        // Stop first: the new listener may take the same port.
        if running.take().is_some() {
            self.pairings.lock().unwrap().clear();
        }
        match self.start(port, ips).await {
            Ok((r, error)) => {
                *self.error.lock().unwrap() = error;
                *running = Some(r);
            }
            Err(e) => {
                tracing::warn!("phone view: not listening: {e}");
                *self.error.lock().unwrap() = Some(e);
            }
        }
    }

    /// Binds every one of `ips` that takes the port. An error when none does; otherwise the
    /// running listener and, when some address did not bind, why.
    async fn start(
        self: &Arc<Self>,
        port: u16,
        ips: Vec<IpAddr>,
    ) -> std::result::Result<(Running, Option<String>), String> {
        if ips.is_empty() {
            return Err(
                "This PC has no private network address. Connect it to your home \
                        Wi-Fi or network, then turn the setting off and on again."
                    .to_string(),
            );
        }
        let identity = tls::load_or_make(&self.store, self.protector.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        let cancel = CancellationToken::new();
        let mut tasks = Vec::new();
        let mut bound = Vec::new();
        let mut problems = Vec::new();
        for ip in ips {
            let addr = SocketAddr::new(ip, port);
            match TcpListener::bind(addr).await {
                Ok(listener) => {
                    tracing::info!("phone view: listening on https://{addr}");
                    tasks.push(tokio::spawn(server::serve(
                        listener,
                        identity.config.clone(),
                        Arc::downgrade(self),
                        cancel.clone(),
                    )));
                    bound.push(ip);
                }
                Err(e) => problems.push(format!("could not listen on {addr}: {e}")),
            }
        }
        if bound.is_empty() {
            return Err(problems.join("; "));
        }
        let error = (!problems.is_empty()).then(|| problems.join("; "));
        Ok((
            Running {
                cancel,
                port,
                ips: bound,
                fingerprint: identity.fingerprint,
                tasks,
            },
            error,
        ))
    }

    /// The paired phone whose credential is `credential`, noted as seen from `ip`.
    async fn authenticate(&self, credential: &str, ip: IpAddr) -> Result<Option<PhoneDevice>> {
        let hash = token_hash(credential);
        let Some((device, stored)) = self.store.phone_device_by_hash(&hash).await? else {
            return Ok(None);
        };
        if !crate::util::ct_eq(&stored, &hash) {
            return Ok(None);
        }
        // A phone unused for 90 days is unpaired: a credential copied long ago stops working.
        let idle_too_long = device
            .last_seen_at
            .as_deref()
            .unwrap_or(&device.created_at)
            .parse::<chrono::DateTime<chrono::Utc>>()
            .is_ok_and(|t| chrono::Utc::now() - t > chrono::TimeDelta::days(MAX_IDLE_DAYS));
        if idle_too_long {
            self.store.remove_phone_device(&device.id).await?;
            self.record(
                &device,
                "expired",
                &device.id,
                "Unpaired after 90 days unused".into(),
            )
            .await;
            return Ok(None);
        }
        let ip_text = ip.to_string();
        let seen_lately = device
            .last_seen_at
            .as_deref()
            .and_then(|seen| chrono::DateTime::parse_from_rfc3339(seen).ok())
            .is_some_and(|t| {
                chrono::Utc::now() - t.with_timezone(&chrono::Utc) < chrono::TimeDelta::seconds(60)
            });
        if !seen_lately || device.last_ip.as_deref() != Some(ip_text.as_str()) {
            self.store
                .touch_phone_device(&device.id, &now(), &ip_text)
                .await?;
        }
        Ok(Some(device))
    }

    fn blocked(&self, ip: IpAddr) -> bool {
        self.limiter.lock().unwrap().blocked(ip, Instant::now())
    }

    fn fail(&self, ip: IpAddr) {
        self.limiter.lock().unwrap().fail(ip, Instant::now());
    }

    /// A phone presents a pairing's token or code. On success the PC asks the person, and the
    /// phone gets its claim secret and the number to compare.
    /// `name` is cleaned already (`clean_name`).
    async fn claim(
        self: &Arc<Self>,
        secret: &str,
        name: String,
        ip: IpAddr,
    ) -> std::result::Result<(String, String), ClaimRefusal> {
        let claim = self
            .pairings
            .lock()
            .unwrap()
            .claim(secret, name, ip, Instant::now())?;
        let request = PairRequest {
            name: claim.name.clone(),
            ip,
            match_code: claim.match_code.clone(),
        };
        let weak = Arc::downgrade(self);
        let secret = claim.secret.clone();
        tokio::spawn(async move {
            let approver = match weak.upgrade() {
                Some(inner) => inner.approver.clone(),
                None => return,
            };
            let allowed = approver.approve(request).await;
            if let Some(inner) = weak.upgrade() {
                inner
                    .pairings
                    .lock()
                    .unwrap()
                    .answer(&secret, allowed, Instant::now());
            }
        });
        Ok((claim.secret, claim.match_code))
    }

    /// The phone asks where its claim stands. When allowed, its credential is made and stored
    /// (hashed) now, and returned once.
    async fn collect(
        &self,
        secret: &str,
        ip: IpAddr,
    ) -> Result<Option<(Collected, Option<String>)>> {
        let collected = self
            .pairings
            .lock()
            .unwrap()
            .collect(secret, Instant::now());
        let Some(collected) = collected else {
            return Ok(None);
        };
        let Collected::Allowed { name } = &collected else {
            return Ok(Some((collected, None)));
        };
        let credential = new_token();
        let at = now();
        let device = PhoneDevice {
            id: new_id(),
            name: name.clone(),
            created_at: at.clone(),
            last_seen_at: Some(at.clone()),
            last_ip: Some(ip.to_string()),
        };
        self.store
            .add_phone_device(&device, &token_hash(&credential))
            .await?;
        self.record(&device, "paired", &device.id, format!("paired from {ip}"))
            .await;
        Ok(Some((collected, Some(credential))))
    }

    /// Adds a phone action to the audit trail; a failure is logged, the action stands.
    async fn record(&self, device: &PhoneDevice, action: &str, target: &str, summary: String) {
        let row = PhoneAction {
            device_id: device.id.clone(),
            device_name: device.name.clone(),
            action: action.to_string(),
            target: target.to_string(),
            summary,
            at: now(),
        };
        if let Err(e) = self.store.add_phone_action(&row).await {
            tracing::error!("recording a phone action failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_name("  iPhone ").unwrap(), "iPhone");
        assert_eq!(clean_name("").unwrap(), "Phone");
        assert!(clean_name(&"x".repeat(41)).is_err());
        assert!(clean_name("Pho\u{202e}ne").is_err());
        assert!(clean_name("a\nb").is_err());
    }

    #[test]
    fn the_qr_code_has_a_quiet_zone() {
        let (size, path) = qr("https://192.168.1.2:47323/#t=abc").unwrap();
        assert!(size >= 21 + 8);
        assert!(path.starts_with('M'));
        assert!(!path.contains("M0 "));
    }

    #[test]
    fn the_dialog_names_the_phone_its_address_and_number() {
        let (title, text) = pair_question(&PairRequest {
            name: "iPhone".into(),
            ip: "192.168.1.9".parse().unwrap(),
            match_code: "0421".into(),
        });
        assert_eq!(title, "Pair a phone with Sage?");
        assert!(text.contains("iPhone") && text.contains("192.168.1.9") && text.contains("0421"));
    }
}
