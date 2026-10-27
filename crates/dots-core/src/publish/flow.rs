//! The Releases page's flows (updates UI spec sections 5.2 to 5.9): the one-time key setup, a
//! trial, its notes, Promote, Retry, Drop trial and Withdraw, and what the page shows. They
//! work through `Hub`, which `GitHub` implements; the tests use a fake repository.

use serde::{Deserialize, Serialize};

use super::github::{GitHub, MergeOutcome, MergedPull, ReleaseInfo, RunInfo};
use super::keys::{self, MadeKey, PROTOTYPE_PUBKEY};
use super::versions::{self, VERSION_FILES};
use crate::releases::WITHDRAWN_MARKER;
use crate::{Error, Result};

pub const KEY_SECRET: &str = "TAURI_SIGNING_PRIVATE_KEY";
pub const PASSWORD_SECRET: &str = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD";
pub const CONF: &str = "apps/dots-app/tauri.conf.json";
pub const NOTES_FILE: &str = "release-notes.md";
pub const KEY_BRANCH: &str = "release-key";
pub const KEY_PULL_TITLE: &str = "Use the release signing key";

/// The repository calls the flows make (`GitHub` is the real one).
#[allow(async_fn_in_trait)]
pub trait Hub {
    async fn secret_exists(&self, name: &str) -> Result<bool>;
    async fn secrets_key(&self) -> Result<(String, String)>;
    async fn put_secret(&self, name: &str, key_id: &str, encrypted_b64: &str) -> Result<()>;
    async fn file(&self, path: &str, reference: &str) -> Result<Option<String>>;
    async fn head(&self, branch: &str) -> Result<String>;
    async fn commit_files(
        &self,
        parent: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String>;
    async fn set_branch(&self, branch: &str, sha: &str, force: bool) -> Result<()>;
    async fn delete_branch(&self, branch: &str) -> Result<()>;
    async fn tag(&self, tag: &str, sha: &str) -> Result<()>;
    async fn tag_sha(&self, tag: &str) -> Result<String>;
    async fn tags(&self) -> Result<Vec<String>>;
    async fn merge(&self, base: &str, head: &str, message: &str) -> Result<MergeOutcome>;
    async fn open_pull(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)>;
    async fn merged_pulls_since(&self, base: Option<&str>, head: &str) -> Result<Vec<MergedPull>>;
    async fn commits_ahead(&self, base: &str, branch: &str) -> Result<u32>;
    async fn releases(&self) -> Result<Vec<ReleaseInfo>>;
    async fn edit_release(
        &self,
        id: u64,
        name: Option<&str>,
        body: Option<&str>,
        prerelease: Option<bool>,
    ) -> Result<()>;
    async fn run_for_tag(&self, tag: &str) -> Result<Option<RunInfo>>;
    async fn rerun(&self, run_id: u64) -> Result<()>;
}

impl Hub for GitHub {
    async fn secret_exists(&self, name: &str) -> Result<bool> {
        GitHub::secret_exists(self, name).await
    }
    async fn secrets_key(&self) -> Result<(String, String)> {
        GitHub::secrets_key(self).await
    }
    async fn put_secret(&self, name: &str, key_id: &str, encrypted_b64: &str) -> Result<()> {
        GitHub::put_secret(self, name, key_id, encrypted_b64).await
    }
    async fn file(&self, path: &str, reference: &str) -> Result<Option<String>> {
        GitHub::file(self, path, reference).await
    }
    async fn head(&self, branch: &str) -> Result<String> {
        GitHub::head(self, branch).await
    }
    async fn commit_files(
        &self,
        parent: &str,
        files: &[(String, String)],
        message: &str,
    ) -> Result<String> {
        GitHub::commit_files(self, parent, files, message).await
    }
    async fn set_branch(&self, branch: &str, sha: &str, force: bool) -> Result<()> {
        GitHub::set_branch(self, branch, sha, force).await
    }
    async fn delete_branch(&self, branch: &str) -> Result<()> {
        GitHub::delete_branch(self, branch).await
    }
    async fn tag(&self, tag: &str, sha: &str) -> Result<()> {
        GitHub::tag(self, tag, sha).await
    }
    async fn tag_sha(&self, tag: &str) -> Result<String> {
        GitHub::tag_sha(self, tag).await
    }
    async fn tags(&self) -> Result<Vec<String>> {
        GitHub::tags(self).await
    }
    async fn merge(&self, base: &str, head: &str, message: &str) -> Result<MergeOutcome> {
        GitHub::merge(self, base, head, message).await
    }
    async fn open_pull(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<(u64, String)> {
        GitHub::open_pull(self, head, base, title, body).await
    }
    async fn merged_pulls_since(&self, base: Option<&str>, head: &str) -> Result<Vec<MergedPull>> {
        GitHub::merged_pulls_since(self, base, head).await
    }
    async fn commits_ahead(&self, base: &str, branch: &str) -> Result<u32> {
        GitHub::commits_ahead(self, base, branch).await
    }
    async fn releases(&self) -> Result<Vec<ReleaseInfo>> {
        GitHub::releases(self).await
    }
    async fn edit_release(
        &self,
        id: u64,
        name: Option<&str>,
        body: Option<&str>,
        prerelease: Option<bool>,
    ) -> Result<()> {
        GitHub::edit_release(self, id, name, body, prerelease).await
    }
    async fn run_for_tag(&self, tag: &str) -> Result<Option<RunInfo>> {
        GitHub::run_for_tag(self, tag).await
    }
    async fn rerun(&self, run_id: u64) -> Result<()> {
        GitHub::rerun(self, run_id).await
    }
}

/// Whether releases are set up: both secrets on GitHub and a real key on master.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Setup {
    Done,
    /// Nothing is there yet: the setup can run.
    Needed,
    /// Both secrets are in and master still has the prototype key: the setup's step 4, the
    /// pull request with the public key, is what is left.
    KeyPending,
    /// Some is there and some is not; the setup never touches it. `secrets`: how many of the
    /// two exist; `pubkey`: master has a key that is not the prototype's.
    HalfDone {
        secrets: u8,
        pubkey: bool,
    },
}

pub const HALF_DONE: &str = "GitHub already has a signing key. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again.";

pub async fn setup_state(h: &impl Hub) -> Result<Setup> {
    let secrets =
        h.secret_exists(KEY_SECRET).await? as u8 + h.secret_exists(PASSWORD_SECRET).await? as u8;
    let pubkey = master_has_real_key(h).await?;
    Ok(match (secrets, pubkey) {
        (2, true) => Setup::Done,
        (0, false) => Setup::Needed,
        (2, false) => Setup::KeyPending,
        (secrets, pubkey) => Setup::HalfDone { secrets, pubkey },
    })
}

async fn master_has_real_key(h: &impl Hub) -> Result<bool> {
    let conf = h.file(CONF, "master").await?.unwrap_or_default();
    Ok(keys::pubkey_in_conf(&conf).is_some_and(|k| k != PROTOTYPE_PUBKEY))
}

/// Setup step 3: both secrets, sealed with the repository's key. Only when nothing is set up.
pub async fn put_key_in_secrets(h: &impl Hub, key: &MadeKey, password: &str) -> Result<()> {
    if setup_state(h).await? != Setup::Needed {
        return Err(Error::Conflict(HALF_DONE.into()));
    }
    let (key_id, public) = h.secrets_key().await?;
    h.put_secret(KEY_SECRET, &key_id, &keys::seal(&public, &key.private_b64)?)
        .await?;
    h.put_secret(PASSWORD_SECRET, &key_id, &keys::seal(&public, password)?)
        .await
}

/// Setup step 4: a pull request that puts the public key into Dots. Its number and page.
pub async fn open_pubkey_pull(h: &impl Hub, public_b64: &str) -> Result<(u64, String)> {
    if master_has_real_key(h).await? {
        return Err(Error::Conflict("master already has a release key.".into()));
    }
    let conf = h
        .file(CONF, "master")
        .await?
        .ok_or_else(|| Error::Other(format!("{CONF} is not on master")))?;
    let master = h.head("master").await?;
    let changed = keys::with_pubkey(&conf, public_b64)?;
    let commit = h
        .commit_files(&master, &[(CONF.to_string(), changed)], KEY_PULL_TITLE)
        .await?;
    h.set_branch(KEY_BRANCH, &commit, true).await?;
    h.open_pull(
        KEY_BRANCH,
        "master",
        KEY_PULL_TITLE,
        "Replaces the prototype's updater key with the release signing key. Made by Dots' Releases page.",
    )
    .await
}

/// A trial: a `-rc.<n>` tag on its version's release branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    pub version: String,
    pub rc: u32,
    pub tag: String,
    pub release: Option<ReleaseInfo>,
    pub run: Option<RunInfo>,
    pub stage: Stage,
    pub newer_on_master: u32,
    pub notes: String,
}

