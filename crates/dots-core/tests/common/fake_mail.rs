//! A Gmail mailbox in memory for email watchers (watchers spec section 4.2). It records each
//! command as the IMAP client would send it, so a test sees that nothing is ever written, and it
//! answers `UID n:*` past the newest mail with the newest one, as IMAP servers do.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use dots_core::watchers::email::{login_refused, FolderState, MailSource, Mailbox};
use dots_core::watchers::Failure;

#[derive(Default)]
struct Folder {
    uid_validity: u32,
    mails: BTreeMap<u32, Vec<u8>>,
}

#[derive(Default)]
struct State {
    password: String,
    folders: BTreeMap<String, Folder>,
    commands: Vec<String>,
    fail: Option<Failure>,
}

#[derive(Clone)]
pub struct FakeMail {
    state: Arc<Mutex<State>>,
}

impl FakeMail {
    /// A mailbox whose app password is `password`, with an empty INBOX (UIDVALIDITY 1).
    pub fn new(password: &str) -> FakeMail {
        let state = State {
            password: password.into(),
            ..State::default()
        };
        let fake = FakeMail {
            state: Arc::new(Mutex::new(state)),
        };
        fake.folder("INBOX", 1);
        fake
    }

    /// Makes the folder `name` (its IMAP name) with `uid_validity`.
    pub fn folder(&self, name: &str, uid_validity: u32) {
        let folder = Folder {
            uid_validity,
            mails: BTreeMap::new(),
        };
        self.state
            .lock()
            .unwrap()
            .folders
            .insert(name.into(), folder);
    }

    /// Puts `mail` into `folder` under the next UID, and returns the UID.
    pub fn deliver(&self, folder: &str, mail: Vec<u8>) -> u32 {
        let mut s = self.state.lock().unwrap();
        let f = s.folders.get_mut(folder).expect("a folder");
        let uid = f.mails.keys().last().copied().unwrap_or(0) + 1;
        f.mails.insert(uid, mail);
        uid
    }

    pub fn deliver_as(&self, folder: &str, uid: u32, mail: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .folders
            .get_mut(folder)
            .unwrap()
            .mails
            .insert(uid, mail);
    }

    pub fn set_uid_validity(&self, folder: &str, uid_validity: u32) {
        self.state
            .lock()
            .unwrap()
            .folders
            .get_mut(folder)
            .unwrap()
            .uid_validity = uid_validity;
    }

    /// The next connect fails with `failure`.
    pub fn fail_next(&self, failure: Failure) {
        self.state.lock().unwrap().fail = Some(failure);
    }

    pub fn commands(&self) -> Vec<String> {
        self.state.lock().unwrap().commands.clone()
    }

    pub fn clear_commands(&self) {
        self.state.lock().unwrap().commands.clear();
    }
}

#[async_trait]
impl MailSource for FakeMail {
    async fn connect(&self, address: &str, password: &str) -> Result<Box<dyn Mailbox>, Failure> {
        let mut s = self.state.lock().unwrap();
        s.commands.push(format!("LOGIN {address}"));
        if let Some(failure) = s.fail.take() {
            return Err(failure);
        }
        if password != s.password {
            return Err(Failure::Permanent(login_refused(address)));
        }
        Ok(Box::new(Session {
            state: self.state.clone(),
            folder: String::new(),
        }))
    }
}

struct Session {
    state: Arc<Mutex<State>>,
    folder: String,
}

#[async_trait]
impl Mailbox for Session {
    async fn examine(&mut self, folder: &str) -> Result<Option<FolderState>, Failure> {
        let mut s = self.state.lock().unwrap();
        s.commands.push(format!("EXAMINE {folder}"));
        self.folder = folder.into();
        Ok(s.folders.get(folder).map(|f| FolderState {
            uid_validity: f.uid_validity,
            uid_next: f.mails.keys().last().copied().unwrap_or(0) + 1,
        }))
    }

    async fn search_from(&mut self, first_uid: u32) -> Result<Vec<u32>, Failure> {
        let mut s = self.state.lock().unwrap();
        s.commands.push(format!("UID SEARCH UID {first_uid}:*"));
        let f = &s.folders[&self.folder];
        let mut uids: Vec<u32> = f
            .mails
            .keys()
            .copied()
            .filter(|u| *u >= first_uid)
            .collect();
        if uids.is_empty() {
            uids.extend(f.mails.keys().last().copied());
        }
        Ok(uids)
    }

    async fn fetch(&mut self, uid: u32) -> Result<Vec<u8>, Failure> {
        let mut s = self.state.lock().unwrap();
        s.commands.push(format!("UID FETCH {uid} BODY.PEEK[]"));
        Ok(s.folders[&self.folder].mails[&uid].clone())
    }

    async fn logout(&mut self) {
        self.state.lock().unwrap().commands.push("LOGOUT".into());
    }
}

/// A mail as a test writes it.
pub struct MailSpec<'a> {
    pub id: &'a str,
    pub from: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    /// The body is HTML, with no plain part.
    pub html: bool,
    pub attachments: Vec<(&'a str, Vec<u8>)>,
}

impl<'a> MailSpec<'a> {
    pub fn new(id: &'a str, from: &'a str, subject: &'a str) -> MailSpec<'a> {
        MailSpec {
            id,
            from,
            subject,
            body: "Please find the invoice attached.",
            html: false,
            attachments: Vec::new(),
        }
    }
}

/// The RFC 5322 bytes of `spec`, to `me@gmail.com`, dated Friday 23 October 2026 09:00 UTC.
pub fn mail(spec: MailSpec) -> Vec<u8> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let kind = if spec.html { "text/html" } else { "text/plain" };
    let mut out = format!(
        "From: {}\r\nTo: me@gmail.com\r\nSubject: {}\r\nDate: Fri, 23 Oct 2026 09:00:00 +0000\r\n\
         Message-ID: <{}>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b0\"\r\n\r\n\
         --b0\r\nContent-Type: {kind}; charset=utf-8\r\n\r\n{}\r\n",
        spec.from, spec.subject, spec.id, spec.body
    );
    for (name, bytes) in &spec.attachments {
        out.push_str(&format!(
            "--b0\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\
             Content-Disposition: attachment; filename=\"{name}\"\r\n\r\n{}\r\n",
            b64.encode(bytes)
        ));
    }
    out.push_str("--b0--\r\n");
    out.into_bytes()
}
