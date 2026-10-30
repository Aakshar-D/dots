//! `dots-admin`, the org admin's helper (env vault spec §12): the policy signing key, the policy
//! itself and the join codes people paste into Dots. It uses `dots-core`'s policy and join code
//! types and checks, so what it writes is what Dots reads.
//!
//! Every command is a function here, so tests call them without a terminal; `main` only wires
//! `run` to the console.

use std::io::Write;
use std::path::Path;

mod args;
pub mod join;
pub mod keys;
pub mod policy;
mod request;
pub mod roster;

/// The environment variable a password is read from before asking on the terminal.
pub const PASSWORD_VAR: &str = "DOTS_ADMIN_PASSWORD";

/// Why a command did not run or did not succeed, in plain words. A secret key or password is
/// never part of the text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdminError {
    /// The command line is wrong.
    #[error("{0}\nRun 'dots-admin help' to see the commands.")]
    Usage(String),
    /// The command ran and failed, or found the input invalid.
    #[error("{0}")]
    Failed(String),
}

impl AdminError {
    /// The process exit code: 2 for a wrong command line, 1 for a failure.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, AdminError>;

/// What a password is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// The password that will protect a new key: asked twice on the terminal.
    NewKey,
    /// The password of an existing key, to sign with it.
    Unlock,
}

/// A source of passwords: `console_password` in the binary, a closure in tests.
pub type PasswordSource<'a> = &'a mut dyn FnMut(Ask) -> Result<String>;

/// Where a command writes and how it gets a password.
pub struct Io<'a> {
    /// The command's result: a policy, a signed text, a join code, a summary.
    pub out: &'a mut dyn Write,
    /// Notes and warnings that should not end up in a redirected result.
    pub err: &'a mut dyn Write,
    pub password: PasswordSource<'a>,
}

const USAGE: &str = "\
dots-admin: the org admin's key, policy and join code helper for Dots.

