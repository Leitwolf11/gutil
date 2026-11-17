use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::jobs;
use crate::presentation::Presentation;

mod count;

const HELP: &str = "Usage: gutil wc [OPTION]... [FILE]...\n\nCount newlines, words, characters, bytes, or maximum display width.\nRead stdin when FILE is '-' or omitted.\n\n  -c, --bytes             Count bytes\n  -m, --chars             Count characters\n  -l, --lines             Count newlines\n  -L, --max-line-length   Measure the widest line\n  -w, --words             Count words\n      --files0-from=FILE  Read NUL-delimited file names\n      --total=MODE        auto, always, only, or never\n      --help              Print help\n      --version           Print version\n";

const LINES: u8 = 1 << 0;
const WORDS: u8 = 1 << 1;
const CHARS: u8 = 1 << 2;
const BYTES: u8 = 1 << 3;
const WIDTH: u8 = 1 << 4;
const LABEL: &str = "gutil wc";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Total {
    #[default]
    Auto,
    Always,
    Only,
    Never,
}

#[derive(Default)]
struct Options {
    metrics: u8,
    paths: Vec<PathBuf>,
    files0: Option<PathBuf>,
    total: Total,
}

enum Request {
    Help,
    Version,
    Count(Options, jobs::Count),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("--files0-from requires a path")]
    MissingList,
    #[error("--files0-from cannot be combined with path operands")]
    ConflictingPaths,
    #[error("--total expects auto, always, only, or never")]
    Total,
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let (options, workers) = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Count(options, workers)) => (options, workers),
        Err(error) => {
            ui.error(&mut io::stderr(), LABEL, &error.to_string())?;
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(workers.get())
        .build()?;
    runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(options, ui))
            .await
            .map_err(io::Error::other)?
    })
}

impl Options {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
    ) -> Result<Request, ParseError> {
        let mut options = Self::default();
        let mut flags = true;
        while let Some(arg) = args.next() {
            if !flags {
                options.paths.push(arg.into());
                continue;
            }
            let bytes = arg.as_encoded_bytes();
            if let Some(count) = jobs::option(bytes, &mut args) {
                workers = count?;
                continue;
            }
            let metric = match bytes {
                b"--" => {
                    flags = false;
                    continue;
                }
                b"--help" => return Ok(Request::Help),
                b"--version" => return Ok(Request::Version),
                b"--bytes" => BYTES,
                b"--chars" => CHARS,
                b"--lines" => LINES,
                b"--max-line-length" => WIDTH,
                b"--words" => WORDS,
                b"--files0-from" => {
                    options.files0 = Some(args.next().ok_or(ParseError::MissingList)?.into());
                    continue;
                }
                b"--total" => {
                    let value = args.next().ok_or(ParseError::Total)?;
                    options.set_total(&value)?;
                    continue;
                }
                [b'-', b'-', ..] => {
                    options.long(&arg)?;
                    continue;
                }
                [b'-', short @ ..] if !short.is_empty() => {
                    for (index, flag) in short.iter().enumerate() {
                        if *flag == b'j' {
                            workers = jobs::short_value(&short[index + 1..], &mut args)?;
                            break;
                        }
                        options.metrics |= match flag {
                            b'c' => BYTES,
                            b'm' => CHARS,
                            b'l' => LINES,
                            b'L' => WIDTH,
                            b'w' => WORDS,
                            _ => return Err(ParseError::Option(arg)),
                        };
                    }
                    continue;
                }
                _ => {
                    options.paths.push(arg.into());
                    continue;
                }
            };
            options.metrics |= metric;
        }
        if options.files0.is_some() && !options.paths.is_empty() {
            return Err(ParseError::ConflictingPaths);
        }
        if options.metrics == 0 {
            options.metrics = LINES | WORDS | BYTES;
        }
        Ok(Request::Count(options, workers))
    }

    fn long(&mut self, argument: &OsStr) -> Result<(), ParseError> {
        let text = argument
            .to_str()
            .ok_or_else(|| ParseError::Option(argument.into()))?;
        if let Some(value) = text.strip_prefix("--files0-from=") {
            self.files0 = Some(PathBuf::from(value));
        } else if let Some(value) = text.strip_prefix("--total=") {
            self.set_total(OsStr::new(value))?;
        } else {
            return Err(ParseError::Option(argument.into()));
        }
        Ok(())
    }
    fn set_total(&mut self, value: &OsStr) -> Result<(), ParseError> {
        self.total = match value.as_encoded_bytes() {
            b"auto" => Total::Auto,
            b"always" => Total::Always,
            b"only" => Total::Only,
            b"never" => Total::Never,
            _ => return Err(ParseError::Total),
        };
        Ok(())
    }
}

