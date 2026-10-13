mod common;

use std::path::{Path, PathBuf};

use common::{fake_dot, git_out, init_repo};
use dots_core::model::{FileStatus, WorkspaceMode};
use dots_core::workspace::{diff, link_on_path, untracked_entry, Prepared, WorkspaceManager};
use dots_core::Error;

/// A worktree of a new repository under `tmp`, and a folder outside it that holds `secret.txt`.
async fn worktree_and_outside(tmp: &Path) -> (Prepared, PathBuf) {
    let repo = tmp.join("repo");
    init_repo(&repo);
    let dot = fake_dot("links", &repo, WorkspaceMode::Worktree);
    let p = WorkspaceManager::new(tmp.join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap();
    let outside = tmp.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "TOP-SECRET\n").unwrap();
    (p, outside)
}

/// Makes `link` a junction to the folder `target`. A junction needs no privilege, so the tests
/// that use one always run on Windows.
#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) {
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        made.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&made.stderr)
    );
}

/// Says on the terminal that a test did not run. Written straight to stderr, which the test
/// harness does not capture, so the line shows in a passing `cargo test`.
fn report_skipped(test: &str, why: &dyn std::fmt::Display) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "SKIPPED {test}: {why}");
}

#[tokio::test]
async fn folder_mode_uses_workdir_directly() {
    let tmp = tempfile::tempdir().unwrap();
    let dot = fake_dot("f", tmp.path(), WorkspaceMode::Folder);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert_eq!(p.path, tmp.path());
    assert!(p.branch.is_none() && p.base_commit.is_none());
    assert!(!wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(tmp.path().exists());
}

#[tokio::test]
async fn missing_workdir_is_invalid() {
    let tmp = tempfile::tempdir().unwrap();
    let dot = fake_dot("m", &tmp.path().join("nope"), WorkspaceMode::Folder);
    let err = WorkspaceManager::new(tmp.path().join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn worktree_mode_requires_a_git_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let dot = fake_dot("g", &plain, WorkspaceMode::Worktree);
    let err = WorkspaceManager::new(tmp.path().join("wt"))
        .prepare(&dot, "R1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn worktree_is_created_on_a_new_branch_from_head() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let head = git_out(&repo, &["rev-parse", "HEAD"]);
    let dot = fake_dot("triage", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert_eq!(p.path, tmp.path().join("wt").join("R1"));
    assert!(p.path.join("README.md").exists());
    assert_eq!(p.branch.as_deref(), Some("dots/triage/R1"));
    assert_eq!(p.base_commit.as_deref(), Some(head.as_str()));
    assert!(!git_out(&repo, &["branch", "--list", "dots/triage/R1"]).is_empty());
    assert!(!wm.has_changes(&p.path, &head).await.unwrap());
}

#[tokio::test]
async fn unchanged_worktree_is_removed_with_its_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("clean", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert!(wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(!p.path.exists());
    assert!(git_out(&repo, &["branch", "--list", "dots/clean/R1"]).is_empty());
}

#[tokio::test]
async fn changed_worktrees_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("dirty", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));

    let uncommitted = wm.prepare(&dot, "R1").await.unwrap();
    std::fs::write(uncommitted.path.join("new.txt"), "x").unwrap();
    assert!(!wm.cleanup_if_unchanged(&dot, &uncommitted).await.unwrap());
    assert!(uncommitted.path.exists());

    let committed = wm.prepare(&dot, "R2").await.unwrap();
    std::fs::write(committed.path.join("c.txt"), "y").unwrap();
    git_out(&committed.path, &["add", "."]);
    git_out(&committed.path, &["commit", "-q", "-m", "work"]);
    assert!(wm
        .has_changes(&committed.path, committed.base_commit.as_deref().unwrap())
        .await
        .unwrap());
    assert!(!wm.cleanup_if_unchanged(&dot, &committed).await.unwrap());
}

#[tokio::test]
async fn restore_recreates_a_removed_worktree_at_the_same_path() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("again", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));

    // Branch deleted by cleanup: recreated from the base commit.
    let p = wm.prepare(&dot, "R1").await.unwrap();
    assert!(wm.cleanup_if_unchanged(&dot, &p).await.unwrap());
    assert!(!p.path.exists());
    let restored = wm.restore(&dot, &p).await.unwrap();
    assert_eq!(restored, p);
    assert!(p.path.join("README.md").is_file());
    assert_eq!(
        git_out(&p.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "dots/again/R1"
    );
    assert_eq!(
        git_out(&p.path, &["rev-parse", "HEAD"]),
        p.base_commit.clone().unwrap()
    );

    // Directory deleted behind git's back, branch still there (with a commit): reused.
    std::fs::write(p.path.join("c.txt"), "y").unwrap();
    git_out(&p.path, &["add", "."]);
    git_out(&p.path, &["commit", "-q", "-m", "work"]);
    let tip = git_out(&p.path, &["rev-parse", "HEAD"]);
    std::fs::remove_dir_all(&p.path).unwrap();
    wm.restore(&dot, &p).await.unwrap();
    assert_eq!(git_out(&p.path, &["rev-parse", "HEAD"]), tip);
}

#[tokio::test]
async fn an_untracked_name_keeps_its_leading_space() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, _outside) = worktree_and_outside(tmp.path()).await;
    // A space sorts before every other name, so this is the first path git lists.
    std::fs::write(p.path.join(" lead.txt"), "x\n").unwrap();
    std::fs::write(p.path.join("next.txt"), "y\n").unwrap();

    let d = diff(&p.path, p.base_commit.as_deref().unwrap())
        .await
        .unwrap();
    let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, [" lead.txt", "next.txt"]);
    assert!(
        d.files[0].patch.ends_with("@@ -0,0 +1,1 @@\n+x\n"),
        "{:?}",
        d.files[0].patch
    );
}

