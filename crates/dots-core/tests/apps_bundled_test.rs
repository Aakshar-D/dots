//! The apps that come with Dots (app library spec section 8): Projects is built into Dots from
//! its folder, passes the checks a dot's proposal gets, and its own tests pass.

use std::collections::BTreeMap;
use std::path::Path;

use dots_core::apps::bundled::{projects, projects_files, PROJECTS};
use dots_core::apps::data;
use dots_core::apps::source::read_files;
use dots_core::apps::tests::run_tests;

/// Every file below `dir`, by its path relative to `root` with `/`.
fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&path).unwrap());
        }
    }
}

#[test]
fn projects_holds_every_file_of_its_folder_and_only_lf_text() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("apps")
        .join(PROJECTS);
    let mut on_disk = BTreeMap::new();
    walk(&root, &root, &mut on_disk);
    let built_in = projects_files();
    assert_eq!(
        built_in.keys().collect::<Vec<_>>(),
        on_disk.keys().collect::<Vec<_>>(),
        "list each file of apps/projects in bundled::PROJECTS_FILES"
    );
    for (path, bytes) in &built_in {
        // A CRLF checkout would change the migrations' bytes, and with them what a change to
        // Projects must keep (.gitattributes keeps the folder LF).
        assert!(!bytes.contains(&b'\r'), "{path} holds a carriage return");
        assert!(std::str::from_utf8(bytes).is_ok(), "{path} is not UTF-8");
    }
}

#[test]
fn projects_passes_the_checks_a_proposal_gets_and_its_own_tests() {
    let src = projects().unwrap();
    assert_eq!(src.manifest.name, PROJECTS);
    assert_eq!(src.manifest.title, "Projects");
    assert_eq!(read_files(&projects_files()).unwrap(), src);
    let report = run_tests(&src);
    assert!(report.passed(), "{}", report.problem().unwrap_or_default());
    assert!(report.results.len() >= 15, "{} tests", report.results.len());
}

#[test]
fn projects_migrations_run_and_its_sample_loads() {
    let src = projects().unwrap();
    let mut conn = data::open_in_memory().unwrap();
    assert_eq!(
        data::migrate(&mut conn, &src.migrations).unwrap(),
        src.migrations.len()
    );
    data::load_sample(&conn, src.sample.as_deref().unwrap()).unwrap();
    let tasks = data::query(&conn, "SELECT COUNT(*) AS n FROM tasks", &[]).unwrap();
    assert_eq!(tasks[0]["n"], 4);
}

#[test]
fn an_app_read_from_memory_is_bounded_like_a_folder() {
    let mut files = projects_files();
    files.insert("ui/big.bin".into(), vec![0; 2 * 1024 * 1024]);
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("more than 2 MB"), "{err}");
    let mut files = projects_files();
    files.insert("notes.txt".into(), b"hi".to_vec());
    let err = read_files(&files).unwrap_err().to_string();
    assert!(err.contains("notes.txt: an app holds"), "{err}");
}
