mod cat;
mod jobs;
mod ls;
mod pwd;

use std::env;
use std::ffi::OsStr;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

const HELP: &str = "Usage: gutil [OPTIONS] <COMMAND> [ARGUMENTS]\n\nCommands:\n  cat  Concatenate files\n  ls   List directory contents\n  pwd  Print the working directory\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n";

fn main() -> ExitCode {
    // Discard argv[0] and preserve non-UTF-8 filesystem operands.
    let args = env::args_os().skip(1); // nosemgrep: args-os, rust.lang.security.args-os.args-os
    match run(args) {
        Ok(status) => ExitCode::from(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "gutil: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<u8> {
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
            Err(error) => return writeln!(io::stderr(), "gutil: {error}").map(|()| 2),
        }
    };
    let command = command.as_deref().and_then(OsStr::to_str);
    if command == Some("cat") {
        return match cat::run(args, job_count) {
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => {
                writeln!(io::stderr(), "gutil cat: {error}")?;
                Ok(1)
            }
            result => result,
        };
    }

    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();
    let result = match command {
        Some("ls") => ls::run(
            args,
            job_count,
            stdout.is_terminal(),
            &mut output,
            &mut errors,
        ),
        Some("pwd") => pwd::run(args, job_count, &mut output, &mut errors),
        Some("-h" | "--help") => output.write_all(HELP.as_bytes()).map(|()| 0),
        Some("-V" | "--version") => {
            writeln!(output, "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        _ => writeln!(errors, "gutil: expected a command (see 'gutil --help')").map(|()| 2),
    };
    match result.and_then(|status| output.flush().map(|()| status)) {
        Err(error) if command == Some("pwd") => {
            writeln!(errors, "gutil pwd: {error}")?;
            Ok(1)
        }
        result => result,
    }
}