/// A junction needs no privilege, so this test always runs on Windows. Git walks into a
/// junction and reports the files behind it; the diff must show the junction and stop there.
#[cfg(windows)]
#[tokio::test]
async fn an_untracked_junction_is_listed_once_and_not_entered() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, outside) = worktree_and_outside(tmp.path()).await;
    make_junction(&p.path.join("junc"), &outside);

    let d = diff(&p.path, p.base_commit.as_deref().unwrap())
        .await
        .unwrap();
    let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["junc"]);
    let link = &d.files[0];
    assert_eq!((link.status, link.binary), (FileStatus::Added, false));
    // Shown as git shows a link: the place it points to, never what is there.
    assert!(
        link.patch.contains("new file mode 120000"),
        "{}",
        link.patch
    );
    assert!(link.patch.contains("outside"), "{}", link.patch);
    assert!(!link.patch.contains("TOP-SECRET"), "{}", link.patch);
    assert!(!d.truncated);
}

#[tokio::test]
async fn an_untracked_symlink_is_listed_and_its_target_is_not_read() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, outside) = worktree_and_outside(tmp.path()).await;
    let target = outside.join("secret.txt");
    let link = p.path.join("link.txt");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(&target, &link);
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&target, &link);
    if let Err(e) = made {
        // Windows without developer mode or the symlink privilege (error 1314). Any other
        // failure is a broken test, and neither may pass silently.
        assert_eq!(e.raw_os_error(), Some(1314), "creating a symlink: {e}");
        report_skipped(
            "an_untracked_symlink_is_listed_and_its_target_is_not_read",
            &e,
        );
        return;
    }

    let d = diff(&p.path, p.base_commit.as_deref().unwrap())
        .await
        .unwrap();
    let paths: Vec<&str> = d.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["link.txt"]);
    let shown = &d.files[0];
    assert_eq!((shown.status, shown.binary), (FileStatus::Added, false));
    assert!(
        shown.patch.contains("new file mode 120000"),
        "{}",
        shown.patch
    );
    assert!(shown.patch.contains("secret.txt"), "{}", shown.patch);
    assert!(!shown.patch.contains("TOP-SECRET"), "{}", shown.patch);
}

/// The decision itself, without a real link: a path the diff is told is a link is listed and
/// never opened, whatever is behind it.
#[tokio::test]
async fn a_path_marked_as_a_link_is_listed_without_being_opened() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("note.txt"), "TOP-SECRET\n").unwrap();

    let (opened, _) = untracked_entry(tmp.path(), "note.txt", false).await;
    assert!(opened.patch.ends_with("+TOP-SECRET\n"), "{}", opened.patch);

    let (listed, truncated) = untracked_entry(tmp.path(), "note.txt", true).await;
    assert_eq!(
        (listed.path.as_str(), listed.status, listed.binary),
        ("note.txt", FileStatus::Added, false)
    );
    // Not a real link, so there is no target to show; the file itself was not read.
    assert_eq!(listed.patch, "");
    assert!(!truncated);

    // Ordinary files and folders, and paths that are not there, are not links.
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("sub").join("a.txt"), "a").unwrap();
    for rel in ["note.txt", "sub/a.txt", "missing/b.txt"] {
        assert_eq!(link_on_path(tmp.path(), rel), None, "{rel}");
    }
}