/// A promoted version still building, or whose build failed (Retry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Promotion {
    pub version: String,
    pub tag: String,
    pub run: Option<RunInfo>,
    pub stage: Stage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageState {
    pub setup: Setup,
    pub live: Option<ReleaseInfo>,
    pub trial: Option<Trial>,
    pub promotion: Option<Promotion>,
    pub changes: Vec<MergedPull>,
    pub suggested: String,
}

/// The first line of `release-notes.md`: the version the notes belong to, so a later release
/// never publishes them (the workflow checks it).
fn notes_marker(version: &str) -> String {
    format!("<!-- dots:notes {version} -->")
}

/// The notes as the file holds them: the marker line, then the text.
pub fn mark_notes(version: &str, notes: &str) -> String {
    format!("{}\n{}", notes_marker(version), unmark_notes(notes))
}

/// The notes without the marker line.
pub fn unmark_notes(text: &str) -> String {
    match text.split_once('\n') {
        Some((first, rest)) if first.starts_with("<!-- dots:notes ") => rest.to_string(),
        _ if text.starts_with("<!-- dots:notes ") => String::new(),
        _ => text.to_string(),
    }
}

fn withdrawn(r: &ReleaseInfo) -> bool {
    r.body.trim_start().starts_with(WITHDRAWN_MARKER)
}

