//! Publishing from this Dots (updates UI spec section 5): whether it is a publisher, the key
//! made for the one-time setup (kept in memory until it is in GitHub's secrets), and the
//! commands' access to the repository with the token from the vault.

use std::sync::{Arc, Mutex};

use dots_core::publish::github::{Access, GitHub};
use dots_core::publish::keys::MadeKey;
use dots_core::Runtime;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::CmdError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PublisherState {
    /// No token chosen.
    None,
    Checking,
    Publisher {
        login: String,
    },
    Refused {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublisherStatus {
    /// The vault entry chosen for publishing.
    pub token: Option<String>,
    pub state: PublisherState,
}

/// A pull request the setup opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullOpened {
    pub number: u64,
    pub url: String,
}

pub const NOT_A_PUBLISHER: &str = "Dots is not set up to publish.";

/// Tauri-managed.
pub struct Publisher {
    status: Mutex<PublisherStatus>,
    /// The key made by setup step 1, until step 3 has put it in GitHub's secrets.
    pending_key: Mutex<Option<MadeKey>>,
    /// Its public half, kept for step 4 (not a secret).
    public_key: Mutex<Option<String>>,
}

impl Default for Publisher {
    fn default() -> Self {
        Self {
            status: Mutex::new(PublisherStatus {
                token: None,
                state: PublisherState::None,
            }),
            pending_key: Mutex::new(None),
            public_key: Mutex::new(None),
        }
    }
}

/// The state a token's access gives: `None` without a token; a publisher, or why not.
pub fn state_of(token: Option<&str>, access: Option<dots_core::Result<Access>>) -> PublisherState {
    match (token, access) {
        (None, _) | (_, None) => PublisherState::None,
        (Some(_), Some(Err(e))) => PublisherState::Refused {
            reason: e.to_string(),
        },
        (Some(_), Some(Ok(access))) => match access.publisher_problem() {
            None => PublisherState::Publisher {
                login: access.login,
            },
            Some(reason) => PublisherState::Refused {
                reason: reason.to_string(),
            },
        },
    }
}

/// After step 3: the key is dropped once it is in the secrets, kept when that failed.
pub fn after_put(slot: &Mutex<Option<MadeKey>>, put: &Result<(), CmdError>) {
    if put.is_ok() {
        *slot.lock().unwrap() = None;
    }
}

fn publisher(app: &AppHandle) -> Result<tauri::State<'_, Publisher>, CmdError> {
    app.try_state::<Publisher>()
        .ok_or_else(|| CmdError::internal("Dots did not start; see the startup error"))
}

pub fn status(app: &AppHandle) -> Result<PublisherStatus, CmdError> {
    Ok(publisher(app)?.status.lock().unwrap().clone())
}

pub fn is_publisher(app: &AppHandle) -> bool {
    status(app).is_ok_and(|s| matches!(s.state, PublisherState::Publisher { .. }))
}

/// Checks the chosen token's access again.
pub async fn check(app: &AppHandle, rt: &Arc<Runtime>) -> Result<PublisherStatus, CmdError> {
    let name = rt.settings().await.publish_token;
    let token = rt.publish_token().await?;
    let access = match &token {
        Some(token) => Some(match GitHub::new(dots_core::releases::API, token) {
            Ok(gh) => gh.access().await,
            Err(e) => Err(e),
        }),
        None => None,
    };
    let state = state_of(token.as_deref(), access);
    let next = PublisherStatus { token: name, state };
    *publisher(app)?.status.lock().unwrap() = next.clone();
    Ok(next)
}

/// The client for a publish command: only for a publisher, with the token read now.
pub async fn hub(app: &AppHandle, rt: &Arc<Runtime>) -> Result<GitHub, CmdError> {
    if !is_publisher(app) {
        return Err(dots_core::Error::Conflict(NOT_A_PUBLISHER.into()).into());
    }
    let token = rt
        .publish_token()
        .await?
        .ok_or_else(|| CmdError::from(dots_core::Error::Conflict(NOT_A_PUBLISHER.into())))?;
    Ok(GitHub::new(dots_core::releases::API, &token)?)
}

/// Setup step 1: the key pair made, kept in memory; its public half for the page.
pub fn keep_key(app: &AppHandle, made: MadeKey) -> Result<String, CmdError> {
    let public = made.public_b64.clone();
    let p = publisher(app)?;
    *p.pending_key.lock().unwrap() = Some(made);
    *p.public_key.lock().unwrap() = Some(public.clone());
    Ok(public)
}

