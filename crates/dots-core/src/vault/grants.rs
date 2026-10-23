//! What a dot is granted (env vault spec §7): for each consumer, the variable names it may
//! resolve. A consumer is one Library MCP server (`mcp:<server>`) or the dot's shell (`shell`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::entries::check_variable_name;
use crate::library::{check_name, LibraryKind};
use crate::{Error, Result};

/// The key of the shell consumer.
pub const SHELL: &str = "shell";
const MCP_PREFIX: &str = "mcp:";

/// Who a variable is resolved for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consumer {
    /// The Library MCP server of this name, started for the dot's run.
    Mcp(String),
    /// The dot's agent process: the `claude` CLI and its Bash tool, or the local engine's
    /// shell tool.
    Shell,
}

impl Consumer {
    /// `mcp:<server>` or `shell`, as `Grants` and messages name it.
    pub fn key(&self) -> String {
        match self {
            Self::Mcp(server) => format!("{MCP_PREFIX}{server}"),
            Self::Shell => SHELL.to_string(),
        }
    }

    pub fn parse(key: &str) -> Result<Consumer> {
        if key == SHELL {
            return Ok(Self::Shell);
        }
        match key.strip_prefix(MCP_PREFIX) {
            Some(server) => {
                check_name(LibraryKind::McpServer, server)?;
                Ok(Self::Mcp(server.to_string()))
            }
            None => Err(Error::Invalid(format!(
                "a grant is for 'shell' or 'mcp:<server>', not {key:?}"
            ))),
        }
    }
}

/// A dot's grants: consumer key to variable names, in the order they were granted. A fixed
/// list: a server that later asks for a new `${VAR}` does not get it until it is granted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grants(pub BTreeMap<String, Vec<String>>);

fn contains(names: &[String], name: &str) -> bool {
    names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

impl Grants {
    /// The names granted to `consumer`.
    pub fn names(&self, consumer: &Consumer) -> &[String] {
        self.0
            .get(&consumer.key())
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Whether `name` is granted to `consumer`, in any case.
    pub fn allows(&self, consumer: &Consumer, name: &str) -> bool {
        contains(self.names(consumer), name)
    }

    /// Each key is a consumer, each name a variable name granted once.
    pub fn validate(&self) -> Result<()> {
        for (key, names) in &self.0 {
            Consumer::parse(key)?;
            let mut seen: Vec<String> = Vec::new();
            for name in names {
                check_variable_name(name)?;
                if contains(&seen, name) {
                    return Err(Error::Invalid(format!("{key} is granted {name} twice")));
                }
                seen.push(name.clone());
            }
        }
        Ok(())
    }

    /// The grants as a save stores them (spec §7): the shell's as given; a server the dot uses
    /// keeps the names given for it, and a server it did not use before (`had` lists those it
    /// did) is also granted every variable its config reads now (`using`: each server with
    /// its variables); a server it no longer uses loses its grants.
    pub fn synced(&self, had: &[String], using: &[(String, Vec<String>)]) -> Grants {
        let mut out = BTreeMap::new();
        if let Some(shell) = self.0.get(SHELL) {
            if !shell.is_empty() {
                out.insert(SHELL.to_string(), shell.clone());
            }
        }
        for (server, variables) in using {
            let key = Consumer::Mcp(server.clone()).key();
            let mut names = self.0.get(&key).cloned().unwrap_or_default();
            if !had.contains(server) {
                for variable in variables {
                    // A name Personal cannot hold never resolves, and `validate` would refuse it.
                    if check_variable_name(variable).is_ok() && !contains(&names, variable) {
                        names.push(variable.clone());
                    }
                }
            }
            if !names.is_empty() {
                out.insert(key, names);
            }
        }
        Grants(out)
    }
}