fn branch(version: &str) -> String {
    format!("release/{version}")
}

/// A `v<version>-rc.<n>` tag as its version and number.
fn rc_of(tag: &str) -> Option<(String, u32)> {
    let (version, n) = tag.strip_prefix('v')?.split_once("-rc.")?;
    versions::parse(version)?;
    Some((version.to_string(), n.parse().ok()?))
}

fn version_key(v: &str) -> (u64, u64, u64) {
    versions::parse(v).unwrap_or_default()
}

/// What the Releases page shows.
pub async fn page_state(h: &impl Hub) -> Result<PageState> {
    let setup = setup_state(h).await?;
    let releases = h.releases().await?;
    let live = releases
        .iter()
        .find(|r| !r.prerelease && !r.draft && !withdrawn(r))
        .cloned();
    let tags = h.tags().await?;
    let live_version = live
        .as_ref()
        .map(|r| r.tag.trim_start_matches('v').to_string());
    let changes = h
        .merged_pulls_since(live.as_ref().map(|r| r.tag.as_str()), "master")
        .await?;
    let mut suggested = versions::next_minor(live_version.as_deref());
    while tags.iter().any(|t| t == &format!("v{suggested}")) {
        suggested = versions::next_minor(Some(&suggested));
    }

    // The newest trial whose version was never promoted and whose branch still exists.
    let mut rcs: Vec<(String, u32)> = tags.iter().filter_map(|t| rc_of(t)).collect();
    rcs.sort_by_key(|(v, n)| (version_key(v), *n));
    let mut trial = None;
    if let Some((version, rc)) = rcs.last().cloned() {
        let promoted = tags.iter().any(|t| t == &format!("v{version}"));
        if !promoted && h.head(&branch(&version)).await.is_ok() {
            let tag = format!("v{version}-rc.{rc}");
            let release = releases.iter().find(|r| r.tag == tag).cloned();
            if !release.as_ref().is_some_and(withdrawn) {
                let run = h.run_for_tag(&tag).await?;
                let notes = match h.file(NOTES_FILE, &branch(&version)).await? {
                    Some(notes) => unmark_notes(&notes),
                    None => versions::draft_notes(&changes),
                };
                trial = Some(Trial {
                    stage: stage_of(run.as_ref(), release.as_ref()),
                    newer_on_master: h.commits_ahead(&tag, "master").await?,
                    version,
                    rc,
                    tag,
                    release,
                    run,
                    notes,
                });
            }
        }
    }

    // The newest plain tag that has no published release yet: a promotion in progress or
    // failed.
    let newest_plain = tags
        .iter()
        .filter_map(|t| t.strip_prefix('v'))
        .filter(|v| versions::parse(v).is_some())
        .max_by_key(|v| version_key(v))
        .map(str::to_string);
    let mut promotion = None;
    if let Some(version) = newest_plain {
        let tag = format!("v{version}");
        if !releases.iter().any(|r| r.tag == tag && !r.draft) {
            let run = h.run_for_tag(&tag).await?;
            promotion = Some(Promotion {
                stage: stage_of(run.as_ref(), None),
                version,
                tag,
                run,
            });
        }
    }

    Ok(PageState {
        setup,
        live,
        trial,
        promotion,
        changes,
        suggested,
    })
}

