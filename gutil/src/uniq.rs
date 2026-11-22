use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::ops::Range;
use std::path::Path;

use crate::jobs;
use crate::presentation::Presentation;

mod options;

use options::{Case, Options, Request, Separator};

const LABEL: &str = "gutil uniq";
const DASH: &str = "-";
const HELP: &str = "Usage: gutil uniq [OPTIONS] [INPUT [OUTPUT]]\n\nFilter adjacent matching records. Use '-' for standard input or output.\n\n  -c, --count              Prefix the occurrence count\n  -d, --repeated           Show one record per repeated group\n  -D, --all-repeated[=METHOD]  Show every repeated record\n      --group[=METHOD]     Show all records with group separators\n  -f, --skip-fields N      Skip N initial fields\n  -i, --ignore-case        Ignore ASCII case\n  -s, --skip-chars N       Skip N characters after fields\n  -u, --unique             Show only groups of one record\n  -w, --check-chars N      Compare at most N characters\n  -z, --zero-terminated    Use NUL-delimited input and output\n  -j, --jobs, --threads N  Limit runtime workers\n      --help               Print help\n      --version            Print version\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Run(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    let result = runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(&options, ui))
            .await
            .map_err(io::Error::other)?
    });
    match result {
        Ok(()) => Ok(0),
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            Ok(1)
        }
    }
}

fn execute(options: &Options, ui: Presentation) -> io::Result<()> {
    let input = if options.input == Path::new(DASH) {
        None
    } else {
        Some(File::open(&options.input).map_err(|error| path_error(&options.input, &error))?)
    };
    let file = if options.output == Path::new(DASH) {
        if let Some(input) = &input {
            if input.metadata()?.is_file() {
                let input_id = same_file::Handle::from_file(input.try_clone()?)?;
                reject_same(&input_id, &same_file::Handle::stdout()?)?;
            }
        } else if let (Ok(input), Ok(output)) =
            (same_file::Handle::stdin(), same_file::Handle::stdout())
        {
            if input == output && input.as_file().metadata()?.is_file() {
                reject_same(&input, &output)?;
            }
        }
        None
    } else {
        Some(
            open_output(&options.output, input.as_ref())
                .map_err(|error| path_error(&options.output, &error))?,
        )
    };
    let mut output: Box<dyn Write> = if let Some(file) = file {
        Box::new(file)
    } else {
        Box::new(io::stdout().lock())
    };
    let mut output = BufWriter::with_capacity(64 * 1024, &mut output);
    if let Some(file) = input {
        filter(
            options,
            &mut BufReader::with_capacity(64 * 1024, file),
            &mut output,
            ui,
        )?;
    } else {
        filter(options, &mut io::stdin().lock(), &mut output, ui)?;
    }
    output.flush()
}

fn path_error(path: &Path, error: &io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

fn open_output(path: &Path, input: Option<&File>) -> io::Result<File> {
    let output = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    if output.metadata()?.is_file() {
        let input_id = if let Some(input) = input {
            same_file::Handle::from_file(input.try_clone()?)?
        } else {
            same_file::Handle::stdin()?
        };
        let output_id = same_file::Handle::from_file(output.try_clone()?)?;
        reject_same(&input_id, &output_id)?;
        output.set_len(0)?;
    }
    Ok(output)
}

fn reject_same(input: &same_file::Handle, output: &same_file::Handle) -> io::Result<()> {
    if input == output {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input and output are the same file",
        ))
    } else {
        Ok(())
    }
}

struct Writer<'a, W> {
    options: &'a Options,
    output: &'a mut W,
    wrote_group: bool,
}

impl<W: Write> Writer<'_, W> {
    fn start(&mut self, separator: Separator) -> io::Result<()> {
        if matches!(separator, Separator::Before | Separator::Both)
            || (self.wrote_group && matches!(separator, Separator::Between))
        {
            self.output.write_all(&[self.options.end])?;
        }
        self.wrote_group = true;
        Ok(())
    }

    fn record(&mut self, record: &[u8], count: u64) -> io::Result<()> {
        if self.options.count {
            write!(self.output, "{count:>7} ")?;
        }
        self.output.write_all(record)?;
        self.output.write_all(&[self.options.end])
    }

    fn finish(&mut self, record: &[u8], count: u64) -> io::Result<()> {
        if let Some(method) = self.options.group {
            if matches!(method, Separator::After | Separator::Both) {
                self.output.write_all(&[self.options.end])?;
            }
        } else if self.options.repeated.is_none()
            && match count {
                1 => !self.options.repeated_only,
                _ => !self.options.unique_only,
            }
        {
            self.record(record, count)?;
        }
        Ok(())
    }
}