fn execute(mut options: Options, ui: Presentation) -> io::Result<u8> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    let mut stderr = io::stderr().lock();
    let mut stdin_list = false;
    let mut implicit = false;
    let mut failed = false;
    if let Some(list) = options.files0.take() {
        stdin_list = list == Path::new("-");
        let paths = if stdin_list {
            read_paths(io::stdin().lock())
        } else {
            File::open(&list).and_then(|file| read_paths(BufReader::new(file)))
        };
        match paths {
            Ok(paths) => options.paths = paths,
            Err(error) => {
                ui.error(&mut stderr, LABEL, &format!("{}: {error}", list.display()))?;
                return Ok(1);
            }
        }
    } else if options.paths.is_empty() {
        implicit = true;
        options.paths.push(PathBuf::from("-"));
    }
    let formatting_width = field_width(&options.paths, options.metrics, stdin_list);
    let locale = count::Locale::detect();
    let mut total = count::Counts::default();
    let mut progress = ui.progress(LABEL, "bytes", true);
    let mut buffer = vec![0; count::BUFFER_SIZE];
    for path in &options.paths {
        let result = count_input(path, stdin_list, options.metrics, locale, &mut buffer);
        match result {
            Ok(counts) => {
                total.add(counts);
                if let Some(progress) = &mut progress {
                    progress.advance(counts.bytes, &mut stderr);
                }
                if options.total != Total::Only {
                    write_counts(
                        &mut stdout,
                        counts,
                        options.metrics,
                        formatting_width,
                        (!implicit).then_some(path.as_os_str()),
                    )?;
                }
            }
            Err(error) => {
                failed = true;
                ui.error(&mut stderr, LABEL, &format!("{}: {error}", path.display()))?;
            }
        }
    }
    if matches!(options.total, Total::Always | Total::Only)
        || (options.total == Total::Auto && options.paths.len() > 1)
    {
        let only = options.total == Total::Only;
        write_counts(
            &mut stdout,
            total,
            options.metrics,
            if only { 1 } else { formatting_width },
            (!only).then_some(OsStr::new("total")),
        )?;
    }
    stdout.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut stderr);
    }
    Ok(u8::from(failed))
}

fn count_input(
    path: &Path,
    stdin_list: bool,
    metrics: u8,
    locale: count::Locale,
    buffer: &mut [u8],
) -> io::Result<count::Counts> {
    if path.as_os_str().is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty file name",
        ))
    } else if path == Path::new("-") {
        if stdin_list {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "standard input is already reading file names",
            ))
        } else {
            count::read(&mut io::stdin().lock(), metrics, locale, buffer)
        }
    } else {
        File::open(path).and_then(|mut file| {
            if metrics == BYTES {
                let metadata = file.metadata()?;
                if metadata.is_file() && metadata.len() != 0 {
                    return Ok(count::Counts {
                        bytes: metadata.len(),
                        ..count::Counts::default()
                    });
                }
            }
            count::read(&mut file, metrics, locale, buffer)
        })
    }
}

fn read_paths(reader: impl BufRead) -> io::Result<Vec<PathBuf>> {
    reader
        .split(0)
        .map(|path| {
            let path = path?;
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                Ok(PathBuf::from(OsString::from_vec(path)))
            }
            #[cfg(not(unix))]
            {
                String::from_utf8(path)
                    .map(PathBuf::from)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
            }
        })
        .collect()
}

fn field_width(paths: &[PathBuf], metrics: u8, stdin_list: bool) -> usize {
    if paths.len() == 1 && metrics.is_power_of_two() {
        return 1;
    }
    let mut bytes = 0u64;
    let mut minimum = 1;
    for path in paths {
        if path == Path::new("-") {
            if !stdin_list {
                minimum = 7;
            }
        } else if let Ok(metadata) = fs::metadata(path) {
            if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            } else {
                minimum = 7;
            }
        }
    }
    (if bytes == 0 {
        1
    } else {
        bytes.ilog10() as usize + 1
    })
    .max(minimum)
}

fn write_counts(
    output: &mut impl Write,
    counts: count::Counts,
    metrics: u8,
    width: usize,
    path: Option<&OsStr>,
) -> io::Result<()> {
    let mut selected = [
        (LINES, counts.lines),
        (WORDS, counts.words),
        (CHARS, counts.chars),
        (BYTES, counts.bytes),
        (WIDTH, counts.width),
    ]
    .into_iter()
    .filter_map(|(flag, value)| (metrics & flag != 0).then_some(value));
    if let Some(first) = selected.next() {
        write!(output, "{first:>width$}")?;
    }
    for value in selected {
        write!(output, " {value:>width$}")?;
    }
    if let Some(path) = path {
        output.write_all(b" ")?;
        output.write_all(path.as_encoded_bytes())?;
    }
    output.write_all(b"\n")
}
