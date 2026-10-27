//! The person's side of the apps (app library spec sections 5, 6.2 and 10): the rows of the
//! Apps screen, the screen's calls, an app's or a proposal's files for the frame and the Code
//! tab, and the call log.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::call::Actor;
use super::registry::blocking;
use super::source::{self, AppSource};
use super::{AppCall, Apps};
use crate::library::LibraryItem;
use crate::model::{FileDiff, FileStatus, WorktreeDiff};
use crate::{Error, Result};

/// The most calls `calls` gives at once.
pub const MAX_CALLS_LISTED: u32 = 500;

/// An installed app as the Apps screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppRow {
    pub item: LibraryItem,
    /// The names of the dots that use it.
    pub dots: Vec<String>,
    /// Why its code cannot be read (spec section 11): it is left out of runs until it can.
    pub error: Option<String>,
}

/// One file of an app: its text when it is UTF-8, else its bytes in base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppFile {
    /// Relative to the app's folder, with `/`.
    pub path: String,
    pub text: Option<String>,
    pub base64: Option<String>,
}

fn files_of(src: &AppSource) -> Vec<AppFile> {
    let files: BTreeMap<String, Vec<u8>> = src.files();
    files
        .into_iter()
        .map(|(path, bytes)| match String::from_utf8(bytes) {
            Ok(text) => AppFile {
                path,
                text: Some(text),
                base64: None,
            },
            Err(e) => AppFile {
                path,
                text: None,
                base64: Some(base64::engine::general_purpose::STANDARD.encode(e.into_bytes())),
            },
        })
        .collect()
}

/// How a version's files became another's (spec section 6.2, the Code tab): each file that is
/// new, gone or changed, by path, as a unified diff with three lines of context. A file that is
/// not UTF-8 on either side is listed as binary, without a patch.
pub fn diff_files(
    old: &BTreeMap<String, Vec<u8>>,
    new: &BTreeMap<String, Vec<u8>>,
) -> WorktreeDiff {
    let paths: std::collections::BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    let mut files = Vec::new();
    for path in paths {
        let (before, after) = (old.get(path), new.get(path));
        let status = match (before, after) {
            (None, Some(_)) => FileStatus::Added,
            (Some(_), None) => FileStatus::Deleted,
            (Some(a), Some(b)) if a != b => FileStatus::Modified,
            _ => continue,
        };
        let empty = Vec::new();
        let texts = (
            std::str::from_utf8(before.unwrap_or(&empty)),
            std::str::from_utf8(after.unwrap_or(&empty)),
        );
        let (patch, binary) = match texts {
            (Ok(a), Ok(b)) => {
                let patch = similar::TextDiff::from_lines(a, b)
                    .unified_diff()
                    .context_radius(3)
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string();
                (patch, false)
            }
            _ => (String::new(), true),
        };
        files.push(FileDiff {
            path: path.clone(),
            status,
            patch,
            binary,
        });
    }
    WorktreeDiff {
        files,
        truncated: false,
        exists: true,
    }
}

impl Apps {
    /// Every installed app with the dots that use it and the error that keeps it out of runs.
    /// A dot whose policy denies every tool of the app does not use it; while the code cannot
    /// be read, every dot that ticks it is named.
    pub async fn rows(&self) -> Result<Vec<AppRow>> {
        let mut rows = Vec::new();
        for (item, config) in self.list().await? {
            let (users, error) = match self.load(&item, &config).await {
                Ok(src) => (self.users(&item, &src.manifest).await?, None),
                Err(e) => (self.store.dots_with_app(&item).await?, Some(e.to_string())),
            };
            let dots = users.into_iter().map(|d| d.spec.name).collect();
            rows.push(AppRow { item, dots, error });
        }
        Ok(rows)
    }

    /// A call from the app's screen (spec section 5): the actor is always the person, and only
    /// a listed tool can be called. The rule the call breaks is `Invalid`.
    pub async fn screen_call(&self, name: &str, tool: &str, input: Value) -> Result<Value> {
        self.call(name, tool, input, Actor::Person)
            .await
            .map_err(Error::Invalid)
    }

    /// The active version's files of the app `name`.
    pub async fn files(&self, name: &str) -> Result<Vec<AppFile>> {
        let (item, config) = self.get(name).await?;
        let src = self.load(&item, &config).await?;
        Ok(files_of(&src))
    }

    /// A proposal's files, for the card's Code tab (spec section 6.2). A decided proposal has
    /// none left: `NotFound`. Only a stored proposal's id names a folder: the UI passes ids,
    /// not paths, so `../<app id>/code/1` is no proposal.
    pub async fn proposal_files(&self, id: &str) -> Result<Vec<AppFile>> {
        let p = self.store.get_app_proposal(id).await?;
        let dir = self.proposed_dir(&p.id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("the files of app proposal {id}")));
        }
        let src = blocking(move || source::read_folder(&dir)).await??;
        Ok(files_of(&src))
    }

    /// The Code tab's diff of the waiting proposal `id` (spec section 6.2): against the app's
    /// active version for a change, every file added for a new app. A decided proposal has no
    /// files left: `NotFound`.
    pub async fn proposal_diff(&self, id: &str) -> Result<WorktreeDiff> {
        let p = self.store.get_app_proposal(id).await?;
        let dir = self.proposed_dir(&p.id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("the files of app proposal {id}")));
        }
        let next = blocking(move || source::read_folder(&dir)).await??.files();
        let current = match (&p.app_id, self.get(&p.app_name).await) {
            (Some(app_id), Ok((item, config))) if item.id == *app_id => {
                self.load(&item, &config).await?.files()
            }
            _ => BTreeMap::new(),
        };
        Ok(diff_files(&current, &next))
    }

    /// The newest `limit` logged calls of the app `name` (at most 500), newest first.
    pub async fn calls(&self, name: &str, limit: u32) -> Result<Vec<AppCall>> {
        let (item, _) = self.get(name).await?;
        self.store
            .app_calls(&item.id, limit.min(MAX_CALLS_LISTED))
            .await
    }
}
