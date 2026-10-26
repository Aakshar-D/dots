//! Email watchers (watchers spec section 4.2) on a fake Gmail: starting from the newest mail,
//! one run per matching mail oldest first, the filters, the cursor and UIDVALIDITY, catching up
//! after a restart, IMAP's quirks (Review Focus 3 and 4), the body and the attachments, the cap,
//! the failures, and never a command that writes.

mod common;

use chrono::TimeDelta;
use common::fake_mail::{mail, FakeMail, MailSpec};
use common::watchers::{t0, Harness};
use dots_core::model::{Dot, EmailConfig, PausedReason, Watcher, WatcherConfig};
use dots_core::watchers::email::{imap_folder_name, login_refused, no_label};
use dots_core::watchers::{entry_gone, Failure, Poll, Sources};
use serde_json::json;

const PASS: &str = "app-pass-1234567890";
const WRITES: [&str; 7] = [
    "SELECT", "STORE", "COPY", "MOVE", "EXPUNGE", "APPEND", "DELETE",
];

async fn setup(f: impl FnOnce(&mut EmailConfig)) -> (Harness, FakeMail, Dot, Watcher) {
    let fake = FakeMail::new(PASS);
    let sources = Sources {
        mail: std::sync::Arc::new(fake.clone()),
        ..Sources::default()
    };
    let h = Harness::with_sources(sources).await;
    h.secret("GMAIL_APP_PASSWORD", PASS).await;
    let dot = h.dot("billing").await;
    let mut config = EmailConfig::new("me@gmail.com", "GMAIL_APP_PASSWORD");
    f(&mut config);
    let w = h
        .watcher(&dot, "Billing mail", WatcherConfig::Email(config))
        .await;
    (h, fake, dot, w)
}

fn invoice(n: u32) -> Vec<u8> {
    mail(MailSpec::new(
        &format!("{n}@acme.com"),
        "Billing <billing@acme.com>",
        &format!("Invoice {n}"),
    ))
}

#[tokio::test]
async fn a_new_watcher_starts_from_the_newest_mail_then_runs_once_per_mail_oldest_first() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    for n in 1..=3 {
        fake.deliver("INBOX", invoice(n));
    }
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap(),
        Some(json!({"uid_validity": 1, "last_uid": 3}))
    );
    fake.deliver("INBOX", invoice(4));
    fake.deliver("INBOX", invoice(5));
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(
        runs[0].payload,
        Some(
            json!({"source": "email", "from": "billing@acme.com", "to": "me@gmail.com",
            "subject": "Invoice 4", "date": "2026-10-23T09:00:00Z", "message_id": "4@acme.com",
            "body": "Please find the invoice attached.", "attachments": [],
            "watcher_id": w.id, "watcher_name": "Billing mail"})
        )
    );
    assert_eq!(runs[1].payload.as_ref().unwrap()["subject"], "Invoice 5");
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap(),
        Some(json!({"uid_validity": 1, "last_uid": 5}))
    );
    assert_eq!(
        fake.commands(),
        [
            "LOGIN me@gmail.com",
            "EXAMINE INBOX",
            "LOGOUT",
            "LOGIN me@gmail.com",
            "EXAMINE INBOX",
            "UID SEARCH UID 4:*",
            "UID FETCH 4 BODY.PEEK[]",
            "UID FETCH 5 BODY.PEEK[]",
            "LOGOUT"
        ]
    );
    // Nothing new: nothing more.
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 2);
    assert!(fake
        .commands()
        .iter()
        .all(|c| !WRITES.iter().any(|w| c.starts_with(w))));
}

#[tokio::test]
async fn every_filter_must_match() {
    let (h, fake, dot, w) = setup(|c| {
        c.from = Some("@acme.com".into());
        c.subject = Some("invoice".into());
        c.has_attachment = Some(true);
    })
    .await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let with = |id: &str, from: &str, subject: &str, attached: bool| {
        let mut spec = MailSpec::new(id, from, subject);
        if attached {
            spec.attachments.push(("a.pdf", b"%PDF".to_vec()));
        }
        mail(spec)
    };
    fake.deliver("INBOX", with("1@x", "x@other.com", "Invoice 1", true));
    fake.deliver("INBOX", with("2@x", "x@acme.com", "Hello", true));
    fake.deliver("INBOX", with("3@x", "x@acme.com", "Invoice 3", false));
    let last = fake.deliver(
        "INBOX",
        with("4@x", "Accounts <X@ACME.COM>", "your INVOICE 4", true),
    );
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].payload.as_ref().unwrap()["subject"],
        "your INVOICE 4"
    );
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap().unwrap()["last_uid"],
        last
    );
}

