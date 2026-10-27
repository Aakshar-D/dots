//! Dots' own releases on GitHub (updates UI spec sections 4.1 and 4.5): a version's notes, and
//! whether its developers withdrew it. Read without a token, from the public API.

use std::time::Duration;

use serde::Deserialize;

use crate::{Error, Result};

/// The repository Dots is published from. Not a setting.
pub const REPO: &str = "Aakshar-D/dots";
/// GitHub's API; tests pass a mock server instead.
pub const API: &str = "https://api.github.com";
/// The first line of a withdrawn release's body (part 2 writes it, with the release made a
/// prerelease).
pub const WITHDRAWN_MARKER: &str = "<!-- dots:withdrawn -->";

const TIMEOUT: Duration = Duration::from_secs(15);

/// One version's release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub name: String,
    /// The notes, without the withdrawn marker.
    pub body: String,
    /// The release's page on GitHub.
    pub url: String,
    pub prerelease: bool,
    pub withdrawn: bool,
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
}

/// The release of `version` (without its `v`), `None` when there is none. `api` is
/// `API` outside tests.
pub async fn fetch_release(api: &str, version: &str) -> Result<Option<Release>> {
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("dots/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Other(e.to_string()))?;
    let url = format!("{api}/repos/{REPO}/releases/tags/v{version}");
    let response = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| Error::Other(format!("reading the release of {version} failed: {e}")))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(Error::Other(format!(
            "reading the release of {version} failed: GitHub answered {}",
            response.status()
        )));
    }
    let raw: Raw = response
        .json()
        .await
        .map_err(|e| Error::Other(format!("reading the release of {version} failed: {e}")))?;
    let body = raw.body.unwrap_or_default();
    let marked = body.trim_start().starts_with(WITHDRAWN_MARKER);
    let body = if marked {
        let rest = body.trim_start().trim_start_matches(WITHDRAWN_MARKER);
        rest.trim_start_matches(['\r', '\n']).to_string()
    } else {
        body
    };
    Ok(Some(Release {
        version: version.to_string(),
        name: raw.name.unwrap_or_else(|| format!("Dots {version}")),
        body,
        url: raw.html_url,
        prerelease: raw.prerelease,
        withdrawn: raw.prerelease && marked,
    }))
}