fn filter(
    options: &Options,
    input: &mut impl BufRead,
    output: &mut impl Write,
    ui: Presentation,
) -> io::Result<()> {
    let mut writer = Writer {
        options,
        output,
        wrote_group: false,
    };
    let mut first = Vec::new();
    let mut record = Vec::new();
    let mut key = 0..0;
    let mut count = 0u64;
    let unicode = utf8_locale();
    let mut progress = ui.progress(LABEL, "records", true);
    let mut errors = io::stderr().lock();
    loop {
        record.clear();
        let size = input
            .read_until(options.end, &mut record)
            .map_err(|error| path_error(&options.input, &error))?;
        if size == 0 {
            break;
        }
        if record.last() == Some(&options.end) {
            record.pop();
        }
        let next = comparison_key(options, &record, unicode);
        let duplicate = count != 0
            && if matches!(options.case, Case::IgnoreAscii) {
                first[key.clone()].eq_ignore_ascii_case(&record[next.clone()])
            } else {
                first[key.clone()] == record[next.clone()]
            };
        if duplicate {
            count = count.saturating_add(1);
            if options.group.is_some() {
                writer.record(&record, count)?;
            } else if let Some(method) = options.repeated {
                if count == 2 {
                    writer.start(method)?;
                    if !options.unique_only {
                        writer.record(&first, 1)?;
                    }
                }
                writer.record(&record, count)?;
            }
        } else {
            if count != 0 {
                writer.finish(&first, count)?;
            }
            if let Some(method) = options.group {
                // Both mode shares a single separator between adjacent groups.
                if !matches!(method, Separator::Both) || !writer.wrote_group {
                    writer.start(method)?;
                }
                writer.record(&record, 1)?;
            }
            std::mem::swap(&mut first, &mut record);
            key = next;
            count = 1;
        }
        if let Some(progress) = &mut progress {
            progress.advance(1, &mut errors);
        }
    }
    if count != 0 {
        writer.finish(&first, count)?;
    }
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(())
}

fn utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
}

fn comparison_key(options: &Options, bytes: &[u8], unicode: bool) -> Range<usize> {
    let mut remainder = bytes;
    for _ in 0..options.fields {
        if remainder.is_empty() {
            break;
        }
        remainder = skip_while(remainder, unicode, true);
        remainder = skip_while(remainder, unicode, false);
    }
    remainder = skip_units(remainder, options.skip, unicode);
    let start = bytes.len() - remainder.len();
    let end = if options.width == usize::MAX {
        bytes.len()
    } else {
        bytes.len() - skip_units(remainder, options.width, unicode).len()
    };
    start..end
}

fn skip_while(mut bytes: &[u8], unicode: bool, blank: bool) -> &[u8] {
    while !bytes.is_empty() {
        let (width, is_blank) = unit(bytes, unicode);
        if is_blank != blank {
            break;
        }
        bytes = &bytes[width..];
    }
    bytes
}

fn skip_units(mut bytes: &[u8], count: usize, unicode: bool) -> &[u8] {
    if !unicode {
        return &bytes[count.min(bytes.len())..];
    }
    for _ in 0..count {
        if bytes.is_empty() {
            break;
        }
        bytes = &bytes[unit(bytes, true).0..];
    }
    bytes
}

fn unit(bytes: &[u8], unicode: bool) -> (usize, bool) {
    let first = bytes[0];
    let width = match first {
        0xc2..=0xdf if unicode => 2,
        0xe0..=0xef if unicode => 3,
        0xf0..=0xf4 if unicode => 4,
        _ => return (1, matches!(first, b' ' | b'\t')),
    };
    let character = bytes
        .get(..width)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(|text| text.chars().next());
    character.map_or((1, false), |character| (
        width,
        matches!(character, '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' | '\u{205f}' | '\u{3000}')
    ))
}
