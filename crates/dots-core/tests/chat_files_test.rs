//! Staging a chat attachment (attachments spec section 4.2): the 25 MB limit, folders, links
//! and missing paths, a hostile pasted name, and the helpers the prompt and the Files panel use.

use std::path::Path;

use dots_core::chat_files::{can_open, can_open_file, human_size, ChatFiles, MAX_FILE_BYTES};
use dots_core::Error;

fn invalid(e: Error) -> String {
    match e {
        Error::Invalid(m) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_file_at_the_limit_is_staged() {
    let dir = tempfile::tempdir().unwrap();
    let files = ChatFiles::new(dir.path().join("chat-files"));
    let from = dir.path().join("exact.bin");
    let bytes: Vec<u8> = (0..MAX_FILE_BYTES).map(|i| (i % 251) as u8).collect();
    std::fs::write(&from, &bytes).unwrap();
    let (name, size) = files.stage_path("A1", &from).unwrap();
    assert_eq!(name, "exact.bin");
    assert_eq!(size, MAX_FILE_BYTES);
    let stored = dir.path().join("chat-files").join("A1").join("exact.bin");
    assert!(std::fs::read(stored).unwrap() == bytes);
}

#[test]
fn a_file_one_byte_over_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let files = ChatFiles::new(dir.path().join("chat-files"));
    let from = dir.path().join("big.bin");
    let f = std::fs::File::create(&from).unwrap();
    f.set_len(MAX_FILE_BYTES + 1).unwrap();
    drop(f);
    let e = files.stage_path("A1", &from).unwrap_err();
    assert_eq!(invalid(e), "big.bin is larger than 25 MB.");
    assert!(!dir.path().join("chat-files").join("A1").exists());
}

#[test]
fn folders_links_and_missing_paths_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let files = ChatFiles::new(dir.path().join("chat-files"));
    let docs = dir.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    assert_eq!(
        invalid(files.stage_path("A1", &docs).unwrap_err()),
        "docs is a folder. Attach the files in it instead."
    );
    let missing = invalid(
        files
            .stage_path("A2", &dir.path().join("nope.txt"))
            .unwrap_err(),
    );
    assert!(
        missing.starts_with("nope.txt could not be read: "),
        "{missing}"
    );
    // A link to a real file is refused without being followed.
    let target = dir.path().join("real.txt");
    std::fs::write(&target, "x").unwrap();
    let link = dir.path().join("link.txt");
    if make_file_link(&target, &link) {
        assert_eq!(
            invalid(files.stage_path("A3", &link).unwrap_err()),
            "link.txt is a link. Attach the file it points to instead."
        );
    } else {
        eprintln!("skipped the link case: this account may not create symlinks");
    }
    // A junction to a folder is a link too, not a folder.
    let junction = dir.path().join("jdocs");
    if make_dir_link(&docs, &junction) {
        assert_eq!(
            invalid(files.stage_path("A4", &junction).unwrap_err()),
            "jdocs is a link. Attach the file it points to instead."
        );
    }
    for id in ["A1", "A2", "A3", "A4"] {
        assert!(!dir.path().join("chat-files").join(id).exists(), "{id}");
    }
}

#[cfg(windows)]
fn make_file_link(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn make_file_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn make_dir_link(target: &Path, link: &Path) -> bool {
    std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(unix)]
fn make_dir_link(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[test]
fn bytes_get_a_safe_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("chat-files");
    let files = ChatFiles::new(root.clone());
    let (name, size) = files.stage_bytes("A1", "..\\..\\CON.png", b"x").unwrap();
    assert_eq!(name, "_CON.png");
    assert_eq!(size, 1);
    assert_eq!(
        std::fs::read(root.join("A1").join("_CON.png")).unwrap(),
        b"x"
    );
    // Too many bytes are refused like a file.
    let big = vec![0u8; MAX_FILE_BYTES as usize + 1];
    assert_eq!(
        invalid(files.stage_bytes("A2", "shot.png", &big).unwrap_err()),
        "shot.png is larger than 25 MB."
    );
    assert!(!root.join("A2").exists());
}

#[test]
fn human_size_matches_the_ui() {
    assert_eq!(human_size(240), "240 B");
    assert_eq!(human_size(2458), "2.4 KB");
    assert_eq!(human_size(1_572_864), "1.5 MB");
    assert_eq!(human_size(31_457_280), "30 MB");
    assert_eq!(human_size(20_480), "20 KB");
}

#[test]
fn only_documents_images_and_media_open() {
    for name in [
        "r.pdf",
        "a.exe.pdf",
        "N.TXT",
        "x.md",
        "x.csv",
        "x.json",
        "x.yml",
        "x.log",
        "p.PNG",
        "p.jpeg",
        "p.heic",
        "s.mp3",
        "v.mkv",
        "d.docx",
        "d.xlsx",
        "d.pptx",
        "d.odt",
    ] {
        assert!(can_open(Path::new(name)), "{name}");
    }
    for name in [
        "a.PS1",
        "C:\\x\\setup.exe",
        "app.appref-ms",
        "README",
        "run.cmd.",
        "run.cmd ",
        "run.cmd. . ",
        "RUN.CMD",
        "a.pdf.exe",
        "a.pdf.",
        "a.pdf ",
        "x.docm",
        "x.xlsm",
        "x.html",
        "x.htm",
        "x.svg",
        "cleanup.py",
        "a.sh",
        "a.zip",
        "a.iso",
        "a.pdf:evil.exe",
        "a.exe:x.pdf",
        ".pdf",
    ] {
        assert!(!can_open(Path::new(name)), "{name}");
    }
}

#[test]
fn a_file_opens_only_by_the_name_it_really_has() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("run.cmd"), "x").unwrap();
    std::fs::write(dir.path().join("r.pdf"), "x").unwrap();
    assert!(can_open_file(&dir.path().join("r.pdf")));
    assert!(!can_open_file(&dir.path().join("run.cmd")));
    assert!(!can_open_file(&dir.path().join("missing.pdf")));
    // Windows drops trailing dots and spaces: these name run.cmd, and r.pdf.
    assert!(!can_open_file(&dir.path().join("run.cmd.")));
    assert!(!can_open_file(&dir.path().join("run.cmd ")));
    assert!(!can_open_file(&dir.path().join("r.pdf.")));
}
