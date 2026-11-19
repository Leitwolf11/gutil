use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};

use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

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
            let _ = ui.error(&mut io::stderr(), "gutil tee", &error.to_string());
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
            let _ = ui.error(&mut io::stderr(), "gutil tee", &error.to_string());
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

struct Output<W> {
    name: String,
    writer: W,
}

trait Sink {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn finish(self) -> io::Result<()>;
}

enum Destination {
    #[cfg(windows)]
    Console(io::Stdout),
    File(File),
}

impl Sink for Destination {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            #[cfg(windows)]
            Self::Console(stdout) => {
                let mut writer = stdout.lock();
                writer.write_all(bytes)?;
                writer.flush()
            }
            Self::File(writer) => writer.write_all(bytes),
        }
    }

    fn finish(self) -> io::Result<()> {
        match self {
            #[cfg(windows)]
            Self::Console(mut stdout) => stdout.flush(),
            Self::File(file) => close_file(file),
        }
    }
}

#[cfg(unix)]
fn close_file(file: File) -> io::Result<()> {
    // Consume ownership exactly once. In particular, never retry close after
    // EINTR: the descriptor may already have been reused by another thread.
    nix::unistd::close(file).map_err(io::Error::from)
}

#[cfg(windows)]
fn close_file(file: File) -> io::Result<()> {
    use io_close::Close;
    file.close()
}

fn standard_output() -> io::Result<Destination> {
    let stdout = io::stdout();
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        stdout
            .as_fd()
            .try_clone_to_owned()
            .map(File::from)
            .map(Destination::File)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        if stdout.is_terminal() {
            Ok(Destination::Console(stdout))
        } else {
            stdout
                .as_handle()
                .try_clone_to_owned()
                .map(File::from)
                .map(Destination::File)
        }
    }
}

fn copy(options: &Options, ui: Presentation) -> io::Result<u8> {
    copy_with_output(options, ui, standard_output())
}

fn copy_with_output(
    options: &Options,
    ui: Presentation,
    standard_output: io::Result<Destination>,
) -> io::Result<u8> {
    let mut outputs = Vec::with_capacity(options.paths.len().saturating_add(1));
    let mut errors = io::stderr().lock();
    let mut failed = false;
    match standard_output {
        Ok(writer) => outputs.push(Output {
            name: "standard output".to_owned(),
            writer,
        }),
        Err(error) => {
            if output_error(
                "standard output",
                &error,
                options.mode,
                ui,
                &mut errors,
                &mut failed,
            )? {
                return Ok(1);
            }
        }
    }
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
                let _ = ui.error(
                    &mut errors,
                    "gutil tee",
                    &format!("{}: {error}", path.display()),
                );
                failed = true;
                if options.mode.exits() {
                    return Ok(1);
                }
            }
        }
    }
    let stdin = io::stdin();
    let progress = ui.progress("gutil tee", "bytes", !stdin.is_terminal());
    copy_stream(
        &mut stdin.lock(),
        outputs,
        options.mode,
        ui,
        &mut errors,
        failed,
        progress,
    )
}

fn copy_stream<W: Sink>(
    input: &mut impl Read,
    mut outputs: Vec<Output<W>>,
    mode: ErrorMode,
    ui: Presentation,
    errors: &mut impl Write,
    mut failed: bool,
    mut progress: Option<ProgressReport>,
) -> io::Result<u8> {
    let mut buffer = vec![0; 128 * 1024];
    while !outputs.is_empty() {
        let size = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => size,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = ui.error(errors, "gutil tee", &format!("standard input: {error}"));
                failed = true;
                break;
            }
        };
        if write_outputs(&mut outputs, &buffer[..size], mode, ui, errors, &mut failed)? {
            return Ok(1);
        }
        if let Some(progress) = &mut progress {
            progress.advance(size as u64, errors);
        }
    }
    for output in outputs {
        if let Err(error) = output.writer.finish() {
            if output_error(&output.name, &error, mode, ui, errors, &mut failed)? {
                return Ok(1);
            }
        }
    }
    if let Some(progress) = &mut progress {
        progress.finish(errors);
    }
    Ok(u8::from(failed))
}

fn write_outputs<W: Sink>(
    outputs: &mut Vec<Output<W>>,
    bytes: &[u8],
    mode: ErrorMode,
    ui: Presentation,
    errors: &mut impl Write,
    failed: &mut bool,
) -> io::Result<bool> {
    let mut index = 0;
    while index < outputs.len() {
        if let Err(error) = outputs[index].writer.write_chunk(bytes) {
            let output = outputs.remove(index);
            if output_error(&output.name, &error, mode, ui, errors, failed)? {
                return Ok(true);
            }
            if let Err(error) = output.writer.finish() {
                if output_error(&output.name, &error, mode, ui, errors, failed)? {
                    return Ok(true);
                }
            }
        } else {
            index += 1;
        }
    }
    Ok(false)
}

fn output_error(
    name: &str,
    error: &io::Error,
    mode: ErrorMode,
    ui: Presentation,
    errors: &mut impl Write,
    failed: &mut bool,
) -> io::Result<bool> {
    let pipe = error.kind() == io::ErrorKind::BrokenPipe;
    if pipe && mode == ErrorMode::Default {
        #[cfg(unix)]
        {
            signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGPIPE)?;
            return Ok(true);
        }
        #[cfg(windows)]
        return Err(io::Error::new(error.kind(), format!("{name}: {error}")));
    }
    if pipe && mode.ignores_pipe() {
        return Ok(false);
    }
    // Reporting must not turn an output failure into success when stderr is
    // closed, or prevent warn modes from forwarding to surviving outputs.
    let _ = ui.error(errors, "gutil tee", &format!("{name}: {error}"));
    *failed = true;
    Ok(mode.exits())
}

#[cfg(test)]
mod tests;