#[tokio::test]
async fn a_new_uidvalidity_starts_again_from_the_newest_mail() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    fake.deliver("INBOX", invoice(1));
    h.watchers.poll_now(&w.id).await.unwrap();
    fake.set_uid_validity("INBOX", 9);
    fake.deliver("INBOX", invoice(2));
    h.watchers.poll_now(&w.id).await.unwrap();
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap(),
        Some(json!({"uid_validity": 9, "last_uid": 2}))
    );
}

#[tokio::test]
async fn mail_that_came_while_dots_was_closed_is_handled_at_start() {
    let (mut h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    h.restart();
    fake.deliver("INBOX", invoice(1));
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}

#[tokio::test]
async fn a_search_past_the_newest_uid_fires_nothing_and_gaps_are_skipped() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    for n in 1..=3 {
        fake.deliver("INBOX", invoice(n));
    }
    h.watchers.poll_now(&w.id).await.unwrap();
    fake.clear_commands();
    // `UID 4:*` names UID 3, the newest, which was handled: no fetch, no run.
    h.watchers.poll_now(&w.id).await.unwrap();
    assert!(h.runs(&dot.id).await.is_empty());
    assert!(!fake.commands().iter().any(|c| c.starts_with("UID FETCH")));
    fake.deliver_as("INBOX", 9000, invoice(9000));
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap().unwrap()["last_uid"],
        9000
    );
    assert_eq!(
        fake.commands()
            .iter()
            .filter(|c| c.starts_with("UID FETCH"))
            .count(),
        1
    );
}

#[tokio::test]
async fn the_body_is_plain_text_at_most_20000_characters() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let mut html = MailSpec::new("h@x", "a@acme.com", "Html");
    html.body = "<p>Hello <b>you</b></p>";
    html.html = true;
    fake.deliver("INBOX", mail(html));
    let long = "x".repeat(25_000);
    let mut big = MailSpec::new("l@x", "a@acme.com", "Long");
    big.body = &long;
    fake.deliver("INBOX", mail(big));
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    let body = |i: usize| {
        runs[i].payload.as_ref().unwrap()["body"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(body(0).contains("Hello you"), "{}", body(0));
    assert!(!body(0).contains("<b>"));
    assert_eq!(body(1).chars().count(), 20_000);
}

#[tokio::test]
async fn attachments_are_staged_up_to_25_mb_in_all_with_safe_names() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    let mb = 1024 * 1024;
    let mut spec = MailSpec::new("a@x", "a@acme.com", "Files");
    spec.attachments = vec![
        ("../../evil.pdf", vec![1; 10]),
        ("CON.txt", vec![2; 3]),
        ("a.pdf", vec![3; 4]),
        ("A.PDF", vec![4; 5]),
        ("one.bin", vec![5; 20 * mb]),
        ("two.bin", vec![6; 10 * mb]),
    ];
    fake.deliver("INBOX", mail(spec));
    h.watchers.poll_now(&w.id).await.unwrap();
    let run = &h.runs(&dot.id).await[0];
    assert_eq!(
        run.payload.as_ref().unwrap()["attachments"],
        json!([
            {"name": "evil.pdf", "size": 10, "path": null},
            {"name": "_CON.txt", "size": 3, "path": null},
            {"name": "a.pdf", "size": 4, "path": null},
            {"name": "A (2).PDF", "size": 5, "path": null},
            {"name": "one.bin", "size": 20 * mb, "path": null},
            {"name": "two.bin", "size": 10 * mb, "skipped": "too large"},
        ])
    );
    let staged = h.watchers.files().stage_dir(&w.id, &run.id);
    for name in ["evil.pdf", "_CON.txt", "a.pdf", "A (2).PDF", "one.bin"] {
        assert!(staged.join(name).is_file(), "{name}");
    }
    assert!(!staged.join("two.bin").exists());
    assert_eq!(std::fs::read(staged.join("evil.pdf")).unwrap(), vec![1; 10]);
}

#[tokio::test]
async fn the_cap_stops_at_the_first_dropped_mail_and_a_resume_continues_from_it() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    for n in 1..=22 {
        fake.deliver("INBOX", invoice(n));
    }
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 20);
    assert_eq!(
        h.store.get_watcher(&w.id).await.unwrap().paused_reason,
        Some(PausedReason::Flood)
    );
    assert_eq!(
        h.store.get_watcher_cursor(&w.id).await.unwrap().unwrap()["last_uid"],
        20
    );
    h.watchers.resume(&w.id).await.unwrap();
    h.watchers.poll_now(&w.id).await.unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 22);
    assert_eq!(runs[20].payload.as_ref().unwrap()["subject"], "Invoice 21");
}

