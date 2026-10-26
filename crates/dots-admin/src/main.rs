//! `dots-admin`: see the library for the commands.

use std::process::ExitCode;

use dots_admin::{console_password, run, Io};

fn main() -> ExitCode {
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect();
    let Ok(args) = args else {
        eprintln!("dots-admin: an argument is not valid text");
        return ExitCode::from(2);
    };
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    let mut password = console_password;
    let mut io = Io {
        out: &mut out,
        err: &mut err,
        password: &mut password,
    };
    match run(&args, &mut io) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dots-admin: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}
