//! An app's `app.json` (app library spec section 4): its name, the tools dots and its screen
//! call, what it may reach outside its data, and the function Dots calls when a run it started
//! ends.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::schema;
use crate::{Error, Result};

/// The longest app name. With the longest tool name it keeps `mcp__dots__<app>__<tool>` within
/// the 64 characters a Claude tool name may have.
pub const MAX_APP_NAME: usize = 20;
/// The longest tool name (see `MAX_APP_NAME`).
pub const MAX_TOOL_NAME: usize = 31;
/// The most tools an app lists (spec section 4).
pub const MAX_TOOLS: usize = 200;
/// The largest `app.json` (spec section 4).
pub const MAX_MANIFEST_BYTES: usize = 32 * 1024;
pub const MAX_TITLE: usize = 60;
pub const MAX_DESCRIPTION: usize = 300;
pub const MAX_TOOL_DESCRIPTION: usize = 1000;

/// What an app may reach outside its data. v1 has one grant (spec section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Start runs of the dots that use the app (spec section 7); off until the person switches
    /// it on.
    StartDots,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// The input's JSON Schema (`schema::check_input_schema`).
    pub input: Value,
    /// Whether the tool can change the app's data: policy rules `App(<app>.write)` and the call
    /// log use it.
    #[serde(default)]
    pub writes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// An exported function Dots calls when a run the app started ends (spec section 7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_run_end: Option<String>,
}

impl Manifest {
    /// Reads and checks `app.json`'s text.
    pub fn parse(text: &str) -> Result<Manifest> {
        if text.len() > MAX_MANIFEST_BYTES {
            return invalid(format!(
                "app.json is {} bytes; the limit is {MAX_MANIFEST_BYTES}",
                text.len()
            ));
        }
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|e| Error::Invalid(format!("app.json: {e}")))?;
        manifest.check()?;
        Ok(manifest)
    }

    /// The rules of spec section 4 that `serde` does not enforce.
    pub fn check(&self) -> Result<()> {
        check_app_name(&self.name)?;
        let title = self.title.trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE {
            return invalid(format!(
                "app.json: title must be 1 to {MAX_TITLE} characters"
            ));
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return invalid(format!(
                "app.json: description must be at most {MAX_DESCRIPTION} characters"
            ));
        }
        if self.tools.len() > MAX_TOOLS {
            return invalid(format!("app.json: at most {MAX_TOOLS} tools"));
        }
        let mut seen = std::collections::BTreeSet::new();
        for tool in &self.tools {
            check_tool_name(&tool.name)?;
            if !seen.insert(tool.name.as_str()) {
                return invalid(format!("app.json: the tool {} is listed twice", tool.name));
            }
            let len = tool.description.trim().chars().count();
            if len == 0 || len > MAX_TOOL_DESCRIPTION {
                return invalid(format!(
                    "app.json: tool {}: description must be 1 to {MAX_TOOL_DESCRIPTION} characters",
                    tool.name
                ));
            }
            schema::check_input_schema(&tool.input)
                .map_err(|e| Error::Invalid(format!("app.json: tool {}: {e}", tool.name)))?;
        }
        if let Some(f) = &self.on_run_end {
            check_function_name(f).map_err(|_| {
                Error::Invalid(format!("app.json: on_run_end {f:?} is not a function name"))
            })?;
            if !self.permissions.contains(&Permission::StartDots) {
                return invalid("app.json: on_run_end needs the permission \"start_dots\"");
            }
            if seen.contains(f.as_str()) {
                return invalid(format!(
                    "app.json: on_run_end {f:?} is also a tool; Dots alone calls it"
                ));
            }
        }
        Ok(())
    }

    pub fn tool(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn can_start_dots(&self) -> bool {
        self.permissions.contains(&Permission::StartDots)
    }
}

fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// A lower-case name: a letter, then letters, digits and single `_`, not ending in `_`.
fn check_word(name: &str, max: usize) -> std::result::Result<(), ()> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    let rest_ok = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if first_ok && rest_ok && name.len() <= max && !name.contains("__") && !name.ends_with('_') {
        Ok(())
    } else {
        Err(())
    }
}

/// An app's name: 1 to 20 characters of a-z, 0-9 and `_`, starting with a letter; no `__` (the
/// tool names `<app>__<tool>` are split there) and no trailing `_`.
pub fn check_app_name(name: &str) -> Result<()> {
    check_word(name, MAX_APP_NAME).map_err(|_| {
        Error::Invalid(format!(
            "app name {name:?}: use 1 to {MAX_APP_NAME} characters of a-z, 0-9 and '_', starting \
             with a letter, with no '__' and no '_' at the end"
        ))
    })
}

/// A tool's name: as an app's, up to 31 characters.
pub fn check_tool_name(name: &str) -> Result<()> {
    check_word(name, MAX_TOOL_NAME).map_err(|_| {
        Error::Invalid(format!(
            "tool name {name:?}: use 1 to {MAX_TOOL_NAME} characters of a-z, 0-9 and '_', \
             starting with a letter, with no '__' and no '_' at the end"
        ))
    })
}

fn check_function_name(name: &str) -> std::result::Result<(), ()> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && name.len() <= 64 {
        Ok(())
    } else {
        Err(())
    }
}

/// The name a tool is offered by, on both engines: `<app>__<tool>` (spec section 6).
pub fn tool_key(app: &str, tool: &str) -> String {
    format!("{app}__{tool}")
}

/// The app and tool of a `tool_key`.
pub fn split_tool_key(key: &str) -> Option<(&str, &str)> {
    key.split_once("__")
}