/// Setup step 2: the private key written where the person chooses. `None` when cancelled.
pub fn save_key(app: &AppHandle) -> Result<Option<String>, CmdError> {
    use tauri_plugin_dialog::DialogExt;
    let private = publisher(app)?
        .pending_key
        .lock()
        .unwrap()
        .as_ref()
        .map(|k| k.private_b64.clone())
        .ok_or_else(|| {
            CmdError::from(dots_core::Error::Conflict(
                "Make the signing key first.".into(),
            ))
        })?;
    let Some(path) = app
        .dialog()
        .file()
        .set_file_name("dots-updates.key")
        .blocking_save_file()
    else {
        return Ok(None);
    };
    let path = path
        .into_path()
        .map_err(|e| CmdError::internal(format!("that place cannot be written: {e}")))?;
    std::fs::write(&path, private)
        .map_err(|e| CmdError::internal(format!("could not write {}: {e}", path.display())))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Setup step 3: both secrets; the key is forgotten once they are on GitHub.
pub async fn put_secrets(
    app: &AppHandle,
    rt: &Arc<Runtime>,
    password: &str,
) -> Result<(), CmdError> {
    let gh = hub(app, rt).await?;
    let p = publisher(app)?;
    let key = p.pending_key.lock().unwrap().clone().ok_or_else(|| {
        CmdError::from(dots_core::Error::Conflict(
            "Make the signing key first.".into(),
        ))
    })?;
    let put = dots_core::publish::flow::put_key_in_secrets(&gh, &key, password)
        .await
        .map_err(CmdError::from);
    after_put(&p.pending_key, &put);
    put
}

pub const NO_PUBLIC_KEY: &str = "This Dots did not make the key in GitHub's secrets, or has restarted since. Ask whoever set it up for the public key, or remove both secrets on GitHub to start again.";

/// The public key step 4 needs: only this Dots' own, made in this run.
pub fn public_key_for_pull(kept: Option<String>) -> Result<String, CmdError> {
    kept.ok_or_else(|| dots_core::Error::Conflict(NO_PUBLIC_KEY.into()).into())
}

/// Setup step 4: the pull request that puts the public key into Dots.
pub async fn open_key_pull(app: &AppHandle, rt: &Arc<Runtime>) -> Result<PullOpened, CmdError> {
    let gh = hub(app, rt).await?;
    let public = public_key_for_pull(publisher(app)?.public_key.lock().unwrap().clone())?;
    let (number, url) = dots_core::publish::flow::open_pubkey_pull(&gh, &public).await?;
    Ok(PullOpened { number, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(push: bool, scopes: Option<&[&str]>) -> dots_core::Result<Access> {
        Ok(Access {
            login: "shraddesh".into(),
            push,
            scopes: scopes.map(|s| s.iter().map(|x| x.to_string()).collect()),
        })
    }

    #[test]
    fn the_publisher_state_follows_the_token() {
        assert_eq!(state_of(None, None), PublisherState::None);
        assert_eq!(
            state_of(Some("t"), Some(access(true, Some(&["repo", "workflow"])))),
            PublisherState::Publisher {
                login: "shraddesh".into()
            }
        );
        assert_eq!(
            state_of(Some("t"), Some(access(true, Some(&["repo"])))),
            PublisherState::Refused {
                reason: "This token needs the repo and workflow scopes.".into()
            }
        );
        assert_eq!(
            state_of(
                Some("t"),
                Some(Err(dots_core::Error::Other(
                    "GitHub answered 401: Bad credentials".into()
                )))
            ),
            PublisherState::Refused {
                reason: "GitHub answered 401: Bad credentials".into()
            }
        );
    }

    #[test]
    fn a_made_key_is_kept_until_it_is_in_the_secrets_and_then_dropped() {
        let slot = Mutex::new(Some(dots_core::publish::keys::MadeKey {
            private_b64: "p".into(),
            public_b64: "q".into(),
        }));
        after_put(
            &slot,
            &Err(CmdError::internal("GitHub could not be reached")),
        );
        assert!(slot.lock().unwrap().is_some());
        after_put(&slot, &Ok(()));
        assert!(slot.lock().unwrap().is_none());
    }

    // Final review C1: step 4 after a restart, or in a Dots that did not make the key, says why.
    #[test]
    fn step_four_needs_this_dots_own_public_key() {
        assert_eq!(public_key_for_pull(Some("UFVC".into())).unwrap(), "UFVC");
        assert_eq!(
            public_key_for_pull(None).unwrap_err().message,
            NO_PUBLIC_KEY
        );
    }
}
