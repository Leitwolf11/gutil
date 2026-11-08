mod ls;

use std::env;
use std::ffi::OsStr;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

const HELP: &str = "Usage: gutil <COMMAND> [ARGUMENTS]\n\nCommands:\n  ls  List directory contents\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let command = args.next();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();

    let result = match command.as_deref().and_then(OsStr::to_str) {
        Some("ls") => ls::run(args, stdout.is_terminal(), &mut output, &mut errors),
        Some("-h" | "--help") => output.write_all(HELP.as_bytes()).map(|()| 0),
        Some("-V" | "--version") => {
            writeln!(output, "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        _ => writeln!(errors, "gutil: expected a command (see 'gutil --help')").map(|()| 2),
    };

    match result.and_then(|status| output.flush().map(|()| status)) {
        Ok(status) => ExitCode::from(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(errors, "gutil: {error}");
            ExitCode::FAILURE
        }
    }
}
