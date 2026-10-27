//! The publishing flows against a fake repository: what each flow reads, writes and refuses.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use dots_core::publish::flow::{
    self, page_state, promote, publish_trial, put_key_in_secrets, save_notes, setup_state,
    stage_of, Hub, Setup, Stage, KEY_SECRET, NOTES_FILE, PASSWORD_SECRET,
};
use dots_core::publish::github::{JobInfo, MergeOutcome, MergedPull, ReleaseInfo, RunInfo};
use dots_core::publish::keys::{make_key, PROTOTYPE_PUBKEY};
use dots_core::publish::versions::VERSION_FILES;
use dots_core::releases::WITHDRAWN_MARKER;
use dots_core::{Error, Result};

type Tree = BTreeMap<String, String>;

#[derive(Default)]
struct Repo {
    secrets: HashSet<String>,
    commits: HashMap<String, (Option<String>, Tree)>,
    branches: HashMap<String, String>,
    tags: BTreeMap<String, String>,
    releases: Vec<ReleaseInfo>,
    pulls: Vec<MergedPull>,
    merge: Option<MergeOutcome>,
    calls: Vec<String>,
    next: u32,
}

struct Fake(RefCell<Repo>);

fn real_tree(pubkey: &str) -> Tree {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut tree: Tree = VERSION_FILES
        .iter()
        .map(|p| {
            (
                p.to_string(),
                std::fs::read_to_string(root.join(p)).unwrap(),
            )
        })
        .collect();
    tree.insert("apps/dots-app/tauri.conf.json".into(), {
        let real = tree["apps/dots-app/tauri.conf.json"].clone();
        real.replace(PROTOTYPE_PUBKEY, pubkey)
    });
    tree.insert("README.md".into(), "readme".into());
    tree
}

impl Fake {
    fn new(pubkey: &str) -> Fake {
        let mut repo = Repo::default();
        repo.commits.insert("c0".into(), (None, real_tree(pubkey)));
        repo.branches.insert("master".into(), "c0".into());
        repo.pulls = vec![MergedPull {
            number: 26,
            title: "feat(ui): the rail's Claude usage bars".into(),
        }];
        Fake(RefCell::new(repo))
    }
    fn resolve(&self, reference: &str) -> Option<String> {
        let repo = self.0.borrow();
        repo.branches
            .get(reference)
            .or_else(|| repo.tags.get(reference))
            .cloned()
            .or_else(|| {
                repo.commits
                    .contains_key(reference)
                    .then(|| reference.to_string())
            })
    }
    fn tree(&self, reference: &str) -> Tree {
        let sha = self.resolve(reference).unwrap();
        self.0.borrow().commits[&sha].1.clone()
    }
    fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }
    fn log(&self, call: String) {
        self.0.borrow_mut().calls.push(call);
    }
    /// A commit straight on master, as a merged pull request would make.
    fn push_master(&self) {
        let parent = self.resolve("master").unwrap();
        let mut tree = self.tree("master");
        tree.insert("README.md".into(), "newer".into());
        let mut repo = self.0.borrow_mut();
        repo.next += 1;
        let sha = format!("m{}", repo.next);
        repo.commits.insert(sha.clone(), (Some(parent), tree));
        repo.branches.insert("master".into(), sha);
    }
}

