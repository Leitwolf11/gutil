use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use crate::jobs;
use crate::presentation::Presentation;

mod options;
mod wrap;

use options::{Options, Request};
use wrap::Folder;

const LABEL: &str = "gutil fold";
const HELP: &str = "Usage: gutil fold [OPTIONS] [FILE]...\n\nWrap input lines at a maximum width. Use '-' for standard input.\n\n  -b, --bytes          Count bytes\n  -c, --characters     Count characters, accounting for cursor controls\n  -s, --spaces         Prefer to wrap after blank characters\n  -w, --width WIDTH    Set the maximum width (default 80)\n  -j, --jobs, --threads N  Limit runtime workers\n      --help           Print help\n      --version        Print version\n";

pub(crate) fn run(
    arguments: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(arguments, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Fold(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    match runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(&options, ui))
            .await
            .map_err(io::Error::other)?
    }) {
        Ok(status) => Ok(status),
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            Ok(1)
        }
    }
}

fn execute(options: &Options, ui: Presentation) -> io::Result<u8> {
    let mut folder = Folder::new(options);
    let mut progress = ui.progress(LABEL, "bytes", true);
    let mut output = BufWriter::with_capacity(64 * 1024, io::stdout().lock());
    let mut errors = io::stderr().lock();
    let mut status = 0;
    for path in &options.paths {
        let read_error = if path == Path::new("-") {
            folder.copy(
                &mut io::stdin().lock(),
                &mut output,
                &mut progress,
                &mut errors,
            )?
        } else {
            match File::open(path) {
                Ok(mut file) => folder.copy(&mut file, &mut output, &mut progress, &mut errors)?,
                Err(error) => Some(error),
            }
        };
        if let Some(error) = read_error {
            status = 1;
            let _ = ui.error(&mut errors, LABEL, &format!("{}: {error}", path.display()));
        }
    }
    output.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(status)
}