/// "Publish trial": master's head with the versions set to `<version>-rc.<n>` and the notes,
/// on `release/<version>`, tagged.
pub async fn publish_trial(h: &impl Hub, version: &str) -> Result<Trial> {
    let tags = h.tags().await?;
    let live = h
        .releases()
        .await?
        .into_iter()
        .find(|r| !r.prerelease && !r.draft && !withdrawn(r));
    let live_version = live
        .as_ref()
        .map(|r| r.tag.trim_start_matches('v').to_string());
    versions::check_new(version, live_version.as_deref(), &tags).map_err(Error::Invalid)?;
    let rc = versions::next_rc(version, &tags);
    let full = format!("{version}-rc.{rc}");
    let master = h.head("master").await?;
    // Notes edited on an earlier trial of this version are kept.
    let notes = match h.file(NOTES_FILE, &branch(version)).await.ok().flatten() {
        Some(notes) => unmark_notes(&notes),
        None => {
            let pulls = h
                .merged_pulls_since(live.as_ref().map(|r| r.tag.as_str()), &master)
                .await?;
            versions::draft_notes(&pulls)
        }
    };
    let mut files = Vec::new();
    for path in VERSION_FILES {
        let text = h
            .file(path, &master)
            .await?
            .ok_or_else(|| Error::Other(format!("{path} is not on master")))?;
        files.push((path.to_string(), versions::set_version(path, &text, &full)?));
    }
    files.push((NOTES_FILE.to_string(), mark_notes(version, &notes)));
    let commit = h
        .commit_files(&master, &files, &format!("Dots {full}"))
        .await?;
    // The tag first: a branch moved without its tag would hold an untested tree.
    let tag = format!("v{full}");
    h.tag(&tag, &commit).await?;
    h.set_branch(&branch(version), &commit, true).await?;
    Ok(Trial {
        version: version.to_string(),
        rc,
        tag,
        release: None,
        run: None,
        stage: Stage::Waiting,
        newer_on_master: 0,
        notes,
    })
}

/// The newest trial tag of `version`.
async fn trial_tag(h: &impl Hub, version: &str) -> Result<String> {
    let rc = versions::next_rc(version, &h.tags().await?) - 1;
    if rc == 0 {
        return Err(Error::NotFound(format!("no trial of {version}")));
    }
    Ok(format!("v{version}-rc.{rc}"))
}

/// "Save notes": only `release-notes.md` changes on the branch; the trial's release body
/// follows.
pub async fn save_notes(h: &impl Hub, version: &str, notes: &str) -> Result<()> {
    let head = h.head(&branch(version)).await?;
    let commit = h
        .commit_files(
            &head,
            &[(NOTES_FILE.to_string(), mark_notes(version, notes))],
            &format!("Release notes for {version}"),
        )
        .await?;
    h.set_branch(&branch(version), &commit, false).await?;
    let tag = trial_tag(h, version).await?;
    if let Some(release) = h.releases().await?.into_iter().find(|r| r.tag == tag) {
        h.edit_release(release.id, None, Some(&unmark_notes(notes)), None)
            .await?;
    }
    Ok(())
}

pub fn conflict_message(version: &str) -> String {
    format!("master could not take {version}: its version fields changed. Publish a new trial.")
}

