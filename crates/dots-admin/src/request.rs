//! Request codes (accounts design §5): looking at one before acting on it. `roster
//! add-request` (in `roster.rs`) uses the same reading to add the sender to the member list.

use dots_core::accounts::{parse_request, AccessRequest};
use dots_core::vault::org::key_fingerprint;

use crate::args::Args;
use crate::{core_failed, failed, say, Io, Result};

/// Reads a request code, checking its signature; the error is `RequestError`'s own text.
pub(crate) fn read_request(code: &str) -> Result<AccessRequest> {
    parse_request(code).map_err(|e| failed(e.to_string()))
}

/// `request inspect <code>`: what the code says, once its signature has checked out.
pub(crate) fn request_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(crate::usage("'request' needs: inspect"));
    };
    match sub.as_str() {
        "inspect" => inspect_command(rest, io),
        other => Err(crate::usage(format!("unknown request command '{other}'"))),
    }
}

fn inspect_command(args: &[String], io: &mut Io<'_>) -> Result<()> {
    let args = Args::parse(args, &[], &[])?;
    let code = args.one("the request code")?;
    let request = read_request(code)?;
    say(io.out, &format!("org:         {}", request.org_id))?;
    say(io.out, &format!("name:        {}", request.name))?;
    say(io.out, &format!("email:       {}", request.email))?;
    say(
        io.out,
        &format!(
            "key:         {}",
            key_fingerprint(&request.key).map_err(core_failed)?
        ),
    )?;
    say(
        io.out,
        &format!(
            "invite id:   {}",
            request.invite_id.as_deref().unwrap_or("none")
        ),
    )?;
    if let Some(age) = &request.age_recipient {
        say(io.out, &format!("age recipient: {age}"))?;
    }
    say(io.out, &format!("created at:  {}", request.created_at))
}
