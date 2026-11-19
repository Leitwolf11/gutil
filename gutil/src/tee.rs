use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};

use crate::jobs;
use crate::presentation::Presentation;

mod options;

use options::{ErrorMode, Options, Request};

const HELP: &str = "Usage: gutil tee [OPTIONS] [FILE]...\n\nCopy standard input to standard output and each FILE. '-' is a file name.\n\n  -a, --append             Append instead of overwriting files\n  -i, --ignore-interrupts  Ignore Ctrl-C interrupt signals\n  -p                      Continue writing after a pipe closes\n      --output-error[=MODE]  warn, warn-nopipe, exit, or exit-nopipe\n                            Without MODE, use warn-nopipe\n  -j, --jobs, --threads N  Limit runtime workers (the stream stays ordered)\n      --help              Print help\n      --version           Print version\n\nBy default, exit on a broken pipe and diagnose other output errors.\nThe warn modes continue writing to remaining outputs. The exit modes stop\non the first error. The nopipe modes ignore broken pipes and stop when\nno outputs remain.\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "gutil tee {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Copy(options)) => options,
        Err(error) => {
            ui.error(&mut io::stderr(), "gutil tee", &error.to_string())?;
            return Ok(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .max_blocking_threads(options.workers.get())
        .build()?;
    let result = runtime.block_on(async move {
        // Keep the receiver alive: Windows restores default handling when the
        // final listener is dropped, even though Unix handlers persist.
        let interrupts = options
            .ignore_interrupts
            .then(ignore_interrupts)
            .transpose()?;
        let result = tokio::task::spawn_blocking(move || copy(&options, ui))
            .await
            .map_err(io::Error::other)?;
        drop(interrupts);
        result
    });
    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            ui.error(&mut io::stderr(), "gutil tee", &error.to_string())?;
            Ok(1)
        }
    }
}

#[cfg(unix)]
fn ignore_interrupts() -> io::Result<tokio::signal::unix::Signal> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
}

#[cfg(windows)]
fn ignore_interrupts() -> io::Result<tokio::signal::windows::CtrlC> {
    tokio::signal::windows::ctrl_c()
}

struct Output<'a> {
    name: String,
    writer: Destination<'a>,
}

enum Destination<'a> {
    Stdout(io::StdoutLock<'a>),
    File(File),
}

impl Destination<'_> {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Stdout(writer) => {
                writer.write_all(bytes)?;
                // Forward partial lines immediately, including interactive input.
                writer.flush()
            }
            Self::File(writer) => writer.write_all(bytes),
        }
    }
}

fn copy(options: &Options, ui: Presentation) -> io::Result<u8> {
    let stdout = io::stdout();
    let mut outputs = vec![Output {
        name: "standard output".to_owned(),
        writer: Destination::Stdout(stdout.lock()),
    }];
    let mut errors = io::stderr().lock();
    let mut failed = false;
    for path in &options.paths {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(options.append)
            .truncate(!options.append)
            .open(path);
        match file {
            Ok(file) => outputs.push(Output {
                name: path.display().to_string(),
                writer: Destination::File(file),
            }),
            Err(error) => {
                ui.error(
                    &mut errors,
                    "gutil tee",
                    &format!("{}: {error}", path.display()),
                )?;
                failed = true;
                if options.mode.exits() {
                    return Ok(1);
                }
            }
        }
    }
    let stdin = io::stdin();
    let mut progress = ui.progress("gutil tee", "bytes", !stdin.is_terminal());
    let mut input = stdin.lock();
    let mut buffer = vec![0; 128 * 1024];
    while !outputs.is_empty() {
        let size = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                ui.error(
                    &mut errors,
                    "gutil tee",
                    &format!("standard input: {error}"),
                )?;
                failed = true;
                break;
            }
        };
        if write_outputs(
            &mut outputs,
            &buffer[..size],
            options.mode,
            ui,
            &mut errors,
            &mut failed,
        )? {
            return Ok(1);
        }
        if let Some(progress) = &mut progress {
            progress.advance(size as u64, &mut errors);
        }
    }
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(u8::from(failed))
}

fn write_outputs(
    outputs: &mut Vec<Output<'_>>,
    bytes: &[u8],
    mode: ErrorMode,
    ui: Presentation,
    errors: &mut impl Write,
    failed: &mut bool,
) -> io::Result<bool> {
    let mut index = 0;
    while index < outputs.len() {
        if let Err(error) = outputs[index].writer.write_chunk(bytes) {
            let pipe = error.kind() == io::ErrorKind::BrokenPipe;
            if pipe && mode == ErrorMode::Default {
                #[cfg(unix)]
                signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGPIPE)?;
                return Ok(true);
            }
            if !(pipe && mode.ignores_pipe()) {
                ui.error(
                    errors,
                    "gutil tee",
                    &format!("{}: {error}", outputs[index].name),
                )?;
                *failed = true;
                if mode.exits() {
                    return Ok(true);
                }
            }
            outputs.remove(index);
        } else {
            index += 1;
        }
    }
    Ok(false)
}