#[cfg(windows)]
#[test]
fn a_junction_above_a_path_is_found_as_the_link() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(outside.join("deep")).unwrap();
    let root = tmp.path().join("root");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    make_junction(&root.join("sub").join("junc"), &outside);
    for rel in ["sub/junc", "sub/junc/deep", "sub/junc/deep/x.txt"] {
        assert_eq!(
            link_on_path(&root, rel).as_deref(),
            Some("sub/junc"),
            "{rel}"
        );
    }
    assert_eq!(link_on_path(&root, "sub/other.txt"), None);
}

#[test]
fn only_a_direct_child_of_the_worktrees_folder_with_its_own_branch_is_a_dots_worktree() {
    let wm = WorkspaceManager::new("C:/data/worktrees");
    let check = |path: &str, branch: Option<&str>| {
        wm.check_owned(&Prepared {
            path: PathBuf::from(path),
            branch: branch.map(str::to_string),
            base_commit: None,
        })
    };
    check("C:/data/worktrees/R1", Some("dots/triage/R1")).unwrap();
    // A trailing separator names the same folder.
    check("C:/data/worktrees/R1/", Some("dots/triage/R1")).unwrap();

    // Each branch below fits its path's last name, so only the path can be the reason.
    for (path, branch) in [
        ("C:/data/worktrees", "dots/triage/worktrees"),
        ("C:/data/worktrees/R1/sub", "dots/triage/sub"),
        ("C:/data/worktrees/../R1", "dots/triage/R1"),
        ("C:/data/worktrees/R2/../R1", "dots/triage/R1"),
        ("C:/data/worktrees/R1/..", "dots/triage/R1"),
        ("C:/data/worktrees-old/R1", "dots/triage/R1"),
        ("C:/data/R1", "dots/triage/R1"),
        ("C:/repo", "dots/triage/repo"),
        ("worktrees/R1", "dots/triage/R1"),
    ] {
        let err = check(path, Some(branch)).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{path}: {err}");
        assert!(err.to_string().contains("not a dots worktree"), "{err}");
    }
    for branch in [
        "feature-x",
        "main",
        "dots/triage/R2",
        "dots/R1",
        "dots//R1",
        "x/dots/triage/R1",
        "dots/triage/R1/",
        "",
    ] {
        let err = check("C:/data/worktrees/R1", Some(branch)).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{branch}: {err}");
        assert!(err.to_string().contains("not the dots branch"), "{err}");
    }
    let err = check("C:/data/worktrees/R1", None).unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
}

/// Git removes a worktree but leaves a junction inside it where it is, and still reports
/// success. What is left is removed as links, never through them.
#[cfg(windows)]
#[tokio::test]
async fn removing_a_worktree_with_a_junction_removes_the_folder_and_spares_the_target() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, outside) = worktree_and_outside(tmp.path()).await;
    std::fs::create_dir(p.path.join("deps")).unwrap();
    make_junction(&p.path.join("deps").join("junc"), &outside);
    std::fs::write(p.path.join("new.txt"), "x").unwrap();

    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    wm.remove(&tmp.path().join("repo"), &p).await.unwrap();
    assert!(!p.path.exists(), "the worktree folder was left behind");
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "TOP-SECRET
"
    );
    assert!(git_out(
        &tmp.path().join("repo"),
        &["branch", "--list", "dots/links/R1"]
    )
    .is_empty());
}

/// What git leaves is only ever deleted inside the manager's own folder; anywhere else the
/// removal fails and says which folder is still there.
#[cfg(windows)]
#[tokio::test]
async fn a_leftover_outside_the_worktrees_folder_is_reported_and_not_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, outside) = worktree_and_outside(tmp.path()).await;
    make_junction(&p.path.join("junc"), &outside);

    let elsewhere = WorkspaceManager::new(tmp.path().join("elsewhere"));
    let err = elsewhere
        .remove(&tmp.path().join("repo"), &p)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err}");
    assert!(
        err.to_string().contains(&p.path.display().to_string()),
        "{err}"
    );
    assert!(p.path.join("junc").join("secret.txt").is_file());
    // The check runs before git: nothing at all was removed.
    let repo = tmp.path().join("repo");
    assert!(git_out(&repo, &["worktree", "list", "--porcelain"]).contains("/R1"));
    assert!(!git_out(&repo, &["branch", "--list", "dots/links/R1"]).is_empty());
}

