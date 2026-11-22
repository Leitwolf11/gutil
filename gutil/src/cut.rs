use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, IsTerminal, Write};
use std::path::Path;

use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

mod options;
mod positions;
mod records;

use options::{Options, Request};
use records::Records;

const LABEL: &str = "gutil cut";
const BUFFER_SIZE: usize = 64 * 1024;
const HELP: &str = "Usage: gutil cut [OPTIONS] [FILE]...\n\nSelect parts of records. Use '-' for standard input.\n\n  -b, --bytes LIST           Select byte positions\n  -c, --characters LIST      Select character positions\n  -f, --fields LIST          Select fields separated by tabs or -d\n  -F LIST                    Select blank-separated fields with space output\n  -d, --delimiter CHAR       Use CHAR as the input field separator\n  -O, --output-delimiter STR Use STR between selected ranges or fields\n  -w, --whitespace-delimited[=trimmed]  Split on runs of horizontal blanks\n  -n, --no-partial           In byte mode, keep selected characters intact\n  -s, --only-delimited       Omit records without a field separator\n  -z, --zero-terminated      Use NUL as the record separator\n      --complement           Invert the selected positions\n  -j, --jobs, --threads N    Limit runtime workers\n      --help                 Print help\n      --version              Print version\n\nLIST accepts one-based N, N-, N-M, and -M ranges separated by commas or blanks.\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Request::parse(args, workers, crate::text::utf8_locale()) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Cut(options)) => options,
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
    let stdout = io::stdout();
    let capacity = if stdout.is_terminal() { 0 } else { BUFFER_SIZE };
    let mut output = BufWriter::with_capacity(capacity, stdout.lock());
    let mut errors = io::stderr().lock();
    let mut partial = Vec::new();
    let mut progress = ui.progress(LABEL, "bytes", true);
    let mut status = 0;
    for path in &options.paths {
        let mut records = Records::new(options);
        let mut copy = |input: &mut dyn BufRead| {
            copy_records(
                input,
                &mut records,
                &mut partial,
                &mut output,
                &mut progress,
                &mut errors,
            )
        };
        let failure = if path == Path::new("-") {
            copy(&mut io::stdin().lock())?
        } else {
            match File::open(path) {
                Ok(file) => copy(&mut BufReader::with_capacity(BUFFER_SIZE, file))?,
                Err(error) => Some(error),
            }
        };
        if let Some(error) = failure {
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

fn copy_records(
    input: &mut dyn BufRead,
    records: &mut Records<'_>,
    partial: &mut Vec<u8>,
    output: &mut impl Write,
    progress: &mut Option<ProgressReport>,
    errors: &mut impl Write,
) -> io::Result<Option<io::Error>> {
    partial.clear();
    let error = loop {
        let bytes = match input.fill_buf() {
            Ok([]) => break None,
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => break Some(error),
        };
        let length = bytes.len();
        let mut start = 0;
        for end in memchr::memchr_iter(records.terminator(), bytes) {
            let record = &bytes[start..end];
            if partial.is_empty() {
                records.record(record, true, output)?;
            } else {
                partial.extend_from_slice(record);
                records.record(partial, true, output)?;
                partial.clear();
            }
            start = end + 1;
        }
        if records.streams_bytes() {
            if start != length {
                records.record(&bytes[start..], false, output)?;
            }
        } else {
            partial.extend_from_slice(&bytes[start..]);
        }
        input.consume(length);
        if let Some(progress) = progress {
            progress.advance(length as u64, errors);
        }
    };
    if !partial.is_empty() {
        records.record(partial, false, output)?;
    }
    records.finish(output)?;
    Ok(error)
}
