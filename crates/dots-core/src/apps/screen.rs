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

impl Apps {
    /// Every installed app with the dots that use it and the error that keeps it out of runs.
    pub async fn rows(&self) -> Result<Vec<AppRow>> {
        let mut rows = Vec::new();
        for (item, config) in self.list().await? {
            let dots = self
                .store
                .dots_with_app(&item)
                .await?
                .into_iter()
                .map(|d| d.spec.name)
                .collect();
            let error = self.load(&item, &config).await.err().map(|e| e.to_string());
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
    /// none left: `NotFound`.
    pub async fn proposal_files(&self, id: &str) -> Result<Vec<AppFile>> {
        let dir = self.proposed_dir(id);
        if !dir.is_dir() {
            return Err(Error::NotFound(format!("the files of app proposal {id}")));
        }
        let src = blocking(move || source::read_folder(&dir)).await??;
        Ok(files_of(&src))
    }

    /// The newest `limit` logged calls of the app `name` (at most 500), newest first.
    pub async fn calls(&self, name: &str, limit: u32) -> Result<Vec<AppCall>> {
        let (item, _) = self.get(name).await?;
        self.store
            .app_calls(&item.id, limit.min(MAX_CALLS_LISTED))
            .await
    }
}
