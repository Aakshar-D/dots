//! The org's offline copy (env vault spec §5.2 and §6.4): protected on disk, one org's file never
//! opening as another's, and a failed write leaving the old copy.

use std::collections::BTreeMap;
use std::path::Path;

use dots_core::vault::org::{
    delete_offline, offline_path, read_offline, write_offline, NoProtector, OfflineCopy, Protector,
    TestProtector, OFFLINE_DIR,
};
use dots_core::{Error, Result};
use zeroize::Zeroizing;

const NAME: &str = "OFFLINE_NAME_4b1e";
const VALUE: &str = "offline-value-7f3a9c";
const FETCHED: &str = "2026-10-23T09:00:00.000Z";

fn copy(value: &str) -> OfflineCopy {
    let mut entries = BTreeMap::new();
    entries.insert(NAME.to_string(), Zeroizing::new(value.to_string()));
    entries.insert(
        "REGION".to_string(),
        Zeroizing::new("eu-west-1".to_string()),
    );
    OfflineCopy {
        fetched_at: FETCHED.into(),
        entries,
        policy: r#"{"v":1,"policy":"{}","signature":"sig"}"#.into(),
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

/// The names of the files in `dir`, sorted.
fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn message(e: Error) -> String {
    e.to_string()
}

/// Fails every call, as a protector that cannot reach DPAPI would.
#[derive(Debug)]
struct Failing;

impl Protector for Failing {
    fn protect(&self, _data: &[u8], _entropy: &[u8]) -> Result<Vec<u8>> {
        Err(Error::Other("protector down".into()))
    }

    fn unprotect(&self, _data: &[u8], _entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        Err(Error::Other("protector down".into()))
    }
}

#[test]
fn a_copy_round_trips_under_the_data_folders_vault() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(OFFLINE_DIR);
    write_offline(&dir, &TestProtector, "acme-eng", &copy(VALUE)).unwrap();
    assert_eq!(
        offline_path(&dir, "acme-eng").unwrap(),
        data.path().join("vault").join("acme-eng.bin")
    );
    let read = read_offline(&dir, &TestProtector, "acme-eng").unwrap();
    assert_eq!(read, Some(copy(VALUE)));
    // No temp file is left beside it.
    assert_eq!(files(&dir), vec!["acme-eng.bin".to_string()]);
}

#[test]
fn a_new_copy_replaces_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy("first")).unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy("second")).unwrap();
    let read = read_offline(dir.path(), &TestProtector, "acme").unwrap();
    assert_eq!(read, Some(copy("second")));
    assert_eq!(files(dir.path()), vec!["acme.bin".to_string()]);
}

#[test]
fn the_file_holds_neither_the_value_nor_the_name() {
    let dir = tempfile::tempdir().unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy(VALUE)).unwrap();
    let bytes = std::fs::read(offline_path(dir.path(), "acme").unwrap()).unwrap();
    assert!(bytes.starts_with(b"DOTSTEST"));
    assert!(!contains(&bytes, VALUE));
    assert!(!contains(&bytes, NAME));
    assert!(!contains(&bytes, "eu-west-1"));
}

#[test]
fn another_orgs_entropy_does_not_open_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy(VALUE)).unwrap();
    let path = offline_path(dir.path(), "acme").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(TestProtector.unprotect(&bytes, b"beta").is_err());
    assert!(TestProtector.unprotect(&bytes, b"acme").is_ok());

    // A file renamed to another org's name is unprotected with that org's id, and fails.
    std::fs::rename(&path, offline_path(dir.path(), "beta").unwrap()).unwrap();
    let err = message(read_offline(dir.path(), &TestProtector, "beta").unwrap_err());
    assert!(err.contains("org 'beta'"), "{err}");
    assert!(err.contains("other entropy"), "{err}");
    assert!(!err.contains(VALUE), "{err}");
}

#[test]
fn a_missing_copy_reads_none_and_deletes_quietly() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        read_offline(dir.path(), &TestProtector, "acme").unwrap(),
        None
    );
    delete_offline(dir.path(), "acme").unwrap();
    // A data folder without the vault folder yet is the same.
    let absent = dir.path().join(OFFLINE_DIR);
    assert_eq!(read_offline(&absent, &TestProtector, "acme").unwrap(), None);
    delete_offline(&absent, "acme").unwrap();
}

#[test]
fn delete_removes_the_copy_and_its_temp_files_only() {
    let dir = tempfile::tempdir().unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy(VALUE)).unwrap();
    write_offline(dir.path(), &TestProtector, "acme-eng", &copy(VALUE)).unwrap();
    // Temp files a crash left behind.
    std::fs::write(dir.path().join(".acme.01JBXYZ.tmp"), b"x").unwrap();
    std::fs::write(dir.path().join(".acme-eng.01JBXYZ.tmp"), b"x").unwrap();
    delete_offline(dir.path(), "acme").unwrap();
    assert_eq!(
        read_offline(dir.path(), &TestProtector, "acme").unwrap(),
        None
    );
    assert_eq!(
        files(dir.path()),
        vec![
            ".acme-eng.01JBXYZ.tmp".to_string(),
            "acme-eng.bin".to_string()
        ]
    );
    delete_offline(dir.path(), "acme").unwrap();
}

