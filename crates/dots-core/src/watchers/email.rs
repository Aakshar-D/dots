//! Email watchers (watchers spec section 4.2): a Gmail label read over IMAP once a minute,
//! read-only. Only `LOGIN`, `EXAMINE`, `UID SEARCH`, `UID FETCH … BODY.PEEK[]` and `LOGOUT` are
//! ever sent: nothing is marked read, moved, deleted or sent. The cursor keeps the label's
//! `UIDVALIDITY` and the last UID handled; a new watcher, or a new `UIDVALIDITY`, starts from
//! the newest mail.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll as TaskPoll};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use chrono::{DateTime, SecondsFormat, Utc};
use futures_util::TryStreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

use super::files::{safe_name, unique_names, MAX_ATTACHMENTS_BYTES};
use super::{entry_gone, Failure, Fired, Poll, SourceEvent, Watchers, MAX_BODY_CHARS, POLL_EVERY};
use crate::model::{EmailConfig, Watcher};
use crate::Result;

/// How long a connection (TCP, TLS, greeting) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

pub fn login_refused(address: &str) -> String {
    format!("Gmail refused the app password for {address}, or IMAP is off for it.")
}

pub fn no_label(label: &str) -> String {
    format!("Gmail has no label {label}.")
}

pub fn unreachable(error: &str) -> String {
    format!("Could not reach Gmail: {error}.")
}

/// A label's state when examined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderState {
    pub uid_validity: u32,
    pub uid_next: u32,
}

/// Where a mailbox is reached; tests give a fake.
#[async_trait]
pub trait MailSource: Send + Sync {
    async fn connect(
        &self,
        address: &str,
        password: &str,
    ) -> std::result::Result<Box<dyn Mailbox>, Failure>;
}

/// A logged-in mailbox: read commands only.
#[async_trait]
pub trait Mailbox: Send {
    /// `None` when the server has no such folder.
    async fn examine(&mut self, folder: &str) -> std::result::Result<Option<FolderState>, Failure>;
    /// `UID SEARCH UID <first>:*` as the server answers it, ascending.
    async fn search_from(&mut self, first_uid: u32) -> std::result::Result<Vec<u32>, Failure>;
    /// `UID FETCH <uid> BODY.PEEK[]`: the whole message, without marking it read.
    async fn fetch(&mut self, uid: u32) -> std::result::Result<Vec<u8>, Failure>;
    async fn logout(&mut self);
}

/// A label's IMAP name (RFC 3501 modified UTF-7): printable ASCII as it is, `&` as `&-`, each
/// run of other characters as `&`, its UTF-16BE in base64 without padding (`/` as `,`), `-`.
pub fn imap_folder_name(label: &str) -> String {
    fn flush(out: &mut String, run: &mut Vec<u16>) {
        if run.is_empty() {
            return;
        }
        let bytes: Vec<u8> = run.iter().flat_map(|u| u.to_be_bytes()).collect();
        let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes);
        out.push('&');
        out.push_str(&encoded.replace('/', ","));
        out.push('-');
        run.clear();
    }
    let mut out = String::new();
    let mut run: Vec<u16> = Vec::new();
    for c in label.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut out, &mut run);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            let mut units = [0u16; 2];
            run.extend_from_slice(c.encode_utf16(&mut units));
        }
    }
    flush(&mut out, &mut run);
    out
}

/// Where an email watcher has read up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailCursor {
    pub uid_validity: u32,
    pub last_uid: u32,
}

/// A parsed email.
#[derive(Debug, Clone, PartialEq)]
pub struct Mail {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub date: String,
    pub message_id: String,
    pub body: String,
    pub attachments: Vec<(String, Vec<u8>)>,
}