#[allow(async_fn_in_trait)]
impl Hub for Fake {
    async fn secret_exists(&self, name: &str) -> Result<bool> {
        Ok(self.0.borrow().secrets.contains(name))
    }
    async fn secrets_key(&self) -> Result<(String, String)> {
        Ok((
            "kid".into(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into(),
        ))
    }
    async fn put_secret(&self, name: &str, _key_id: &str, _encrypted_b64: &str) -> Result<()> {
        self.log(format!("put {name}"));
        self.0.borrow_mut().secrets.insert(name.to_string());
        Ok(())
    }
    async fn file(&self, path: &str, reference: &str) -> Result<Option<String>> {
        Ok(self
            .resolve(reference)
            .and_then(|_| self.tree(reference).get(path).cloned()))
    }
    async fn head(&self, branch: &str) -> Result<String> {
        self.0
            .borrow()
            .branches
            .get(branch)
            .cloned()
            .ok_or_else(|| Error::Other(format!("no branch {branch}")))
    }
    async fn commit_files(
        &self,
        parent: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String> {
        let mut tree = self.tree(parent);
        for (path, text) in files {
            tree.insert(path.clone(), text.clone());
        }
        let mut repo = self.0.borrow_mut();
        repo.next += 1;
        let sha = format!("c{}", repo.next);
        repo.commits
            .insert(sha.clone(), (Some(parent.to_string()), tree));
        repo.calls
            .push(format!("commit {sha} on {parent}: {message}"));
        Ok(sha)
    }
    async fn set_branch(&self, branch: &str, sha: &str, _force: bool) -> Result<()> {
        self.log(format!("branch {branch} -> {sha}"));
        self.0
            .borrow_mut()
            .branches
            .insert(branch.into(), sha.into());
        Ok(())
    }
    async fn delete_branch(&self, branch: &str) -> Result<()> {
        self.log(format!("delete branch {branch}"));
        self.0.borrow_mut().branches.remove(branch);
        Ok(())
    }
    async fn tag(&self, tag: &str, sha: &str) -> Result<()> {
        self.log(format!("tag {tag} -> {sha}"));
        self.0.borrow_mut().tags.insert(tag.into(), sha.into());
        Ok(())
    }
    async fn tags(&self) -> Result<Vec<String>> {
        Ok(self.0.borrow().tags.keys().cloned().collect())
    }
    async fn merge(&self, base: &str, head: &str, _message: &str) -> Result<MergeOutcome> {
        self.log(format!("merge {head} into {base}"));
        let outcome = self
            .0
            .borrow()
            .merge
            .clone()
            .unwrap_or(MergeOutcome::Merged("mm".into()));
        Ok(outcome)
    }
    async fn open_pull(
        &self,
        head: &str,
        base: &str,
        title: &str,
        _body: &str,
    ) -> Result<(u64, String)> {
        self.log(format!("pull {head} -> {base}: {title}"));
        Ok((31, "https://github.com/Aakshar-D/dots/pull/31".into()))
    }
    async fn merged_pulls_since(
        &self,
        _base: Option<&str>,
        _head: &str,
    ) -> Result<Vec<MergedPull>> {
        Ok(self.0.borrow().pulls.clone())
    }
    async fn commits_ahead(&self, _base: &str, _branch: &str) -> Result<u32> {
        Ok(0)
    }
    async fn releases(&self) -> Result<Vec<ReleaseInfo>> {
        Ok(self.0.borrow().releases.clone())
    }
    async fn edit_release(
        &self,
        id: u64,
        name: Option<&str>,
        body: Option<&str>,
        prerelease: Option<bool>,
    ) -> Result<()> {
        self.log(format!("edit release {id}"));
        let mut repo = self.0.borrow_mut();
        let release = repo.releases.iter_mut().find(|r| r.id == id).unwrap();
        if let Some(name) = name {
            release.name = name.into();
        }
        if let Some(body) = body {
            release.body = body.into();
        }
        if let Some(prerelease) = prerelease {
            release.prerelease = prerelease;
        }
        Ok(())
    }
    async fn run_for_tag(&self, _tag: &str) -> Result<Option<RunInfo>> {
        Ok(None)
    }
    async fn rerun(&self, run_id: u64) -> Result<()> {
        self.log(format!("rerun {run_id}"));
        Ok(())
    }
}

fn release(id: u64, tag: &str, prerelease: bool) -> ReleaseInfo {
    ReleaseInfo {
        id,
        tag: tag.into(),
        name: format!("Dots {}", tag.trim_start_matches('v')),
        body: "- Notes".into(),
        prerelease,
        draft: false,
        url: format!("https://github.com/Aakshar-D/dots/releases/tag/{tag}"),
        published_at: Some("2026-10-27T10:00:00Z".into()),
    }
}

const REAL_KEY: &str = "UkVBTEtFWQ==";

#[tokio::test]
async fn setup_is_needed_done_or_half_done() {
    let fake = Fake::new(PROTOTYPE_PUBKEY);
    assert_eq!(setup_state(&fake).await.unwrap(), Setup::Needed);
    fake.0.borrow_mut().secrets.insert(KEY_SECRET.into());
    assert!(matches!(
        setup_state(&fake).await.unwrap(),
        Setup::HalfDone { .. }
    ));
    fake.0.borrow_mut().secrets.insert(PASSWORD_SECRET.into());
    assert!(matches!(
        setup_state(&fake).await.unwrap(),
        Setup::HalfDone { .. }
    ));
    let done = Fake::new(REAL_KEY);
    done.0
        .borrow_mut()
        .secrets
        .extend([KEY_SECRET.to_string(), PASSWORD_SECRET.to_string()]);
    assert_eq!(setup_state(&done).await.unwrap(), Setup::Done);
    let key_only = Fake::new(REAL_KEY);
    assert!(matches!(
        setup_state(&key_only).await.unwrap(),
        Setup::HalfDone { .. }
    ));
}

#[tokio::test]
async fn a_half_set_up_repository_is_left_alone() {
    let fake = Fake::new(REAL_KEY);
    let key = make_key("correct horse battery").unwrap();
    assert!(put_key_in_secrets(&fake, &key, "correct horse battery")
        .await
        .is_err());
    assert!(fake.calls().iter().all(|c| !c.starts_with("put ")));
    // A repository that needs the setup takes both secrets.
    let fresh = Fake::new(PROTOTYPE_PUBKEY);
    put_key_in_secrets(&fresh, &key, "correct horse battery")
        .await
        .unwrap();
    assert_eq!(
        fresh.calls(),
        [
            format!("put {KEY_SECRET}"),
            format!("put {PASSWORD_SECRET}")
        ]
    );
    // The key's PR goes once the secrets are there; never when master has a real key.
    let (n, _) = flow::open_pubkey_pull(&fresh, &key.public_b64)
        .await
        .unwrap();
    assert_eq!(n, 31);
    assert!(fresh
        .calls()
        .iter()
        .any(|c| c == "pull release-key -> master: Use the release signing key"));
    assert!(flow::open_pubkey_pull(&fake, &key.public_b64)
        .await
        .is_err());
}

#[tokio::test]
async fn a_trial_commits_the_versions_and_notes_on_its_branch_and_tags_it() {
    let fake = Fake::new(REAL_KEY);
    let trial = publish_trial(&fake, "0.3.0").await.unwrap();
    assert_eq!((trial.tag.as_str(), trial.rc), ("v0.3.0-rc.1", 1));
    let tagged = fake.0.borrow().tags["v0.3.0-rc.1"].clone();
    assert_eq!(fake.0.borrow().branches["release/0.3.0"], tagged);
    let (parent, tree) = fake.0.borrow().commits[&tagged].clone();
    assert_eq!(parent.as_deref(), Some("c0"));
    for path in VERSION_FILES {
        assert!(tree[path].contains("0.3.0-rc.1"), "{path}");
    }
    assert_eq!(tree[NOTES_FILE], "- The rail's Claude usage bars (#26)\n");
    assert_eq!(tree["README.md"], "readme");
    // A version that is not MAJOR.MINOR.PATCH: refused, nothing written.
    let before = fake.calls().len();
    assert!(publish_trial(&fake, "0.3").await.is_err());
    assert_eq!(fake.calls().len(), before);
}

#[tokio::test]
async fn a_second_trial_takes_the_next_number_from_masters_new_head() {
    let fake = Fake::new(REAL_KEY);
    publish_trial(&fake, "0.3.0").await.unwrap();
    fake.push_master();
    let second = publish_trial(&fake, "0.3.0").await.unwrap();
    assert_eq!(second.tag, "v0.3.0-rc.2");
    let tagged = fake.0.borrow().tags["v0.3.0-rc.2"].clone();
    let (parent, tree) = fake.0.borrow().commits[&tagged].clone();
    assert_eq!(parent, Some(fake.resolve("master").unwrap()));
    assert_eq!(tree["README.md"], "newer");
}

#[tokio::test]
async fn saving_notes_changes_only_the_notes_file_and_the_release_body() {
    let fake = Fake::new(REAL_KEY);
    publish_trial(&fake, "0.3.0").await.unwrap();
    fake.0
        .borrow_mut()
        .releases
        .push(release(9, "v0.3.0-rc.1", true));
    let before = fake.tree("release/0.3.0");
    save_notes(&fake, "0.3.0", "- Edited").await.unwrap();
    let after = fake.tree("release/0.3.0");
    for (path, text) in &before {
        if path == NOTES_FILE {
            assert_eq!(after[path], "- Edited");
        } else {
            assert_eq!(&after[path], text, "{path}");
        }
    }
    assert_eq!(fake.0.borrow().releases[0].body, "- Edited");
}

#[tokio::test]
async fn promote_ships_the_trial_tree_and_stops_on_a_conflicting_merge() {
    let fake = Fake::new(REAL_KEY);
    publish_trial(&fake, "0.3.0").await.unwrap();
    fake.push_master();
    let trial_sha = fake.resolve("release/0.3.0").unwrap();
    promote(&fake, "0.3.0").await.unwrap();
    let tagged = fake.0.borrow().tags["v0.3.0"].clone();
    let (parent, tree) = fake.0.borrow().commits[&tagged].clone();
    assert_eq!(parent, Some(trial_sha.clone()));
    let trial_tree = fake.tree(&trial_sha);
    for (path, text) in &tree {
        if VERSION_FILES.contains(&path.as_str()) {
            assert!(
                text.contains("\"0.3.0\"") || text.contains("version = \"0.3.0\""),
                "{path}"
            );
        } else {
            assert_eq!(text, &trial_tree[path], "{path}");
        }
    }
    assert!(fake
        .calls()
        .iter()
        .any(|c| c == "merge release/0.3.0 into master"));

    let conflicted = Fake::new(REAL_KEY);
    publish_trial(&conflicted, "0.3.0").await.unwrap();
    conflicted.0.borrow_mut().merge = Some(MergeOutcome::Conflict);
    let err = promote(&conflicted, "0.3.0").await.unwrap_err().to_string();
    assert!(
        err.contains(
            "master could not take 0.3.0: its version fields changed. Publish a new trial."
        ),
        "{err}"
    );
    assert!(!conflicted.0.borrow().tags.contains_key("v0.3.0"));
}

#[tokio::test]
async fn drop_and_withdraw_mark_the_release_withdrawn() {
    let fake = Fake::new(REAL_KEY);
    publish_trial(&fake, "0.3.0").await.unwrap();
    fake.0
        .borrow_mut()
        .releases
        .push(release(9, "v0.3.0-rc.1", true));
    flow::drop_trial(&fake, "0.3.0").await.unwrap();
    let dropped = fake.0.borrow().releases[0].clone();
    assert!(dropped.prerelease);
    assert_eq!(dropped.name, "Dots 0.3.0-rc.1 (withdrawn)");
    assert!(dropped.body.starts_with(WITHDRAWN_MARKER));
    assert!(!fake.0.borrow().branches.contains_key("release/0.3.0"));

    let live = release(10, "v0.2.0", false);
    fake.0.borrow_mut().releases.push(live.clone());
    flow::withdraw(&fake, &live).await.unwrap();
    let withdrawn = fake.0.borrow().releases[1].clone();
    assert!(withdrawn.prerelease);
    assert_eq!(withdrawn.name, "Dots 0.2.0 (withdrawn)");
    assert_eq!(withdrawn.body, format!("{WITHDRAWN_MARKER}\n- Notes"));
    // The page no longer counts it as live.
    let page = page_state(&fake).await.unwrap();
    assert!(page.live.is_none());
}

fn job(
    name: &str,
    status: &str,
    conclusion: Option<&str>,
    steps: &[(&str, &str, Option<&str>)],
) -> JobInfo {
    JobInfo {
        name: name.into(),
        status: status.into(),
        conclusion: conclusion.map(str::to_string),
        steps: steps
            .iter()
            .map(|(n, s, c)| (n.to_string(), s.to_string(), c.map(str::to_string)))
            .collect(),
    }
}

fn run(status: &str, jobs: Vec<JobInfo>) -> RunInfo {
    RunInfo {
        id: 7,
        status: status.into(),
        conclusion: None,
        url: "u".into(),
        jobs,
    }
}

#[test]
fn the_stage_follows_the_run_and_the_release() {
    assert_eq!(stage_of(None, None), Stage::Waiting);
    assert_eq!(stage_of(Some(&run("queued", vec![])), None), Stage::Waiting);
    let testing = run("in_progress", vec![job("test", "in_progress", None, &[])]);
    assert_eq!(stage_of(Some(&testing), None), Stage::Testing);
    let building = run(
        "in_progress",
        vec![
            job("test", "completed", Some("success"), &[]),
            job(
                "release",
                "in_progress",
                None,
                &[("Build the installer", "in_progress", None)],
            ),
        ],
    );
    assert_eq!(stage_of(Some(&building), None), Stage::Building);
    let signing = run(
        "in_progress",
        vec![job(
            "release",
            "in_progress",
            None,
            &[("Sign the installer", "in_progress", None)],
        )],
    );
    assert_eq!(stage_of(Some(&signing), None), Stage::Signing);
    let failed = run(
        "completed",
        vec![job(
            "test",
            "completed",
            Some("failure"),
            &[("Run the core tests", "completed", Some("failure"))],
        )],
    );
    assert_eq!(
        stage_of(Some(&failed), None),
        Stage::Failed {
            step: "Run the core tests".into()
        }
    );
    let published = release(9, "v0.3.0-rc.1", true);
    assert_eq!(stage_of(Some(&failed), Some(&published)), Stage::Published);
}