/// "Promote": the tagged trial's tree with the final version and the notes saved since,
/// merged into master, and tagged. Only a trial that was published (its tests passed and it
/// built) is promoted; a later commit on its branch is never shipped.
pub async fn promote(h: &impl Hub, version: &str) -> Result<()> {
    let tag = trial_tag(h, version).await?;
    let published = h
        .releases()
        .await?
        .into_iter()
        .any(|r| r.tag == tag && !r.draft && !withdrawn(&r));
    if !published {
        return Err(Error::Conflict(format!(
            "Promote waits for {} to be published.",
            tag.trim_start_matches('v')
        )));
    }
    let base = h.tag_sha(&tag).await?;
    let notes = match h.file(NOTES_FILE, &branch(version)).await? {
        Some(notes) => notes,
        None => h.file(NOTES_FILE, &base).await?.unwrap_or_default(),
    };
    let mut files = Vec::new();
    for path in VERSION_FILES {
        let text = h
            .file(path, &base)
            .await?
            .ok_or_else(|| Error::Other(format!("{path} is not in the trial")))?;
        files.push((
            path.to_string(),
            versions::set_version(path, &text, version)?,
        ));
    }
    files.push((NOTES_FILE.to_string(), mark_notes(version, &notes)));
    let commit = h
        .commit_files(&base, &files, &format!("Dots {version}"))
        .await?;
    h.set_branch(&branch(version), &commit, true).await?;
    if h.merge(
        "master",
        &branch(version),
        &format!("Release Dots {version}"),
    )
    .await?
        == MergeOutcome::Conflict
    {
        return Err(Error::Conflict(conflict_message(version)));
    }
    h.tag(&format!("v{version}"), &commit).await
}

/// "Retry": the tag's workflow run again.
pub async fn retry(h: &impl Hub, tag: &str) -> Result<()> {
    let run = h
        .run_for_tag(tag)
        .await?
        .ok_or_else(|| Error::NotFound(format!("no build of {tag}")))?;
    h.rerun(run.id).await
}

/// Marks a release withdrawn: a prerelease (so "latest" skips it) named "(withdrawn)", its
/// body starting with the marker every Dots reads (updates UI spec section 4.5).
pub async fn withdraw(h: &impl Hub, release: &ReleaseInfo) -> Result<()> {
    let version = release.tag.trim_start_matches('v');
    let body = if withdrawn(release) {
        release.body.clone()
    } else {
        format!("{WITHDRAWN_MARKER}\n{}", release.body)
    };
    h.edit_release(
        release.id,
        Some(&format!("Dots {version} (withdrawn)")),
        Some(&body),
        Some(true),
    )
    .await
}

/// "Drop trial": its release withdrawn and its branch gone.
pub async fn drop_trial(h: &impl Hub, version: &str) -> Result<()> {
    let tag = trial_tag(h, version).await?;
    if let Some(release) = h.releases().await?.into_iter().find(|r| r.tag == tag) {
        withdraw(h, &release).await?;
    }
    h.delete_branch(&branch(version)).await
}

/// Where a build is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    Waiting,
    Testing,
    Building,
    Signing,
    Published,
    Failed { step: String },
}

pub fn stage_of(run: Option<&RunInfo>, release: Option<&ReleaseInfo>) -> Stage {
    if release.is_some_and(|r| !r.draft) {
        return Stage::Published;
    }
    let Some(run) = run else {
        return Stage::Waiting;
    };
    if let Some(job) = run.jobs.iter().find(|j| {
        matches!(
            j.conclusion.as_deref(),
            Some("failure" | "cancelled" | "timed_out")
        )
    }) {
        let step = job
            .steps
            .iter()
            .find(|s| s.2.as_deref() == Some("failure"))
            .map(|s| s.0.clone())
            .unwrap_or_else(|| job.name.clone());
        return Stage::Failed { step };
    }
    if run.jobs.is_empty() || run.status == "queued" {
        return Stage::Waiting;
    }
    let running = |j: &&super::github::JobInfo| j.status != "completed";
    if run.jobs.iter().filter(running).any(|j| j.name == "test") {
        return Stage::Testing;
    }
    let signing = run.jobs.iter().any(|j| {
        j.steps
            .iter()
            .any(|s| s.0 == "Sign the installer" && s.1 == "in_progress")
    });
    if signing {
        Stage::Signing
    } else {
        Stage::Building
    }
}
