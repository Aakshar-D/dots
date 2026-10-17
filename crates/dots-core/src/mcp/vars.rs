//! `${VAR}` in an MCP server's config (MCP/skills spec section 2): the value of `VAR` in Dots'
//! own environment, read when the server starts. A variable that is not set stops the server
//! from starting.

use std::collections::{BTreeMap, BTreeSet};

use crate::library::McpServerConfig;
use crate::{Error, Result};

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(message.into()))
}

enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

fn is_var_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `s` cut into text and `${VAR}` references. A `${` that is not closed, or whose name is not a
/// variable name, is text.
fn pieces(s: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) if is_var_name(&after[..end]) => {
                out.push(Piece::Text(&rest[..start]));
                out.push(Piece::Var(&after[..end]));
                rest = &after[end + 1..];
            }
            _ => {
                out.push(Piece::Text(&rest[..start + 2]));
                rest = after;
            }
        }
    }
    out.push(Piece::Text(rest));
    out
}

fn expand(s: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::new();
    for piece in pieces(s) {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(name) => match env(name) {
                Some(value) => out.push_str(&value),
                None => return invalid(format!("{name} is not set")),
            },
        }
    }
    Ok(out)
}

impl McpServerConfig {
    /// The strings that may hold `${VAR}`, in the order they are expanded.
    fn expandable(&self) -> Vec<&String> {
        match self {
            Self::Stdio { args, cwd, env, .. } => {
                env.values().chain(args.iter()).chain(cwd.iter()).collect()
            }
            Self::Http { url, headers } => std::iter::once(url).chain(headers.values()).collect(),
        }
    }

    /// The variables the config reads, sorted, each once.
    pub fn variables(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for s in self.expandable() {
            for piece in pieces(s) {
                if let Piece::Var(name) = piece {
                    names.insert(name.to_string());
                }
            }
        }
        names.into_iter().collect()
    }

    /// The config with each `${VAR}` replaced by `env(VAR)`. A variable that `env` does not
    /// have is an error naming it: "SF_TOKEN is not set".
    pub fn expanded(&self, env: &dyn Fn(&str) -> Option<String>) -> Result<McpServerConfig> {
        let map = |m: &BTreeMap<String, String>| -> Result<BTreeMap<String, String>> {
            m.iter()
                .map(|(k, v)| Ok((k.clone(), expand(v, env)?)))
                .collect()
        };
        Ok(match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env: vars,
            } => {
                let vars = map(vars)?;
                Self::Stdio {
                    command: command.clone(),
                    args: args.iter().map(|a| expand(a, env)).collect::<Result<_>>()?,
                    cwd: cwd.as_deref().map(|c| expand(c, env)).transpose()?,
                    env: vars,
                }
            }
            Self::Http { url, headers } => {
                let url = expand(url, env)?;
                Self::Http {
                    url,
                    headers: map(headers)?,
                }
            }
        })
    }
}
