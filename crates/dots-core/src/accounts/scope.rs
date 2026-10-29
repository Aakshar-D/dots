//! The account and workspace the app is showing (accounts design §9). The account is
//! `personal` or a joined org; every account has a `default` workspace, and may have more.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::workspaces::visible_workspaces;
use crate::store::Store;
use crate::vault::org::Orgs;
use crate::{Error, Result};

/// The account of this PC's own dots.
pub const PERSONAL: &str = "personal";
/// The workspace every account has.
pub const DEFAULT_WORKSPACE: &str = "default";

const SCOPE_MARKER: &str = "current_scope";

/// The account and workspace in view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurrentScope {
    pub account: String,
    pub workspace: String,
}

impl Default for CurrentScope {
    fn default() -> Self {
        CurrentScope {
            account: PERSONAL.into(),
            workspace: DEFAULT_WORKSPACE.into(),
        }
    }
}

async fn is_account(orgs: &Orgs, id: &str) -> bool {
    id == PERSONAL || orgs.summaries().await.iter().any(|o| o.id == id)
}

async fn is_visible(store: &Store, orgs: &Orgs, scope: &CurrentScope) -> Result<bool> {
    let seen = visible_workspaces(store, orgs, &scope.account).await?;
    Ok(seen.iter().any(|w| w.id == scope.workspace))
}

/// The stored scope; personal when none is stored, it does not parse, or its org is gone. A
/// workspace that is gone or no longer visible falls back to the account's `default`. A fallback
/// is not written back.
pub async fn current_scope(store: &Store, orgs: &Orgs) -> Result<CurrentScope> {
    // Only a stored value that does not parse falls back; a failing store is an error.
    let stored = match store.marker(SCOPE_MARKER).await {
        Ok(v) => v.and_then(|v| serde_json::from_value::<CurrentScope>(v).ok()),
        Err(Error::Json(_)) => None,
        Err(e) => return Err(e),
    };
    match stored {
        Some(s) if is_account(orgs, &s.account).await => {
            if is_visible(store, orgs, &s).await? {
                return Ok(s);
            }
            Ok(CurrentScope {
                workspace: DEFAULT_WORKSPACE.into(),
                ..s
            })
        }
        _ => Ok(CurrentScope::default()),
    }
}

/// Chooses the account and workspace in view.
pub async fn set_current_scope(
    store: &Store,
    orgs: &Orgs,
    account: &str,
    workspace: &str,
) -> Result<CurrentScope> {
    if !is_account(orgs, account).await {
        return Err(Error::Invalid(format!("no account '{account}' on this PC")));
    }
    let scope = CurrentScope {
        account: account.into(),
        workspace: workspace.into(),
    };
    if !is_visible(store, orgs, &scope).await? {
        return Err(Error::Invalid(format!(
            "no workspace '{workspace}' in this account"
        )));
    }
    store.set_marker(SCOPE_MARKER, &json!(scope), false).await?;
    Ok(scope)
}
