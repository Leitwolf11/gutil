use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::Path;

use crate::jobs;
use crate::presentation::Presentation;

mod options;
mod stops;
mod stream;

use options::{Options, Request};
use stream::Expander;

const LABEL: &str = "gutil expand";
const HELP: &str = "Usage: gutil expand [OPTIONS] [FILE]...\n\nReplace tabs with spaces. Use '-' for standard input.\n\n  -i, --initial         Convert tabs only before the first nonblank\n  -t, --tabs LIST       Set a repeating width or a list of tab stops\n  -j, --jobs, --threads N  Limit runtime workers\n      --help            Print help\n      --version         Print version\n\nA final /N repeats every N columns from column zero. A final +N\nrepeats every N columns from the final explicit stop.\n";

pub(crate) fn run(
    arguments: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Request::parse(arguments, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Expand(options)) => options,
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
    let mut expander = Expander::new(options);
    let mut input_buffer = vec![0; stream::BUFFER_SIZE + 3];
    let stdout = io::stdout();
    let capacity = if stdout.is_terminal() {
        0
    } else {
        stream::BUFFER_SIZE
    };
    let mut output = BufWriter::with_capacity(capacity, stdout.lock());
    let mut errors = io::stderr().lock();
    let mut progress = ui.progress(LABEL, "bytes", true);
    let mut status = 0;
    for path in &options.paths {
        let read_error = if path == Path::new("-") {
            expander.copy(
                &mut io::stdin().lock(),
                &mut output,
                &mut input_buffer,
                &mut progress,
                &mut errors,
            )?
        } else {
            match File::open(path) {
                Ok(mut file) => expander.copy(
                    &mut file,
                    &mut output,
                    &mut input_buffer,
                    &mut progress,
                    &mut errors,
                )?,
                Err(error) => Some(error),
            }
        };
        if let Some(error) = read_error {
            let _ = ui.error(&mut errors, LABEL, &format!("{}: {error}", path.display()));
            status = 1;
        }
    }
    output.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(status)
}
