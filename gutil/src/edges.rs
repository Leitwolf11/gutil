use std::ffi::{OsStr, OsString};
use std::fs::{File, Metadata};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use smallvec::SmallVec;

use crate::jobs;
use crate::presentation::Presentation;

mod follow;
mod scan;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Head,
    Tail,
}

impl Edge {
    const fn label(self) -> &'static str {
        match self {
            Self::Head => "gutil head",
            Self::Tail => "gutil tail",
        }
    }
}

#[derive(Clone, Copy)]
enum Unit {
    Bytes,
    Records,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Start,
    End,
}

#[derive(Clone, Copy)]
struct Limit {
    unit: Unit,
    origin: Origin,
    count: u64,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Headers {
    #[default]
    Auto,
    Always,
    Never,
}

struct Options {
    paths: SmallVec<[PathBuf; 4]>,
    limit: Limit,
    headers: Headers,
    delimiter: u8,
    follow: follow::Settings,
}

enum Request {
    Help,
    Version,
    Read(Options, jobs::Count),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("{0} requires an argument")]
    Missing(&'static str),
    #[error("invalid count {0:?}")]
    Count(OsString),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
    #[error("{0}")]
    Follow(&'static str),
}

pub(crate) fn run(
    edge: Edge,
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let (options, workers) = match Options::parse(edge, args, workers) {
        Ok(Request::Help) => return help(edge),
        Ok(Request::Version) => {
            return writeln!(
                io::stdout(),
                "{} {}",
                edge.label(),
                env!("CARGO_PKG_VERSION")
            )
            .map(|()| 0);
        }
        Ok(Request::Read(options, workers)) => (options, workers),
        Err(error) => {
            ui.error(&mut io::stderr(), edge.label(), &error.to_string())?;
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(workers.get())
        .build()?;
    runtime.block_on(async move {
        tokio::task::spawn_blocking(move || {
            let mut stdout = BufWriter::new(io::stdout().lock());
            let mut stderr = io::stderr().lock();
            let result = execute(edge, &options, ui, &mut stdout, &mut stderr)
                .and_then(|status| stdout.flush().map(|()| status));
            match result {
                Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
                    ui.error(&mut stderr, edge.label(), &error.to_string())?;
                    Ok(1)
                }
                result => result,
            }
        })
        .await
        .map_err(io::Error::other)?
    })
}

fn help(edge: Edge) -> io::Result<u8> {
    let label = edge.label();
    let end = match edge {
        Edge::Head => "first",
        Edge::Tail => "last",
    };
    let mut output = io::stdout().lock();
    writeln!(
        output,
        "Usage: {label} [OPTION]... [FILE]...\n\nPrint the {end} 10 lines of each file. Read stdin when FILE is '-' or omitted.\n\n  -c, --bytes=NUM  Select a byte count\n  -n, --lines=NUM  Select a record count\n  -q, --quiet      Suppress file headers\n  -v, --verbose    Always print file headers\n  -z, --zero-terminated  Use NUL as the record delimiter\n      --help       Print help\n      --version    Print version"
    )?;
    match edge {
        Edge::Head => writeln!(output, "\nA negative count omits the last NUM bytes or records."),
        Edge::Tail => writeln!(
            output,
            "  -f, --follow[=descriptor|name]  Follow appended data\n  -F                             Follow by name and retry\n      --retry                    Retry inaccessible files\n  -s, --sleep-interval=SECONDS   Set the follow polling interval\n\nA count beginning with + starts at byte or record NUM."
        ),
    }?;
    Ok(0)
}

impl Options {
    fn parse(
        edge: Edge,
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
    ) -> Result<Request, ParseError> {
        let default_origin = if edge == Edge::Head {
            Origin::Start
        } else {
            Origin::End
        };
        let mut options = Self {
            paths: SmallVec::new(),
            limit: Limit {
                unit: Unit::Records,
                origin: default_origin,
                count: 10,
            },
            headers: Headers::Auto,
            delimiter: b'\n',
            follow: follow::Settings::default(),
        };
        let mut flags = true;
        while let Some(arg) = args.next() {
            if !flags || arg == "-" || !arg.as_encoded_bytes().starts_with(b"-") {
                options.paths.push(PathBuf::from(arg));
                continue;
            }
            if let Some(count) = jobs::option(arg.as_encoded_bytes(), &mut args) {
                workers = count?;
                continue;
            }
            let bytes = arg.as_encoded_bytes();
            match bytes {
                b"--" => flags = false,
                b"--help" => return Ok(Request::Help),
                b"--version" => return Ok(Request::Version),
                b"--quiet" | b"--silent" => options.headers = Headers::Never,
                b"--verbose" => options.headers = Headers::Always,
                b"--zero-terminated" => options.delimiter = 0,
                [b'-', b'-', ..] => options.long(edge, &arg, &mut args)?,
                [b'-', b'0'..=b'9', ..] => {
                    options.limit = parse_count(edge, Unit::Records, &bytes[1..])?;
                }
                [b'-', short @ ..] => options.short(edge, short, &mut args)?,
                _ => return Err(ParseError::Option(arg)),
            }
        }
        if options.paths.is_empty() {
            options.paths.push(PathBuf::from("-"));
        }
        options.follow.validate()?;
        Ok(Request::Read(options, workers))
    }

    fn long(
        &mut self,
        edge: Edge,
        arg: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        let bytes = arg.as_encoded_bytes();
        let (name, value) = bytes
            .iter()
            .position(|byte| *byte == b'=')
            .map_or((bytes, None), |index| {
                (&bytes[..index], Some(&bytes[index + 1..]))
            });
        match name {
            b"--bytes" => self.select(edge, Unit::Bytes, "--bytes", value, args),
            b"--lines" => self.select(edge, Unit::Records, "--lines", value, args),
            _ if edge == Edge::Tail => self.follow.long(name, value, args),
            _ => Err(ParseError::Option(arg.to_os_string())),
        }
    }

    fn short(
        &mut self,
        edge: Edge,
        bytes: &[u8],
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        for (index, flag) in bytes.iter().enumerate() {
            match flag {
                b'q' => self.headers = Headers::Never,
                b'v' => self.headers = Headers::Always,
                b'z' => self.delimiter = 0,
                b'f' if edge == Edge::Tail => self.follow.mode = Some(follow::Mode::Descriptor),
                b'F' if edge == Edge::Tail => self.follow.by_name(),
                b'c' | b'n' => {
                    let (unit, name) = if *flag == b'c' {
                        (Unit::Bytes, "-c")
                    } else {
                        (Unit::Records, "-n")
                    };
                    let value = &bytes[index + 1..];
                    return self.select(
                        edge,
                        unit,
                        name,
                        (!value.is_empty()).then_some(value),
                        args,
                    );
                }
                b's' if edge == Edge::Tail => {
                    let value = &bytes[index + 1..];
                    return self
                        .follow
                        .set_delay((!value.is_empty()).then_some(value), args);
                }
                _ => {
                    return Err(ParseError::Option(OsString::from(format!(
                        "-{}",
                        char::from(*flag)
                    ))))
                }
            }
        }
        Ok(())
    }

    fn select(
        &mut self,
        edge: Edge,
        unit: Unit,
        name: &'static str,
        value: Option<&[u8]>,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        self.limit = match value {
            Some(value) => parse_count(edge, unit, value)?,
            None => parse_count(
                edge,
                unit,
                args.next()
                    .ok_or(ParseError::Missing(name))?
                    .as_encoded_bytes(),
            )?,
        };
        Ok(())
    }

    fn show_headers(&self) -> bool {
        self.headers == Headers::Always || (self.headers == Headers::Auto && self.paths.len() > 1)
    }
}

fn parse_count(edge: Edge, unit: Unit, value: &[u8]) -> Result<Limit, ParseError> {
    let invalid = || ParseError::Count(String::from_utf8_lossy(value).into_owned().into());
    let (unsigned, sign) = match value {
        [sign @ (b'+' | b'-'), unsigned @ ..] => (unsigned, Some(*sign)),
        _ => (value, None),
    };
    let origin = match (edge, sign) {
        (Edge::Head, Some(b'-')) | (Edge::Tail, None | Some(b'-')) => Origin::End,
        _ => Origin::Start,
    };
    let split = unsigned
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(unsigned.len());
    let (digits, suffix) = unsigned.split_at(split);
    let count = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse::<u64>().ok())
        .and_then(|count| count.checked_mul(multiplier(suffix)?))
        .ok_or_else(invalid)?;
    let count = if edge == Edge::Tail && origin == Origin::Start {
        count.saturating_sub(1)
    } else {
        count
    };
    Ok(Limit {
        unit,
        origin,
        count,
    })
}

fn multiplier(suffix: &[u8]) -> Option<u64> {
    match suffix {
        b"" => Some(1),
        b"b" => Some(512),
        [first, rest @ ..] => {
            let power = b"KMGTPEZYRQ"
                .iter()
                .position(|unit| *unit == first.to_ascii_uppercase())?
                + 1;
            let base = match rest {
                b"" | b"iB" => 1024u64,
                b"B" => 1000,
                _ => return None,
            };
            base.checked_pow(u32::try_from(power).ok()?)
        }
    }
}

fn execute(
    edge: Edge,
    options: &Options,
    ui: Presentation,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let mut failed = false;
    let mut previous_header = false;
    let mut followed = Vec::new();
    for path in &options.paths {
        let result = if path == Path::new("-") {
            header(options, path, &mut previous_header, output)?;
            scan::stream(edge, options, &mut io::stdin().lock(), output)
        } else {
            File::open(path).and_then(|mut file| {
                let metadata = file.metadata()?;
                header(options, path, &mut previous_header, output)?;
                scan_file(edge, options, &mut file, &metadata, output)?;
                if options.follow.mode.is_some() {
                    followed.push(follow::Input::opened(path.clone(), file)?);
                }
                Ok(())
            })
        };
        if let Err(error) = result {
            if error.kind() == io::ErrorKind::BrokenPipe {
                return Err(error);
            }
            ui.error(
                errors,
                edge.label(),
                &format!("{}: {error}", path.display()),
            )?;
            if options.follow.retry && path != Path::new("-") {
                followed.push(follow::Input::pending(path.clone()));
            } else {
                failed = true;
            }
        }
    }
    output.flush()?;
    if !followed.is_empty() {
        follow::run(followed, options, ui, output, errors)?;
    }
    Ok(u8::from(failed))
}

fn scan_file(
    edge: Edge,
    options: &Options,
    file: &mut File,
    metadata: &Metadata,
    output: &mut impl Write,
) -> io::Result<()> {
    if options.limit.origin == Origin::End && metadata.is_file() && metadata.len() != 0 {
        return scan::seekable(edge, options, file, metadata.len(), output);
    }
    scan::stream(edge, options, &mut BufReader::new(file), output)
}

fn header(
    options: &Options,
    path: &Path,
    previous: &mut bool,
    output: &mut impl Write,
) -> io::Result<()> {
    if options.show_headers() {
        if *previous {
            output.write_all(b"\n")?;
        }
        output.write_all(b"==> ")?;
        let name = if path == Path::new("-") {
            b"standard input"
        } else {
            path.as_os_str().as_encoded_bytes()
        };
        output.write_all(name)?;
        output.write_all(b" <==\n")?;
        *previous = true;
    }
    Ok(())
}