#[tokio::test]
async fn a_refused_password_pauses_at_once_and_an_unreachable_server_backs_off() {
    let (mut h, fake, _dot, w) = setup(|_| {}).await;
    fake.fail_next(Failure::Transient(
        "Could not reach Gmail: timed out.".into(),
    ));
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(
        (now.paused_reason, now.last_error.as_deref()),
        (None, Some("Could not reach Gmail: timed out."))
    );
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.store.get_watcher(&w.id).await.unwrap().last_error, None);

    h.vault
        .set_value("GMAIL_APP_PASSWORD", "wrong-password-123")
        .await
        .unwrap();
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Stopped);
    let refused = login_refused("me@gmail.com");
    assert_eq!(
        h.store
            .get_watcher(&w.id)
            .await
            .unwrap()
            .last_error
            .as_deref(),
        Some(refused.as_str())
    );
    assert_eq!(h.paused(), vec![(w.id.clone(), Some(refused))]);
}

#[tokio::test]
async fn a_vault_entry_that_is_gone_pauses_the_watcher() {
    let (h, _fake, _dot, w) = setup(|_| {}).await;
    h.vault.remove("GMAIL_APP_PASSWORD").await.unwrap();
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Stopped);
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(now.paused_reason, Some(PausedReason::Error));
    assert_eq!(now.last_error, Some(entry_gone("GMAIL_APP_PASSWORD")));
}

#[tokio::test]
async fn a_label_with_spaces_and_accents_is_examined_by_its_imap_name() {
    let (h, fake, dot, w) = setup(|c| c.label = "Dots Été".into()).await;
    fake.folder("Dots &AMk-t&AOk-", 4);
    h.watchers.poll_now(&w.id).await.unwrap();
    fake.deliver("Dots &AMk-t&AOk-", invoice(1));
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert!(fake
        .commands()
        .contains(&"EXAMINE Dots &AMk-t&AOk-".to_string()));

    let (h, _fake, _dot, w) = setup(|c| c.label = "Nope".into()).await;
    assert_eq!(h.watchers.poll_now(&w.id).await.unwrap(), Poll::Stopped);
    assert_eq!(
        h.store.get_watcher(&w.id).await.unwrap().last_error,
        Some(no_label("Nope"))
    );
}

#[test]
fn imap_names_use_modified_utf7() {
    assert_eq!(imap_folder_name("INBOX"), "INBOX");
    assert_eq!(imap_folder_name("[Gmail]/Sent Mail"), "[Gmail]/Sent Mail");
    assert_eq!(imap_folder_name("Dots Été"), "Dots &AMk-t&AOk-");
    assert_eq!(imap_folder_name("日本"), "&ZeVnLA-");
    assert_eq!(imap_folder_name("a&b"), "a&-b");
}

/// The final review: a failure of Dots' own (here the attachments cannot be staged) is retried
/// with the backoff, as a source's would be; the poll does not end and the mail is not skipped.
#[tokio::test]
async fn a_failure_of_dots_own_backs_off_and_the_mail_is_tried_again() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    std::fs::create_dir_all(h.dir.path().join("data")).unwrap();
    std::fs::write(h.dir.path().join("data").join("watchers"), b"in the way").unwrap();
    let mut spec = MailSpec::new("a@x", "a@acme.com", "Files");
    spec.attachments = vec![("a.pdf", b"%PDF".to_vec())];
    fake.deliver("INBOX", mail(spec));
    assert_eq!(
        h.watchers.poll_now(&w.id).await.unwrap(),
        Poll::Next(t0() + TimeDelta::minutes(1))
    );
    assert!(h
        .store
        .get_watcher(&w.id)
        .await
        .unwrap()
        .last_error
        .is_some());
    assert!(h.runs(&dot.id).await.is_empty());
    std::fs::remove_file(h.dir.path().join("data").join("watchers")).unwrap();
    h.watchers.poll_now(&w.id).await.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}

/// Deferred minor: two checks of one watcher never run at once, so a mail is never fired
/// twice.
#[tokio::test]
async fn two_checks_at_once_fire_a_mail_once() {
    let (h, fake, dot, w) = setup(|_| {}).await;
    h.watchers.poll_now(&w.id).await.unwrap();
    fake.deliver("INBOX", invoice(1));
    let (a, b) = tokio::join!(h.watchers.poll_now(&w.id), h.watchers.poll_now(&w.id));
    a.unwrap();
    b.unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}