/// The email in `bytes`, or `None` when it cannot be read as one.
pub fn parse_mail(bytes: &[u8]) -> Option<Mail> {
    let message = MessageParser::default().parse(bytes)?;
    let from = message
        .from()
        .and_then(|a| a.first())
        .and_then(|a| a.address())
        .unwrap_or_default()
        .to_string();
    let to = message
        .to()
        .map(|a| {
            a.iter()
                .filter_map(|a| a.address())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let date = message
        .date()
        .and_then(|d| DateTime::<Utc>::from_timestamp(d.to_timestamp(), 0))
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_default();
    let message_id = message
        .message_id()
        .unwrap_or_default()
        .trim_matches(['<', '>'])
        .to_string();
    let body = message
        .body_text(0)
        .map(|b| b.into_owned())
        .unwrap_or_default();
    let attachments = message
        .attachments()
        .map(|part| {
            (
                part.attachment_name().unwrap_or("attachment").to_string(),
                part.contents().to_vec(),
            )
        })
        .collect();
    Some(Mail {
        from,
        to,
        subject: message.subject().unwrap_or_default().to_string(),
        date,
        message_id,
        body,
        attachments,
    })
}

/// Whether the mail passes every filter the watcher sets.
pub fn matches(config: &EmailConfig, mail: &Mail) -> bool {
    let from = mail.from.to_lowercase();
    if let Some(want) = &config.from {
        let want = want.to_lowercase();
        let ok = if want.starts_with('@') {
            from.ends_with(&want)
        } else {
            from == want
        };
        if !ok {
            return false;
        }
    }
    if let Some(subject) = &config.subject {
        if !mail
            .subject
            .to_lowercase()
            .contains(&subject.to_lowercase())
        {
            return false;
        }
    }
    if let Some(attached) = config.has_attachment {
        if attached == mail.attachments.is_empty() {
            return false;
        }
    }
    true
}

/// The run's event for the mail (spec section 4.2): the body cut to `MAX_BODY_CHARS`, the
/// attachments with safe unique names, staged while they fit in `MAX_ATTACHMENTS_BYTES`.
pub fn to_event(mail: Mail) -> SourceEvent {
    let names = unique_names(
        &mail
            .attachments
            .iter()
            .map(|(n, _)| safe_name(n))
            .collect::<Vec<_>>(),
    );
    let mut total = 0usize;
    let mut listed = Vec::new();
    let mut files = Vec::new();
    for (name, (_, bytes)) in names.into_iter().zip(mail.attachments) {
        let size = bytes.len();
        if total + size <= MAX_ATTACHMENTS_BYTES {
            total += size;
            listed.push(json!({"name": name, "size": size, "path": null}));
            files.push((name, bytes));
        } else {
            listed.push(json!({"name": name, "size": size, "skipped": "too large"}));
        }
    }
    let body: String = mail.body.chars().take(MAX_BODY_CHARS).collect();
    SourceEvent {
        payload: json!({
            "source": "email",
            "from": mail.from,
            "to": mail.to,
            "subject": mail.subject,
            "date": mail.date,
            "message_id": mail.message_id,
            "body": body,
            "attachments": listed,
        }),
        files,
    }
}

/// One check of an email watcher (spec section 4.2).
pub(crate) async fn poll(
    service: &Watchers,
    watcher: &Watcher,
    config: &EmailConfig,
) -> Result<Poll> {
    let Some(password) = service.deps.vault.personal_value(&config.secret).await? else {
        let failure = Failure::Permanent(entry_gone(&config.secret));
        return service.note_failure(&watcher.id, failure).await;
    };
    let mut mailbox = match service
        .deps
        .sources
        .mail
        .connect(&config.address, &password)
        .await
    {
        Ok(m) => m,
        Err(failure) => return service.note_failure(&watcher.id, failure).await,
    };
    let outcome = read_new(service, watcher, config, mailbox.as_mut()).await;
    mailbox.logout().await;
    match outcome {
        Err(e) => Err(e),
        Ok(Some(failure)) => service.note_failure(&watcher.id, failure).await,
        Ok(None) => {
            service.note_success(&watcher.id).await?;
            Ok(Poll::Next(service.deps.clock.now() + POLL_EVERY))
        }
    }
}

/// Handles the mail past the cursor; a `Failure` is the source's, an `Err` the store's.
async fn read_new(
    service: &Watchers,
    watcher: &Watcher,
    config: &EmailConfig,
    mailbox: &mut dyn Mailbox,
) -> Result<Option<Failure>> {
    let store = &service.deps.store;
    let state = match mailbox.examine(&imap_folder_name(&config.label)).await {
        Ok(Some(state)) => state,
        Ok(None) => return Ok(Some(Failure::Permanent(no_label(&config.label)))),
        Err(failure) => return Ok(Some(failure)),
    };
    let saved: Option<EmailCursor> = store
        .get_watcher_cursor(&watcher.id)
        .await?
        .and_then(|v| serde_json::from_value(v).ok());
    let mut cursor = match saved {
        Some(c) if c.uid_validity == state.uid_validity => c,
        _ => {
            // A new watcher, or a mailbox renumbered: start from the newest mail.
            let fresh = EmailCursor {
                uid_validity: state.uid_validity,
                last_uid: state.uid_next.saturating_sub(1),
            };
            store
                .set_watcher_cursor(&watcher.id, Some(&serde_json::to_value(fresh)?))
                .await?;
            return Ok(None);
        }
    };
    let uids = match mailbox.search_from(cursor.last_uid + 1).await {
        Ok(uids) => uids,
        Err(failure) => return Ok(Some(failure)),
    };
    let handled = cursor.last_uid;
    for uid in uids.into_iter().filter(|u| *u > handled) {
        let bytes = match mailbox.fetch(uid).await {
            Ok(b) => b,
            Err(failure) => return Ok(Some(failure)),
        };
        match parse_mail(&bytes) {
            None => tracing::warn!(watcher = %watcher.id, uid, "an email could not be read"),
            Some(mail) if matches(config, &mail) => {
                match service.fire(&watcher.id, to_event(mail)).await? {
                    Fired::Run(_) => {}
                    Fired::Capped | Fired::Off => return Ok(None),
                }
            }
            Some(_) => {}
        }
        cursor.last_uid = uid;
        store
            .set_watcher_cursor(&watcher.id, Some(&serde_json::to_value(cursor)?))
            .await?;
    }
    Ok(None)
}

/// A plain or TLS connection, for the IMAP client.
enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stream::Plain(_) => f.write_str("Stream::Plain"),
            Stream::Tls(_) => f.write_str("Stream::Tls"),
        }
    }
}

