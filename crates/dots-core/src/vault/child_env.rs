//! What a child process starts with (env vault spec §8): not Dots' whole environment, only a
//! base it needs to run, and then what it is given. So a secret set for Dots never reaches an
//! MCP server or the shell, and `printenv` there shows nothing that was not granted. The `claude`
//! CLI also keeps its own pass-through variables (`ANTHROPIC_*`, `CLAUDE_CODE_*` and the `CLI_ENV`
//! names), which its tools can see.

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
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "ALL_PROXY",
    "ALLUSERSPROFILE",
    "PUBLIC",
    // Where toolchains are: an agent's build finds its JDK, Go, Rust, .NET, Node or Python.
    "JAVA_HOME",
    "GOROOT",
    "GOPATH",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "DOTNET_ROOT",
    "PNPM_HOME",
    "NVM_HOME",
    "NVM_SYMLINK",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "ANDROID_HOME",
];

/// What the `claude` CLI also keeps: its own configuration and login, passed through unread
/// (the subscription rule).
pub const CLI_ENV: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "DISABLE_TELEMETRY",
    "DISABLE_AUTOUPDATER",
    "DISABLE_ERROR_REPORTING",
    "MCP_TIMEOUT",
    "BASH_DEFAULT_TIMEOUT_MS",
    "BASH_MAX_TIMEOUT_MS",
    "MAX_MCP_OUTPUT_TOKENS",
];
/// Prefixes of the `claude` CLI's own variables.
pub const CLI_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_CODE_"];

/// What a shell grant never sets for the `claude` CLI, besides its own variables: names that
/// would point it at another endpoint or load code into it. The CLI keeps what Dots'
/// environment passed through (`BASE_ENV`); other children may be granted them.
pub const CLI_GRANT_REFUSED: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
    "NODE_OPTIONS",
];

/// Which kind of child the environment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
    /// The `claude` CLI: the base and its own variables.
    Cli,
    /// Anything else Dots starts for a run: an MCP server, the local engine's shell.
    Other,
}

/// Whether `name` is one of the `claude` CLI's own variables (its login, its endpoint, its
/// settings). Dots passes these through from its environment and never sets them from Personal.
pub fn is_cli_own(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    CLI_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
        || CLI_PREFIXES.iter().any(|p| upper.starts_with(p))
}

/// Whether a shell grant of `name` is left out of the `claude` CLI's environment: one of its
/// own variables or of `CLI_GRANT_REFUSED`, in any case.
pub fn cli_refuses_grant(name: &str) -> bool {
    is_cli_own(name)
        || CLI_GRANT_REFUSED
            .iter()
            .any(|k| k.eq_ignore_ascii_case(name))
}

fn kept(name: &str, child: Child) -> bool {
    BASE_ENV.iter().any(|k| k.eq_ignore_ascii_case(name))
        || (child == Child::Cli && is_cli_own(name))
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