#[test]
fn a_corrupt_copy_is_an_error_naming_the_org() {
    let dir = tempfile::tempdir().unwrap();
    let path = offline_path(dir.path(), "acme").unwrap();

    std::fs::write(&path, b"not protected at all").unwrap();
    let err = message(read_offline(dir.path(), &TestProtector, "acme").unwrap_err());
    assert!(err.contains("org 'acme'"), "{err}");

    // Protected, but not JSON.
    std::fs::write(&path, TestProtector.protect(b"{\"v\":1,", b"acme").unwrap()).unwrap();
    let err = message(read_offline(dir.path(), &TestProtector, "acme").unwrap_err());
    assert!(err.contains("org 'acme'"), "{err}");
    assert!(err.contains("damaged"), "{err}");

    // An unknown field.
    let json =
        format!(r#"{{"v":1,"fetched_at":"{FETCHED}","entries":{{}},"policy":"p","extra":true}}"#);
    std::fs::write(
        &path,
        TestProtector.protect(json.as_bytes(), b"acme").unwrap(),
    )
    .unwrap();
    assert!(read_offline(dir.path(), &TestProtector, "acme").is_err());

    // A time that is not RFC 3339.
    let json = r#"{"v":1,"fetched_at":"yesterday","entries":{},"policy":"p"}"#;
    std::fs::write(
        &path,
        TestProtector.protect(json.as_bytes(), b"acme").unwrap(),
    )
    .unwrap();
    let err = message(read_offline(dir.path(), &TestProtector, "acme").unwrap_err());
    assert!(err.contains("RFC 3339"), "{err}");
}

#[test]
fn a_damaged_copy_never_quotes_a_value_in_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = offline_path(dir.path(), "acme").unwrap();
    for json in [
        format!(r#"{{"v":"{VALUE}"}}"#),
        format!(r#"{{"v":1,"fetched_at":"{FETCHED}","entries":{{"{VALUE}":5}},"policy":"p"}}"#),
        format!(r#"{{"v":1,"fetched_at":"{FETCHED}","entries":["{VALUE}"],"policy":"p"}}"#),
    ] {
        std::fs::write(
            &path,
            TestProtector.protect(json.as_bytes(), b"acme").unwrap(),
        )
        .unwrap();
        let err = message(read_offline(dir.path(), &TestProtector, "acme").unwrap_err());
        assert!(err.contains("org 'acme'"), "{err}");
        assert!(!err.contains(VALUE), "{err}");
    }
}

#[test]
fn an_unknown_format_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = offline_path(dir.path(), "acme").unwrap();
    for json in [
        format!(r#"{{"v":2,"fetched_at":"{FETCHED}","entries":{{}},"policy":"p"}}"#),
        r#"{"v":2,"shape":"new"}"#.to_string(),
    ] {
        std::fs::write(
            &path,
            TestProtector.protect(json.as_bytes(), b"acme").unwrap(),
        )
        .unwrap();
        let err = message(read_offline(dir.path(), &TestProtector, "acme").unwrap_err());
        assert!(err.contains("org 'acme'"), "{err}");
        assert!(err.contains("format v2"), "{err}");
    }
}

#[test]
fn the_json_form_carries_its_version() {
    let json = serde_json::to_value(copy(VALUE)).unwrap();
    assert_eq!(json["v"], 1);
    assert_eq!(json["fetched_at"], FETCHED);
    assert_eq!(json["entries"][NAME], VALUE);
    let back: OfflineCopy = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, copy(VALUE));
    let mut later = json;
    later["v"] = 2.into();
    assert!(serde_json::from_value::<OfflineCopy>(later).is_err());
}

#[test]
fn a_failed_write_leaves_the_old_copy() {
    let dir = tempfile::tempdir().unwrap();
    write_offline(dir.path(), &TestProtector, "acme", &copy("old-value")).unwrap();
    let err = write_offline(dir.path(), &Failing, "acme", &copy("new-value")).unwrap_err();
    assert!(message(err).contains("protector down"));
    let read = read_offline(dir.path(), &TestProtector, "acme").unwrap();
    assert_eq!(read, Some(copy("old-value")));
    assert_eq!(files(dir.path()), vec!["acme.bin".to_string()]);
}

#[test]
fn a_failed_rename_leaves_no_temp_file() {
    let dir = tempfile::tempdir().unwrap();
    // A folder where the copy goes: the rename over it fails.
    std::fs::create_dir(offline_path(dir.path(), "acme").unwrap()).unwrap();
    let err = write_offline(dir.path(), &TestProtector, "acme", &copy(VALUE)).unwrap_err();
    assert!(message(err).contains("org 'acme'"));
    assert_eq!(files(dir.path()), vec!["acme.bin".to_string()]);
    assert!(offline_path(dir.path(), "acme").unwrap().is_dir());
}

#[test]
fn a_copy_with_a_bad_time_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let mut bad = copy(VALUE);
    bad.fetched_at = "yesterday".into();
    let err = write_offline(dir.path(), &TestProtector, "acme", &bad).unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    assert!(files(dir.path()).is_empty());
}

#[test]
fn a_bad_org_id_is_refused_before_any_file_is_touched() {
    let dir = tempfile::tempdir().unwrap();
    for id in ["../x", "Acme", "", "a/b", "acme.bin"] {
        assert!(
            matches!(offline_path(dir.path(), id), Err(Error::Invalid(_))),
            "{id:?}"
        );
        assert!(write_offline(dir.path(), &TestProtector, id, &copy(VALUE)).is_err());
        assert!(read_offline(dir.path(), &TestProtector, id).is_err());
        assert!(delete_offline(dir.path(), id).is_err());
    }
    assert!(files(dir.path()).is_empty());
}

#[test]
fn debug_shows_the_time_and_names_but_no_value() {
    let shown = format!("{:?}", copy(VALUE));
    assert!(shown.contains(FETCHED), "{shown}");
    assert!(shown.contains(NAME), "{shown}");
    assert!(!shown.contains(VALUE), "{shown}");
    assert!(!shown.contains("eu-west-1"), "{shown}");
}

#[test]
fn the_test_protector_refuses_data_it_did_not_protect() {
    assert!(TestProtector.unprotect(b"plain text", b"acme").is_err());
    assert!(TestProtector.unprotect(b"DOTSTEST", b"acme").is_err());
    let sealed = TestProtector.protect(VALUE.as_bytes(), b"acme").unwrap();
    assert!(!contains(&sealed, VALUE));
    let plain = TestProtector.unprotect(&sealed, b"acme").unwrap();
    assert_eq!(plain.as_slice(), VALUE.as_bytes());
}

#[test]
fn no_protector_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join(OFFLINE_DIR);
    let err = write_offline(&vault, &NoProtector, "acme", &copy(VALUE)).unwrap_err();
    assert!(message(err).contains("no offline copy on this platform"));
    assert!(!vault.exists());
    assert!(NoProtector.unprotect(b"x", b"acme").is_err());
}