impl AsyncRead for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> TaskPoll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> TaskPoll<std::io::Result<usize>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> TaskPoll<std::io::Result<()>> {
        match self.get_mut() {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// The real IMAP client: Gmail over TLS, or (tests) a plain local server.
#[derive(Debug, Clone)]
pub struct ImapMailSource {
    host: String,
    port: u16,
    tls: bool,
}

impl ImapMailSource {
    pub fn gmail() -> ImapMailSource {
        ImapMailSource {
            host: "imap.gmail.com".into(),
            port: 993,
            tls: true,
        }
    }

    #[doc(hidden)]
    pub fn plain(host: &str, port: u16) -> ImapMailSource {
        ImapMailSource {
            host: host.into(),
            port,
            tls: false,
        }
    }

    async fn open(&self) -> std::result::Result<Stream, String> {
        let tcp = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|e| e.to_string())?;
        if !self.tls {
            return Ok(Stream::Plain(tcp));
        }
        let mut roots = tokio_rustls::rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
        let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = tokio_rustls::rustls::pki_types::ServerName::try_from(self.host.clone())
            .map_err(|e| e.to_string())?;
        let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Stream::Tls(Box::new(tls)))
    }
}

fn transient(e: impl std::fmt::Display) -> Failure {
    Failure::Transient(unreachable(&e.to_string()))
}

#[async_trait]
impl MailSource for ImapMailSource {
    async fn connect(
        &self,
        address: &str,
        password: &str,
    ) -> std::result::Result<Box<dyn Mailbox>, Failure> {
        let opened = tokio::time::timeout(CONNECT_TIMEOUT, async {
            let stream = self.open().await?;
            let mut client = async_imap::Client::new(stream);
            match client.read_response().await {
                Some(Ok(_)) => Ok(client),
                Some(Err(e)) => Err(e.to_string()),
                None => Err("the server closed the connection".to_string()),
            }
        })
        .await;
        let client = match opened {
            Err(_) => return Err(transient("timed out")),
            Ok(Err(e)) => return Err(transient(e)),
            Ok(Ok(client)) => client,
        };
        let login = tokio::time::timeout(COMMAND_TIMEOUT, client.login(address, password)).await;
        match login {
            Err(_) => Err(transient("timed out")),
            Ok(Ok(session)) => Ok(Box::new(ImapMailbox { session })),
            Ok(Err((async_imap::error::Error::No(_), _)))
            | Ok(Err((async_imap::error::Error::Bad(_), _))) => {
                Err(Failure::Permanent(login_refused(address)))
            }
            Ok(Err((e, _))) => Err(transient(e)),
        }
    }
}

struct ImapMailbox {
    session: async_imap::Session<Stream>,
}

#[async_trait]
impl Mailbox for ImapMailbox {
    async fn examine(&mut self, folder: &str) -> std::result::Result<Option<FolderState>, Failure> {
        match tokio::time::timeout(COMMAND_TIMEOUT, self.session.examine(folder)).await {
            Err(_) => Err(transient("timed out")),
            Ok(Ok(mailbox)) => Ok(Some(FolderState {
                uid_validity: mailbox.uid_validity.unwrap_or(0),
                uid_next: mailbox.uid_next.unwrap_or(mailbox.exists + 1),
            })),
            Ok(Err(async_imap::error::Error::No(_))) => Ok(None),
            Ok(Err(e)) => Err(transient(e)),
        }
    }

    async fn search_from(&mut self, first_uid: u32) -> std::result::Result<Vec<u32>, Failure> {
        let query = format!("UID {first_uid}:*");
        match tokio::time::timeout(COMMAND_TIMEOUT, self.session.uid_search(query)).await {
            Err(_) => Err(transient("timed out")),
            Ok(Err(e)) => Err(transient(e)),
            Ok(Ok(found)) => {
                let mut uids: Vec<u32> = found.into_iter().collect();
                uids.sort_unstable();
                Ok(uids)
            }
        }
    }

    async fn fetch(&mut self, uid: u32) -> std::result::Result<Vec<u8>, Failure> {
        let fetched = tokio::time::timeout(COMMAND_TIMEOUT, async {
            let stream = self
                .session
                .uid_fetch(uid.to_string(), "BODY.PEEK[]")
                .await?;
            stream.try_collect::<Vec<_>>().await
        })
        .await;
        match fetched {
            Err(_) => Err(transient("timed out")),
            Ok(Err(e)) => Err(transient(e)),
            Ok(Ok(fetches)) => Ok(fetches
                .iter()
                .find_map(|f| f.body())
                .map(<[u8]>::to_vec)
                .unwrap_or_default()),
        }
    }

    async fn logout(&mut self) {
        let _ = tokio::time::timeout(COMMAND_TIMEOUT, self.session.logout()).await;
    }
}
