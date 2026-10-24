//! Join codes (env vault spec §6.1): what the admin hands each person. A code names the org, its
//! source and the policy key, and holds no secrets.

use dots_core::vault::org::join::JOIN_CODE_VERSION;
use dots_core::vault::org::{key_fingerprint, parse_join_code, BaoAuth, JoinCode, SourceSettings};
use serde_json::{Map, Value};

use crate::args::Args;
use crate::keys::read_public_key;
use crate::{core_failed, failed, read_file, say, usage, write_file, Io, Result};

/// A source kind's options, named after its `SourceSettings` fields with `-` for `_`.
struct Kind {
    name: &'static str,
    required: &'static [&'static str],
    optional: &'static [&'static str],
}

const KINDS: &[Kind] = &[
    Kind {
        name: "openbao",
        required: &["address", "path"],
        optional: &["mount", "auth", "auth-mount", "namespace"],
    },
    Kind {
        name: "infisical",
        required: &["site-url", "project-id", "environment"],
        optional: &["secret-path"],
    },
    Kind {
        name: "sops",
        required: &["location"],
        optional: &[],
    },
];

/// The options `join-code` takes besides the source's.
const OPTIONS: &[&str] = &["org-id", "name", "source", "pub", "file"];

/// Builds the join code and reads it back with `parse_join_code`, as Dots will, so a code Dots
/// would refuse is never handed out. `public_key` is the bare `RW…` key.
pub fn join_code(
    org_id: &str,
    name: &str,
    source: SourceSettings,
    public_key: &str,
) -> Result<JoinCode> {
    let code = JoinCode {
        v: JOIN_CODE_VERSION,
        org_id: org_id.to_string(),
        name: name.to_string(),
        source,
        policy_key: public_key.to_string(),
    };
    let read_back = parse_join_code(&code.encode()).map_err(core_failed)?;
    if read_back != code {
        return Err(failed("the join code does not read back as written"));
    }
    Ok(code)
}

/// `join-code --org-id <id> --name <name> --source <kind> [source options] --pub <policy.pub>
/// [--file <out.dotsorg>]`. Without `--file` the code alone goes to stdout and the notes to
/// stderr, so the code can be redirected.
pub(crate) fn join_code_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let mut options = OPTIONS.to_vec();
    for kind in KINDS {
        options.extend(kind.required.iter().chain(kind.optional));
    }
    let args = Args::parse(args, &options, &[])?;
    args.none()?;
    let source = source_settings(&args)?;
    let public_key = read_public_key(&read_file(args.required("pub")?, "the public key")?)?;
    let code = join_code(
        args.required("org-id")?,
        args.required("name")?,
        source,
        &public_key,
    )?;
    let notes = [
        format!("org: {} ({})", code.name, code.org_id),
        format!(
            "source: {} at {}",
            code.source.kind(),
            code.source.address()
        ),
        format!(
            "policy key fingerprint: {}; Dots shows it on joining, so tell people to expect it",
            key_fingerprint(&code.policy_key).map_err(core_failed)?
        ),
    ];
    match args.value("file") {
        Some(path) => {
            write_file(path, &format!("{}\n", code.encode()))?;
            say(io.out, &format!("wrote {path}"))?;
            notes.iter().try_for_each(|note| say(io.out, note))
        }
        None => {
            say(io.out, &code.encode())?;
            notes.iter().try_for_each(|note| say(io.err, note))
        }
    }
}

/// The `SourceSettings` the options describe. Fields left out take `dots-core`'s defaults, as
/// they do in a join code: the settings are read with its own serde rules.
fn source_settings(args: &Args) -> Result<SourceSettings> {
    let kind_name = args.required("source")?;
    let kind = KINDS.iter().find(|k| k.name == kind_name).ok_or_else(|| {
        usage(format!(
            "--source must be openbao, infisical or sops, not '{kind_name}'"
        ))
    })?;
    for option in args.given() {
        let belongs = OPTIONS.contains(&option)
            || kind.required.contains(&option)
            || kind.optional.contains(&option);
        if !belongs {
            let owner = KINDS
                .iter()
                .find(|k| k.required.contains(&option) || k.optional.contains(&option))
                .map_or("another source", |k| k.name);
            return Err(usage(format!("--{option} is for --source {owner}")));
        }
    }
    let mut fields = Map::new();
    fields.insert("kind".into(), Value::from(kind.name));
    for &option in kind.required {
        let value = args
            .value(option)
            .ok_or_else(|| usage(format!("--source {} needs --{option}", kind.name)))?;
        fields.insert(option.replace('-', "_"), Value::from(value));
    }
    for &option in kind.optional {
        if let Some(value) = args.value(option) {
            fields.insert(option.replace('-', "_"), Value::from(value));
        }
    }
    if let Some(auth) = args.value("auth") {
        if !BaoAuth::ALL.iter().any(|a| a.as_str() == auth) {
            return Err(usage(format!(
                "--auth must be token, userpass or ldap, not '{auth}'"
            )));
        }
    }
    if args.value("auth-mount").is_some() && matches!(args.value("auth"), None | Some("token")) {
        return Err(usage("--auth-mount is for --auth userpass or ldap"));
    }
    serde_json::from_value(Value::Object(fields)).map_err(|e| {
        let why = e.to_string().replace('`', "'");
        failed(format!("the source options do not make a source: {why}"))
    })
}
