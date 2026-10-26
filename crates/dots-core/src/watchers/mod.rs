//! Watchers (watchers spec): what starts a dot when a file changes in a folder, an email
//! arrives in a Gmail label, or something happens on a GitHub repository. This module holds the
//! checks a watcher passes before it is saved, with the spec's sentences.

use std::path::Path;

use crate::model::{EmailConfig, FolderConfig, GithubConfig, WatcherConfig};
use crate::store::Store;
use crate::vault::EntryKind;
use crate::{Error, Result};

/// A watcher's name is 1 to 80 characters (spec section 3).
pub const MAX_NAME_CHARS: usize = 80;

pub const NAME_RULE: &str = "A watcher's name is 1 to 80 characters.";
pub const NOT_ABSOLUTE: &str = "The folder must be an absolute path.";
pub const FOLDER_EVENTS_RULE: &str = "Choose at least one of created, changed or deleted.";
pub const ADDRESS_RULE: &str = "Enter the Gmail address.";
pub const FROM_RULE: &str = "From is an address or @domain.";
pub const SECRET_RULE: &str = "Choose the vault entry that holds the password or token.";
pub const REPOS_RULE: &str = "Add at least one repository (owner/repo).";
pub const EVENTS_RULE: &str = "Choose at least one event.";
/// Why a flooded watcher paused, as the notification and the Inbox say it.
pub const FLOOD_TEXT: &str = "20 runs in an hour";

pub fn no_folder(path: &str) -> String {
    format!("No folder at {path}.")
}

pub fn bad_pattern(pattern: &str) -> String {
    format!("The pattern {pattern} is not a valid file pattern.")
}

pub fn no_entry(name: &str) -> String {
    format!("The vault has no entry {name}.")
}

pub fn plain_entry(name: &str) -> String {
    format!("{name} is a plain entry; choose a secret one.")
}

pub fn entry_gone(name: &str) -> String {
    format!("The vault entry {name} is gone.")
}

pub fn bad_repo(repo: &str) -> String {
    format!("{repo} is not owner/repo.")
}

pub fn folder_gone(path: &str) -> String {
    format!("The folder {path} no longer exists.")
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The name trimmed, or the rule it breaks.
pub fn check_name(name: &str) -> Result<String> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_NAME_CHARS {
        return Err(invalid(NAME_RULE));
    }
    Ok(name.to_string())
}

/// `None` for a blank text, else the text trimmed.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// The non-blank items, trimmed.
fn items(list: &[String]) -> Vec<String> {
    list.iter()
        .map(|i| i.trim())
        .filter(|i| !i.is_empty())
        .map(str::to_string)
        .collect()
}

/// The vault entry `name` must exist and hold a secret. Its value is never read here.
async fn check_secret(store: &Store, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid(SECRET_RULE));
    }
    match store.find_env_entry(name).await? {
        None => Err(invalid(no_entry(name))),
        Some(entry) if entry.kind != EntryKind::Secret => Err(invalid(plain_entry(name))),
        Some(_) => Ok(name.to_string()),
    }
}

fn check_folder(config: &FolderConfig) -> Result<FolderConfig> {
    let path = config.path.trim();
    if !Path::new(path).is_absolute() {
        return Err(invalid(NOT_ABSOLUTE));
    }
    if !Path::new(path).is_dir() {
        return Err(invalid(no_folder(path)));
    }
    if config.events.is_empty() {
        return Err(invalid(FOLDER_EVENTS_RULE));
    }
    let pattern = config.pattern.trim();
    if !pattern.is_empty() && globset::Glob::new(pattern).is_err() {
        return Err(invalid(bad_pattern(pattern)));
    }
    Ok(FolderConfig {
        path: path.to_string(),
        pattern: pattern.to_string(),
        events: config.events.clone(),
        recursive: config.recursive,
    })
}

async fn check_email(store: &Store, config: &EmailConfig) -> Result<EmailConfig> {
    let address = config.address.trim();
    let at = address.find('@');
    if !matches!(at, Some(i) if i > 0 && i + 1 < address.len()) {
        return Err(invalid(ADDRESS_RULE));
    }
    let secret = check_secret(store, &config.secret).await?;
    let from = optional(&config.from);
    if let Some(from) = &from {
        let domain = from.rsplit_once('@').map(|(_, d)| d);
        if !matches!(domain, Some(d) if !d.is_empty() && !d.contains(char::is_whitespace)) {
            return Err(invalid(FROM_RULE));
        }
    }
    let label = config.label.trim();
    Ok(EmailConfig {
        address: address.to_string(),
        secret,
        label: if label.is_empty() { "INBOX" } else { label }.to_string(),
        from,
        subject: optional(&config.subject),
        has_attachment: config.has_attachment,
    })
}

async fn check_github(store: &Store, config: &GithubConfig) -> Result<GithubConfig> {
    let repos = items(&config.repos);
    if repos.is_empty() {
        return Err(invalid(REPOS_RULE));
    }
    let shape = regex::Regex::new(r"^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$").expect("a valid regex");
    if let Some(bad) = repos.iter().find(|r| !shape.is_match(r)) {
        return Err(invalid(bad_repo(bad)));
    }
    if config.events.is_empty() {
        return Err(invalid(EVENTS_RULE));
    }
    let secret = check_secret(store, &config.secret).await?;
    Ok(GithubConfig {
        repos,
        events: config.events.clone(),
        secret,
        label: optional(&config.label),
        authors: items(&config.authors),
        skip_bots: config.skip_bots,
        branches: items(&config.branches),
    })
}

/// The config as it is saved (spec sections 3 and 4): strings trimmed, an empty label
/// `INBOX`, blank filters dropped; or the rule it breaks, in the spec's words.
pub async fn check_config(store: &Store, config: &WatcherConfig) -> Result<WatcherConfig> {
    Ok(match config {
        WatcherConfig::Folder(c) => WatcherConfig::Folder(check_folder(c)?),
        WatcherConfig::Email(c) => WatcherConfig::Email(check_email(store, c).await?),
        WatcherConfig::Github(c) => WatcherConfig::Github(check_github(store, c).await?),
    })
}
