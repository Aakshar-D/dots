//! Personal variables (env vault spec §5.1): plain config, shown and edited, or a secret, which
//! is written once and never shown again.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

str_enum!(EntryKind {
    Plain => "plain",
    Secret => "secret",
});

/// The longest variable name.
pub const MAX_VARIABLE_NAME: usize = 128;
/// The longest plain value, in bytes. A secret's limit is `secrets::MAX_SECRET_BYTES`.
pub const MAX_PLAIN_BYTES: usize = 32 * 1024;

/// A personal variable as the Env tab shows it. A secret's `value` is always `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvEntry {
    pub name: String,
    pub kind: EntryKind,
    pub value: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A variable name: a letter or `_`, then letters, digits or `_`, at most 128 characters.
pub fn check_variable_name(name: &str) -> Result<()> {
    if !crate::mcp::vars::is_var_name(name) || name.len() > MAX_VARIABLE_NAME {
        return Err(Error::Invalid(format!(
            "variable name {name:?}: use 1 to {MAX_VARIABLE_NAME} characters of A-Z, a-z, 0-9 \
             or '_', starting with a letter or '_'"
        )));
    }
    Ok(())
}

/// A value a child process can be given: no NUL, a secret not empty, a plain value at most
/// `MAX_PLAIN_BYTES`.
pub fn check_value(kind: EntryKind, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(Error::Invalid("a value cannot hold a NUL character".into()));
    }
    match kind {
        EntryKind::Secret if value.is_empty() => {
            Err(Error::Invalid("a secret needs a value".into()))
        }
        EntryKind::Plain if value.len() > MAX_PLAIN_BYTES => Err(Error::Invalid(format!(
            "a plain value is at most {} KB",
            MAX_PLAIN_BYTES / 1024
        ))),
        _ => Ok(()),
    }
}
