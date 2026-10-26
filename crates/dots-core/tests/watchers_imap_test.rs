//! The real IMAP client (watchers spec section 4.2) against a scripted local server: it logs
//! in, examines, searches and fetches with BODY.PEEK[], and sends no command that could change
//! the mailbox; a refused login is permanent and an unreachable server transient.

use std::sync::{Arc, Mutex};

use dots_core::watchers::email::{FolderState, ImapMailSource, MailSource};
use dots_core::watchers::Failure;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const MAIL: &[u8] = b"From: a@acme.com\r\nSubject: Hi\r\n\r\nHello\r\n";

/// Serves one connection, answering each command by its verb; returns the port and the
/// commands it was sent (without their tags).
async fn server(refuse_login: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let seen = lines.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut read = BufReader::new(read);
        write.write_all(b"* OK IMAP4rev1 ready\r\n").await.unwrap();
        let mut line = String::new();
        while read.read_line(&mut line).await.unwrap() > 0 {
            let text = line.trim_end().to_string();
            line.clear();
            let (tag, command) = text.split_once(' ').unwrap_or((&text, ""));
            let verb = command.to_ascii_uppercase();
            seen.lock().unwrap().push(command.to_string());
            let answer = if verb.starts_with("LOGIN") && refuse_login {
                format!("{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n")
            } else if verb.starts_with("CAPABILITY") {
                format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n")
            } else if verb.starts_with("EXAMINE") {
                format!("* 2 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n* OK [UIDNEXT 3] ok\r\n{tag} OK [READ-ONLY] done\r\n")
            } else if verb.starts_with("UID SEARCH") {
                format!("* SEARCH 1 2\r\n{tag} OK done\r\n")
            } else if verb.starts_with("UID FETCH") {
                let mut out =
                    format!("* 2 FETCH (UID 2 BODY[] {{{}}}\r\n", MAIL.len()).into_bytes();
                out.extend_from_slice(MAIL);
                out.extend_from_slice(format!(")\r\n{tag} OK done\r\n").as_bytes());
                write.write_all(&out).await.unwrap();
                continue;
            } else if verb.starts_with("LOGOUT") {
                write
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
                break;
            } else {
                format!("{tag} OK done\r\n")
            };
            write.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    (port, lines)
}

#[tokio::test]
async fn the_imap_client_only_reads_and_never_marks_anything() {
    let (port, lines) = server(false).await;
    let source = ImapMailSource::plain("127.0.0.1", port);
    let mut mailbox = source
        .connect("me@gmail.com", "app-pass-123")
        .await
        .unwrap();
    assert_eq!(
        mailbox.examine("INBOX").await.unwrap(),
        Some(FolderState {
            uid_validity: 7,
            uid_next: 3
        })
    );
    assert_eq!(mailbox.search_from(1).await.unwrap(), vec![1, 2]);
    assert_eq!(mailbox.fetch(2).await.unwrap(), MAIL);
    mailbox.logout().await;
    let sent: Vec<String> = lines
        .lock()
        .unwrap()
        .iter()
        .map(|l| l.to_ascii_uppercase())
        .collect();
    let allowed = [
        "CAPABILITY",
        "LOGIN",
        "EXAMINE",
        "UID SEARCH",
        "UID FETCH",
        "LOGOUT",
        "ID",
        "NOOP",
    ];
    for line in &sent {
        assert!(allowed.iter().any(|a| line.starts_with(a)), "{line}");
    }
    assert!(sent.iter().any(|l| l.starts_with("EXAMINE")));
    let fetch = sent.iter().find(|l| l.starts_with("UID FETCH")).unwrap();
    assert!(fetch.contains("BODY.PEEK[]"), "{fetch}");
}

#[tokio::test]
async fn a_refused_login_is_permanent_and_an_unreachable_server_transient() {
    let (port, _) = server(true).await;
    let source = ImapMailSource::plain("127.0.0.1", port);
    match source.connect("me@gmail.com", "app-pass-123").await {
        Err(Failure::Permanent(text)) => assert!(!text.contains("app-pass-123"), "{text}"),
        other => panic!("{:?}", other.map(|_| ())),
    }
    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let source = ImapMailSource::plain("127.0.0.1", port);
    assert!(matches!(
        source.connect("me@gmail.com", "x").await,
        Err(Failure::Transient(_))
    ));
}

/// Deferred minor: a server that sends no UIDNEXT: the next UID is one past the newest the
/// server has, not a message count.
#[tokio::test]
async fn without_uidnext_the_next_uid_is_one_past_the_newest() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        let mut read = BufReader::new(read);
        write.write_all(b"* OK IMAP4rev1 ready\r\n").await.unwrap();
        let mut line = String::new();
        while read.read_line(&mut line).await.unwrap() > 0 {
            let text = line.trim_end().to_string();
            line.clear();
            let (tag, command) = text.split_once(' ').unwrap_or((&text, ""));
            let verb = command.to_ascii_uppercase();
            let answer = if verb.starts_with("EXAMINE") {
                format!("* 2 EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n{tag} OK [READ-ONLY] done\r\n")
            } else if verb.starts_with("UID SEARCH") {
                format!("* SEARCH 40 90\r\n{tag} OK done\r\n")
            } else if verb.starts_with("LOGOUT") {
                write
                    .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
                break;
            } else {
                format!("{tag} OK done\r\n")
            };
            write.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    let source = ImapMailSource::plain("127.0.0.1", port);
    let mut mailbox = source
        .connect("me@gmail.com", "app-pass-123")
        .await
        .unwrap();
    assert_eq!(
        mailbox.examine("INBOX").await.unwrap(),
        Some(FolderState {
            uid_validity: 7,
            uid_next: 91
        })
    );
    mailbox.logout().await;
}
