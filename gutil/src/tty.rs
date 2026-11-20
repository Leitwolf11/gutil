use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};

use crate::presentation::Presentation;

const HELP: &str = "Usage: gutil tty [OPTION]...\n\nPrint the terminal connected to standard input.\n\n  -s, --silent, --quiet  Report only the exit status\n      --help            Print help\n      --version         Print version\n";

#[derive(Clone, Copy)]
enum Request {
    Print,
    Quiet,
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
enum Invalid {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("extra operand {0:?}")]
    Operand(OsString),
}

impl Request {
    fn parse(args: impl Iterator<Item = OsString>) -> Result<Self, Invalid> {
        let mut request = Self::Print;
        let mut options = true;
        let mut operand = None;

        for argument in args {
            let bytes = argument.as_encoded_bytes();
            if options && bytes == b"--" {
                options = false;
            } else if options && bytes.len() > 1 && bytes.starts_with(b"-") {
                match bytes {
                    [b'-', b'-', prefix @ ..] if b"help".starts_with(prefix) => {
                        return Ok(Self::Help);
                    }
                    [b'-', b'-', prefix @ ..] if b"version".starts_with(prefix) => {
                        return Ok(Self::Version);
                    }
                    [b'-', b'-', prefix @ ..]
                        if b"silent".starts_with(prefix) || b"quiet".starts_with(prefix) =>
                    {
                        request = Self::Quiet;
                    }
                    [b'-', flags @ ..] if flags.iter().all(|&flag| flag == b's') => {
                        request = Self::Quiet;
                    }
                    _ => return Err(Invalid::Option(argument)),
                }
            } else {
                if std::env::var_os("POSIXLY_CORRECT").is_some() {
                    return Err(Invalid::Operand(argument));
                }
                if operand.is_none() {
                    operand = Some(argument);
                }
            }
        }

        operand.map_or(Ok(request), |argument| Err(Invalid::Operand(argument)))
    }
}

pub(crate) fn run(args: impl Iterator<Item = OsString>, ui: Presentation) -> u8 {
    let request = match Request::parse(args) {
        Ok(request) => request,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), "gutil tty", &error.to_string());
            return 2;
        }
    };
    let input = io::stdin();
    if matches!(request, Request::Quiet) {
        return u8::from(!input.is_terminal());
    }

    let mut output = io::stdout().lock();
    match write_response(request, &input, &mut output).and_then(|status| {
        output.flush()?;
        Ok(status)
    }) {
        Ok(status) => status,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), "gutil tty", &error.to_string());
            3
        }
    }
}

fn write_response(request: Request, input: &io::Stdin, output: &mut impl Write) -> io::Result<u8> {
    match request {
        Request::Help => output.write_all(HELP.as_bytes())?,
        Request::Version => writeln!(output, "gutil tty {}", env!("CARGO_PKG_VERSION"))?,
        Request::Print | Request::Quiet => {
            if let Some(name) = terminal_name(input) {
                output.write_all(name.as_encoded_bytes())?;
                output.write_all(b"\n")?;
            } else {
                output.write_all(b"not a tty\n")?;
                return Ok(1);
            }
        }
    }
    Ok(0)
}

#[cfg(all(unix, not(target_os = "fuchsia")))]
fn terminal_name(input: &io::Stdin) -> Option<OsString> {
    nix::unistd::ttyname(input)
        .ok()
        .map(std::path::PathBuf::into_os_string)
}

#[cfg(not(all(unix, not(target_os = "fuchsia"))))]
fn terminal_name(input: &io::Stdin) -> Option<OsString> {
    let label = if cfg!(windows) { "CONIN$" } else { "stdin" };
    input.is_terminal().then(|| label.into())
}