#[cfg(windows)]
mod dpapi {
    use super::*;
    use dots_core::vault::org::{os_protector, Dpapi};

    #[test]
    fn the_os_protector_is_dpapi() {
        assert_eq!(format!("{:?}", os_protector()), "Dpapi");
    }

    #[test]
    fn a_copy_round_trips_under_this_user() {
        let dir = tempfile::tempdir().unwrap();
        write_offline(dir.path(), &Dpapi, "acme", &copy(VALUE)).unwrap();
        let bytes = std::fs::read(offline_path(dir.path(), "acme").unwrap()).unwrap();
        assert!(!contains(&bytes, VALUE));
        assert!(!contains(&bytes, NAME));
        assert_eq!(
            read_offline(dir.path(), &Dpapi, "acme").unwrap(),
            Some(copy(VALUE))
        );
        // The test protector cannot read DPAPI's file.
        assert!(read_offline(dir.path(), &TestProtector, "acme").is_err());
    }

    #[test]
    fn protected_bytes_hold_no_value_and_open_only_with_their_entropy() {
        let sealed = Dpapi.protect(VALUE.as_bytes(), b"acme").unwrap();
        assert!(!contains(&sealed, VALUE));
        let plain = Dpapi.unprotect(&sealed, b"acme").unwrap();
        assert_eq!(plain.as_slice(), VALUE.as_bytes());

        let err = message(Dpapi.unprotect(&sealed, b"beta").unwrap_err());
        assert!(err.starts_with("DPAPI: unprotecting data failed"), "{err}");
        assert!(err.contains("with other entropy"), "{err}");
        assert!(Dpapi.unprotect(&sealed, b"").is_err());
    }

    #[test]
    fn a_renamed_dpapi_file_does_not_open_as_another_org() {
        let dir = tempfile::tempdir().unwrap();
        write_offline(dir.path(), &Dpapi, "acme", &copy(VALUE)).unwrap();
        std::fs::rename(
            offline_path(dir.path(), "acme").unwrap(),
            offline_path(dir.path(), "beta").unwrap(),
        )
        .unwrap();
        let err = message(read_offline(dir.path(), &Dpapi, "beta").unwrap_err());
        assert!(err.contains("org 'beta'"), "{err}");
        assert!(err.contains("DPAPI"), "{err}");
    }

    #[test]
    fn data_dpapi_did_not_protect_is_an_error_not_a_panic() {
        assert!(Dpapi.unprotect(b"not a DPAPI blob", b"acme").is_err());
        assert!(Dpapi.unprotect(b"", b"acme").is_err());
        let sealed = Dpapi.protect(VALUE.as_bytes(), b"acme").unwrap();
        assert!(Dpapi
            .unprotect(&sealed[..sealed.len() / 2], b"acme")
            .is_err());
    }

    #[test]
    fn empty_data_round_trips() {
        let sealed = Dpapi.protect(b"", b"acme").unwrap();
        assert!(Dpapi.unprotect(&sealed, b"acme").unwrap().is_empty());
    }
}
