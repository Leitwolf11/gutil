mod jobs;
mod ls;

use std::env;
use std::ffi::OsStr;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

const HELP: &str = "Usage: gutil [OPTIONS] <COMMAND> [ARGUMENTS]\n\nCommands:\n  ls  List directory contents\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n";

fn main() -> ExitCode {
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();

    let result = run(
        env::args_os().skip(1),
        stdout.is_terminal(),
        &mut output,
        &mut errors,
    );

    match result.and_then(|status| output.flush().map(|()| status)) {
        Ok(status) => ExitCode::from(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(errors, "gutil: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    mut args: impl Iterator<Item = std::ffi::OsString>,
    terminal: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let mut job_count = jobs::default_count();
    let command = loop {
        let argument = args.next();
        let Some(option) = argument
            .as_ref()
            .and_then(|arg| jobs::option(arg.as_encoded_bytes(), &mut args))
        else {
            break argument;
        };
        match option {
            Ok(count) => job_count = count,
            Err(error) => return writeln!(errors, "gutil: {error}").map(|()| 2),
        }
    };
    match command.as_deref().and_then(OsStr::to_str) {
        Some("ls") => ls::run(args, job_count, terminal, output, errors),
        Some("-h" | "--help") => output.write_all(HELP.as_bytes()).map(|()| 0),
        Some("-V" | "--version") => {
            writeln!(output, "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        _ => writeln!(errors, "gutil: expected a command (see 'gutil --help')").map(|()| 2),
    }
}
