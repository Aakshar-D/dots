//! What a child process starts with (env vault spec §8): not Dots' whole environment, only a
//! base it needs to run, and then what it is given. So a secret set for Dots never reaches the
//! `claude` CLI, an MCP server or the shell, and `printenv` shows nothing that was not granted.

use std::ffi::OsString;

use tokio::process::Command;

/// The variables every child keeps from Dots' environment, compared without regard to case.
pub const BASE_ENV: &[&str] = &[
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "PATH",
    "PATHEXT",
    "PSModulePath",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "CommonProgramFiles(x86)",
    "CommonProgramW6432",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "LANG",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
];

/// What the `claude` CLI also keeps: its own configuration and login, passed through unread
/// (the subscription rule).
pub const CLI_ENV: &[&str] = &["CLAUDE_CONFIG_DIR"];
/// Prefixes of the `claude` CLI's own variables.
pub const CLI_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_CODE_"];

/// Which kind of child the environment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
    /// The `claude` CLI: the base and its own variables.
    Cli,
    /// Anything else Dots starts for a run: an MCP server, the local engine's shell.
    Other,
}

fn kept(name: &str, child: Child) -> bool {
    let upper = name.to_ascii_uppercase();
    BASE_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
        || (child == Child::Cli
            && (CLI_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
                || CLI_PREFIXES.iter().any(|p| upper.starts_with(p))))
}

/// The variables of Dots' environment that a `child` keeps.
pub fn base_env(child: Child) -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(k, _)| k.to_str().is_some_and(|k| kept(k, child)))
        .collect()
}

/// Clears `cmd`'s environment down to the base of `child`. What the caller sets after this
/// (the granted values, Dots' own settings) is added on top.
pub fn start_clean(cmd: &mut Command, child: Child) {
    cmd.env_clear().envs(base_env(child));
}