Commands:
  dots-admin keygen --out <dir> [--no-password]
      Makes the policy signing key pair, <dir>/policy.key and <dir>/policy.pub.
  dots-admin policy new --org-id <id> [--roster [--admin <dots-id> ...]] [--out <file>]
      Writes a policy with every field at its default. With --roster it is a v2 policy that
      requires a signed member list; each --admin (the owner's Dots ID, say) may sign it.
  dots-admin policy check <policy.json>
      Checks a policy as Dots does.
  dots-admin policy sign <policy.json> --key <policy.key> [--out <file>]
      Sets issued_at to now and writes the signed text, to store as DOTS_POLICY.
  dots-admin policy verify <signed> --pub <policy.pub> [--org-id <id>]
      Shows what a Dots client would accept.
  dots-admin policy admins add|remove <policy.json> <dots-id>
      Edits the policy's admins, the keys that may sign the member list. Sign it afterwards.
  dots-admin roster new --org-id <id> --owner-name <name> --owner-email <email>
                        --owner-key <dots-id> [--out <roster.json>]
      Starts a member list with its owner and the default workspace.
  dots-admin roster add <roster.json> --name <name> --email <email> --key <dots-id>
                        --role <owner|admin|member|viewer> [--workspace <id>=<role> ...]
      Adds a member; an email already listed gets the key as another PC.
  dots-admin roster set-role <roster.json> <email> <role>
  dots-admin roster remove <roster.json> <email> [--key <dots-id>]
      Changes a member's role, or removes them (or just one of their keys).
  dots-admin roster set-workspaces <roster.json> <email> <id>=<role> ...
      Sets the member's role in each workspace they are in (admin, member or viewer); the
      default workspace must be one.
  dots-admin roster workspace add <roster.json> <name> [--allow <pattern> ...] [--deny <pattern> ...]
  dots-admin roster workspace rename <roster.json> <id> <name>
  dots-admin roster workspace remove <roster.json> <id>
  dots-admin roster workspace use <roster.json> <id> [--allow <pattern> ...] [--deny <pattern> ...]
      Adds, renames or removes a workspace (never default), or sets the entry names its dots
      may use; 'use' with no patterns clears them. These edit the file only; sign it afterwards.
  dots-admin roster add-request <roster.json> <request code> --role <owner|admin|member|viewer>
      Adds the sender of a request code (see 'request inspect'); an email already listed gets
      the key as another PC. Sign the list afterwards.
  dots-admin roster check <roster.json> --org-id <id>
      Checks a member list as Dots does.
  dots-admin roster sign <roster.json> --key <policy.key> [--out <file>]
      Raises seq by one, sets issued_at to now and writes the signed text, to store as
      DOTS_ROSTER.
  dots-admin roster verify <signed> --pub <policy.pub> [--admin <dots-id> ...] --org-id <id>
      Shows what a Dots client would accept.
  dots-admin request inspect <request code>
      Shows who a request code is from, after checking its signature.
  dots-admin join-code --org-id <id> --name <name> --source <openbao|infisical|sops>
                       [source options] --pub <policy.pub> [--file <out.dotsorg>]
      Prints a join code, or writes it to a .dotsorg file.
      openbao:   --address <url> --path <path> [--mount <kv mount>]
                 [--auth <token|userpass|ldap>] [--auth-mount <mount>] [--namespace <ns>]
      infisical: --site-url <url> --project-id <id> --environment <slug>
                 [--secret-path </path>]
      sops:      --location <full path or https:// url>

A key's password is read from DOTS_ADMIN_PASSWORD when it is set, else from the terminal.
";

/// Runs one command line, without the program name.
pub fn run(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let Some((command, rest)) = args.split_first() else {
        return Err(usage("no command given"));
    };
    match command.as_str() {
        "help" | "--help" | "-h" => say(io.out, USAGE.trim_end()),
        "keygen" => keys::keygen_command(rest, io),
        "policy" => {
            let Some((sub, rest)) = rest.split_first() else {
                return Err(usage(
                    "'policy' needs one of: new, check, sign, verify, admins",
                ));
            };
            match sub.as_str() {
                "new" => policy::new_command(rest, io),
                "check" => policy::check_command(rest, io),
                "sign" => policy::sign_command(rest, io),
                "verify" => policy::verify_command(rest, io),
                "admins" => policy::admins_command(rest, io),
                other => Err(usage(format!("unknown policy command '{other}'"))),
            }
        }
        "request" => request::request_command(rest, io),
        "roster" => roster::roster_command(rest, io),
        "join-code" => join::join_code_command(rest, io),
        other => Err(usage(format!("unknown command '{other}'"))),
    }
}

/// The binary's password source: `DOTS_ADMIN_PASSWORD` when it is set, else the terminal
/// without echo. A new key's password is asked twice.
pub fn console_password(ask: Ask) -> Result<String> {
    if let Some(value) = std::env::var_os(PASSWORD_VAR) {
        return value
            .into_string()
            .map_err(|_| failed(format!("{PASSWORD_VAR} is not valid text")));
    }
    let read = |prompt: &str| {
        rpassword::prompt_password(prompt).map_err(|e| {
            failed(format!(
                "could not read a password from the terminal ({e}); set {PASSWORD_VAR} instead"
            ))
        })
    };
    match ask {
        Ask::Unlock => read("Password for the policy key: "),
        Ask::NewKey => {
            let first = read("Password for the new policy key: ")?;
            let again = read("The same password again: ")?;
            if first != again {
                return Err(failed("the two passwords differ; nothing was written"));
            }
            Ok(first)
        }
    }
}

pub(crate) fn usage(message: impl Into<String>) -> AdminError {
    AdminError::Usage(message.into())
}

pub(crate) fn failed(message: impl Into<String>) -> AdminError {
    AdminError::Failed(message.into())
}

/// A `dots-core` error as plain words: its own message without the "invalid input: " prefix.
pub(crate) fn core_failed(error: dots_core::Error) -> AdminError {
    match error {
        dots_core::Error::Invalid(message) => failed(message),
        other => failed(other.to_string()),
    }
}

/// Writes `text` and a line end.
pub(crate) fn say(to: &mut dyn Write, text: &str) -> Result<()> {
    writeln!(to, "{text}").map_err(|e| failed(format!("could not write the output: {e}")))
}

pub(crate) fn read_file(path: &str, what: &str) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| failed(format!("could not read {what} {path}: {e}")))
}

/// Writes a result file, replacing one that is there.
pub(crate) fn write_file(path: &str, text: &str) -> Result<()> {
    std::fs::write(Path::new(path), text)
        .map_err(|e| failed(format!("could not write {path}: {e}")))
}
