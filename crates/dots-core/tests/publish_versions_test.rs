use dots_core::publish::github::MergedPull;
use dots_core::publish::versions::{
    check_new, draft_notes, next_minor, next_rc, parse, set_version, VERSION_FILES,
};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_five_fields_are_set_and_nothing_else_moves() {
    for path in VERSION_FILES {
        let text = std::fs::read_to_string(root().join(path)).unwrap();
        let changed = set_version(path, &text, "0.3.0-rc.1").unwrap();
        let differing: Vec<(&str, &str)> = text
            .lines()
            .zip(changed.lines())
            .filter(|(a, b)| a != b)
            .collect();
        let expected = if path == "Cargo.lock" { 2 } else { 1 };
        assert_eq!(differing.len(), expected, "{path}: {differing:?}");
        for (_, new) in &differing {
            assert!(new.contains("0.3.0-rc.1"), "{path}: {new}");
        }
        assert_eq!(text.lines().count(), changed.lines().count(), "{path}");
        assert_eq!(text.ends_with('\n'), changed.ends_with('\n'), "{path}");
    }
}

#[test]
fn a_version_must_be_new_and_above_the_live_one() {
    let tags = vec![
        "v0.2.0".to_string(),
        "v0.3.0".to_string(),
        "v0.3.0-rc.1".to_string(),
    ];
    let format = "A version is MAJOR.MINOR.PATCH, for example 0.3.0.";
    assert_eq!(check_new("0.3", Some("0.2.0"), &tags).unwrap_err(), format);
    assert_eq!(
        check_new("0.3.0-rc.1", Some("0.2.0"), &tags).unwrap_err(),
        format
    );
    assert_eq!(
        check_new("0.2.0", Some("0.2.0"), &tags).unwrap_err(),
        "0.2.0 is not above the live 0.2.0."
    );
    assert_eq!(
        check_new("0.3.0", Some("0.2.0"), &tags).unwrap_err(),
        "0.3.0 was published before; use 0.4.0."
    );
    assert_eq!(check_new("0.4.0", Some("0.2.0"), &tags), Ok(()));
    assert_eq!(check_new("0.2.0", None, &[]), Ok(()));
    assert_eq!(parse("1.12.3"), Some((1, 12, 3)));
    assert_eq!(parse("01.2.3x"), None);
}

#[test]
fn the_next_trial_number_follows_the_tags() {
    let tags = vec![
        "v0.3.0-rc.1".to_string(),
        "v0.3.0-rc.2".to_string(),
        "v0.4.0-rc.7".to_string(),
    ];
    assert_eq!(next_rc("0.3.0", &tags), 3);
    assert_eq!(next_rc("0.5.0", &tags), 1);
}

#[test]
fn notes_are_drafted_from_pull_titles() {
    let pulls = vec![
        MergedPull {
            number: 26,
            title: "feat(ui): the rail's Claude usage bars".into(),
        },
        MergedPull {
            number: 27,
            title: "fix: run times".into(),
        },
        MergedPull {
            number: 25,
            title: "Watchers: dots that start on their own".into(),
        },
    ];
    assert_eq!(
        draft_notes(&pulls),
        "- The rail's Claude usage bars (#26)\n- Run times (#27)\n- Watchers: dots that start on their own (#25)\n"
    );
    assert_eq!(draft_notes(&[]), "");
}

#[test]
fn the_next_minor_follows_the_live_version() {
    assert_eq!(next_minor(None), "0.2.0");
    assert_eq!(next_minor(Some("0.2.3")), "0.3.0");
}