/// A leftover that cannot be deleted is an error that names the folder, never a silent success.
#[cfg(windows)]
#[tokio::test]
async fn a_leftover_that_cannot_be_removed_fails_and_names_the_folder() {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT: open the junction itself.
    const THE_LINK_ITSELF: u32 = 0x0200_0000 | 0x0020_0000;
    const FILE_SHARE_READ: u32 = 1;
    let tmp = tempfile::tempdir().unwrap();
    let (p, outside) = worktree_and_outside(tmp.path()).await;
    make_junction(&p.path.join("junc"), &outside);
    // Held open without sharing delete, the junction cannot be removed.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(THE_LINK_ITSELF)
        .open(p.path.join("junc"))
        .unwrap();

    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let err = wm.remove(&tmp.path().join("repo"), &p).await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("could not remove what git left of"), "{text}");
    assert!(text.contains(&p.path.display().to_string()), "{text}");
    assert!(p.path.join("junc").exists());
    drop(held);
    assert!(outside.join("secret.txt").is_file());
    // Git has unregistered the worktree by now; once the hold is released, a second discard
    // still finishes the job.
    wm.discard(&tmp.path().join("repo"), &p).await.unwrap();
    assert!(!p.path.exists());
    assert!(outside.join("secret.txt").is_file());
    assert!(git_out(
        &tmp.path().join("repo"),
        &["branch", "--list", "dots/links/R1"]
    )
    .is_empty());
}

/// Git no longer knows the worktree (it failed half way, or the registration was pruned), but
/// its folder is still there. Discard still removes the folder and the branch.
#[tokio::test]
async fn discard_removes_a_folder_git_no_longer_knows_and_the_branch() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, _outside) = worktree_and_outside(tmp.path()).await;
    let repo = tmp.path().join("repo");
    std::fs::remove_file(p.path.join(".git")).unwrap();
    git_out(&repo, &["worktree", "prune"]);
    assert!(p.path.is_dir());

    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    wm.discard(&repo, &p).await.unwrap();
    assert!(!p.path.exists());
    assert!(git_out(&repo, &["branch", "--list", "dots/links/R1"]).is_empty());
}

/// A leftover folder without a `.git` of its own, inside a data folder that lies in another
/// repository: that repository is not the worktree's, and nothing of it is touched.
#[tokio::test]
async fn a_leftover_inside_another_repository_does_not_make_it_the_worktrees_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let outer = tmp.path().join("outer");
    init_repo(&outer);
    git_out(&outer, &["branch", "dots/links/R9"]);
    git_out(&repo, &["branch", "dots/links/R9"]);
    let path = outer.join("wt").join("R9");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("left.txt"), "x").unwrap();
    let p = Prepared {
        path: path.clone(),
        branch: Some("dots/links/R9".into()),
        base_commit: None,
    };

    WorkspaceManager::new(outer.join("wt"))
        .discard(&repo, &p)
        .await
        .unwrap();
    assert!(!path.exists());
    assert!(git_out(&repo, &["branch", "--list", "dots/links/R9"]).is_empty());
    assert!(!git_out(&outer, &["branch", "--list", "dots/links/R9"]).is_empty());
}

/// The unchanged-run cleanup works from a run row. A row that does not name a worktree of
/// dots is refused before any git command: no other branch or worktree is touched.
#[tokio::test]
async fn cleaning_up_an_unchanged_run_refuses_a_forged_row_before_any_git_command() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    let dot = fake_dot("links", &repo, WorkspaceMode::Worktree);
    let wm = WorkspaceManager::new(tmp.path().join("wt"));
    let a = wm.prepare(&dot, "R1").await.unwrap();
    let b = wm.prepare(&dot, "R2").await.unwrap();
    git_out(&repo, &["branch", "feature-x"]);
    let foreign = tmp.path().join("foreign");
    git_out(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "dots/links/foreign",
            &foreign.to_string_lossy(),
        ],
    );

    let forged = [
        // Another existing branch.
        Prepared {
            branch: Some("feature-x".into()),
            ..a.clone()
        },
        Prepared {
            branch: a.branch.clone(),
            ..b.clone()
        },
        // Another linked worktree, outside the worktrees folder.
        Prepared {
            path: foreign.clone(),
            branch: Some("dots/links/foreign".into()),
            base_commit: a.base_commit.clone(),
        },
    ];
    for row in &forged {
        let err = wm.cleanup_if_unchanged(&dot, row).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{row:?}: {err}");
        for dir in [&a.path, &b.path, &foreign] {
            assert!(dir.is_dir(), "{} was removed", dir.display());
        }
        let branches = git_out(&repo, &["branch", "--list"]);
        for name in [
            "feature-x",
            "dots/links/R1",
            "dots/links/R2",
            "dots/links/foreign",
        ] {
            assert!(branches.contains(name), "{name} was deleted: {branches}");
        }
    }
    assert_eq!(
        git_out(&repo, &["worktree", "list", "--porcelain"])
            .matches("worktree ")
            .count(),
        4
    );
}
