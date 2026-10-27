use std::io::Cursor;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use crypto_box::aead::OsRng;
use dots_core::publish::keys::{make_key, pubkey_in_conf, seal, with_pubkey, PROTOTYPE_PUBKEY};
use minisign::{PublicKeyBox, SecretKeyBox};

const PASSWORD: &str = "correct horse battery";

fn text_of(b64: &str) -> String {
    String::from_utf8(STANDARD.decode(b64).unwrap()).unwrap()
}

#[test]
fn a_made_key_signs_what_its_public_key_verifies() {
    let made = make_key(PASSWORD).unwrap();
    let sk = SecretKeyBox::from_string(&text_of(&made.private_b64))
        .unwrap()
        .into_secret_key(Some(PASSWORD.to_string()))
        .unwrap();
    let pk = PublicKeyBox::from_string(&text_of(&made.public_b64))
        .unwrap()
        .into_public_key()
        .unwrap();
    let data = b"Dots_0.3.0_x64-setup.exe";
    let signature = minisign::sign(Some(&pk), &sk, Cursor::new(data), None, None).unwrap();
    minisign::verify(&pk, &signature, Cursor::new(data), true, false, false).unwrap();
    // The wrong password cannot open it.
    assert!(SecretKeyBox::from_string(&text_of(&made.private_b64))
        .unwrap()
        .into_secret_key(Some("not the password".into()))
        .is_err());
}

#[test]
fn a_short_password_is_refused() {
    let err = make_key("elevenchars").unwrap_err().to_string();
    assert!(
        err.contains("The password needs at least 12 characters."),
        "{err}"
    );
}

#[test]
fn a_sealed_secret_opens_with_the_matching_secret_key() {
    let secret = crypto_box::SecretKey::generate(&mut OsRng);
    let public_b64 = STANDARD.encode(secret.public_key().as_bytes());
    let sealed = seal(&public_b64, "value").unwrap();
    let opened = secret.unseal(&STANDARD.decode(sealed).unwrap()).unwrap();
    assert_eq!(opened, b"value");
}

#[test]
fn the_pubkey_is_read_and_replaced_without_touching_the_rest() {
    let conf = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/dots-app/tauri.conf.json"),
    )
    .unwrap();
    assert_eq!(pubkey_in_conf(&conf).as_deref(), Some(PROTOTYPE_PUBKEY));
    let changed = with_pubkey(&conf, "TkVXS0VZ").unwrap();
    assert_eq!(pubkey_in_conf(&changed).as_deref(), Some("TkVXS0VZ"));
    let differing = conf
        .lines()
        .zip(changed.lines())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differing, 1);
    assert_eq!(conf.lines().count(), changed.lines().count());
}

#[test]
fn debug_shows_no_key() {
    let made = make_key(PASSWORD).unwrap();
    let shown = format!("{made:?}");
    assert!(!shown.contains(&made.private_b64));
    assert!(!shown.contains(&made.public_b64));
}
