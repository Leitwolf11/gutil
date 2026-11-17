use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

mod stream;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Base {
    Base32,
    Base64,
}

impl Base {
    const fn label(self) -> &'static str {
        match self {
            Self::Base32 => "gutil base32",
            Self::Base64 => "gutil base64",
        }
    }
}

struct Options {
    mode: Mode,
    wrap: stream::Wrap,
    path: Option<PathBuf>,
    workers: jobs::Count,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Encode,
    Decode,
    DecodeIgnoringGarbage,
}

enum Request {
    Help,
    Version,
    Convert(Options),
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("extra operand {0:?}")]
    Operand(OsString),
    #[error("--wrap requires a column count")]
    MissingWidth,
    #[error("invalid wrap width")]
    Width,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

impl Options {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
    ) -> Result<Request, Error> {
        let mut decode = false;
        let mut ignore_garbage = false;
        let mut wrap = stream::Wrap::new(76);
        let mut path = None;
        let mut flags = true;
        while let Some(arg) = args.next() {
            let bytes = arg.as_encoded_bytes();
            if !flags || arg == "-" || !bytes.starts_with(b"-") {
                if path.replace(PathBuf::from(&arg)).is_some() {
                    return Err(Error::Operand(arg));
                }
                continue;
            }
            if let Some(count) = jobs::option(bytes, &mut args) {
                workers = count?;
                continue;
            }
            match bytes {
                b"--" => flags = false,
                b"--help" => return Ok(Request::Help),
                b"--version" => return Ok(Request::Version),
                b"--decode" => decode = true,
                b"--ignore-garbage" => ignore_garbage = true,
                b"--wrap" => wrap = wrap_width(None, &mut args)?,
                _ if bytes.starts_with(b"--wrap=") => {
                    wrap = wrap_width(Some(&bytes[7..]), &mut args)?;
                }
                [b'-', short @ ..] if !short.starts_with(b"-") => {
                    for (index, flag) in short.iter().enumerate() {
                        let remaining = &short[index + 1..];
                        match flag {
                            b'd' => decode = true,
                            b'i' => ignore_garbage = true,
                            b'w' => {
                                wrap = wrap_width(
                                    (!remaining.is_empty()).then_some(remaining),
                                    &mut args,
                                )?;
                                break;
                            }
                            b'j' => {
                                workers = jobs::short_value(remaining, &mut args)?;
                                break;
                            }
                            _ => return Err(Error::Option(arg)),
                        }
                    }
                }
                _ => return Err(Error::Option(arg)),
            }
        }
        let mode = if decode {
            if ignore_garbage {
                Mode::DecodeIgnoringGarbage
            } else {
                Mode::Decode
            }
        } else {
            Mode::Encode
        };
        Ok(Request::Convert(Self {
            mode,
            wrap,
            path,
            workers,
        }))
    }
}

fn wrap_width(
    attached: Option<&[u8]>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<stream::Wrap, Error> {
    let separate;
    let value = if let Some(value) = attached {
        value
    } else {
        separate = args.next().ok_or(Error::MissingWidth)?;
        separate.as_encoded_bytes()
    };
    let text = std::str::from_utf8(value).map_err(|_| Error::Width)?;
    let width = text.parse::<usize>().map_err(|_| Error::Width)?;
    Ok(stream::Wrap::new(width))
}

pub(crate) fn run(
    base: Base,
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return writeln!(
            io::stdout(),
            "Usage: {} [OPTIONS] [FILE]\n\nEncode or decode FILE, or read standard input for '-' or no FILE.\n\n  -d, --decode           Decode input\n  -i, --ignore-garbage   Skip nonalphabet bytes while decoding\n  -w, --wrap=COLS        Wrap at COLS columns, default 76, or use 0 for no wrapping\n  -j, --jobs=N          Set the filesystem worker limit\n      --help            Print help\n      --version         Print version",
            base.label()
        ).map(|()| 0),
        Ok(Request::Version) => return writeln!(io::stdout(), "{} {}", base.label(), env!("CARGO_PKG_VERSION")).map(|()| 0),
        Ok(Request::Convert(options)) => options,
        Err(error) => {
            ui.error(&mut io::stderr(), base.label(), &error.to_string())?;
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    let result = runtime
        .block_on(
            async move { tokio::task::spawn_blocking(move || convert(base, &options, ui)).await },
        )
        .map_err(io::Error::other)?;
    match result {
        Ok(()) => Ok(0),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Err(error),
        Err(error) => {
            ui.error(&mut io::stderr(), base.label(), &error.to_string())?;
            Ok(1)
        }
    }
}

fn convert(base: Base, options: &Options, ui: Presentation) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = BufWriter::with_capacity(64 * 1024, stdout.lock());
    let mut errors = io::stderr().lock();
    let path = options
        .path
        .as_deref()
        .filter(|path| *path != Path::new("-"));
    let mut progress = ui.progress(
        base.label(),
        "bytes",
        !(stdin.is_terminal() && path.is_none()),
    );
    let result = if let Some(path) = path {
        let mut file = File::open(path).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", path.display()))
        })?;
        if let Some(progress) = &mut progress {
            if let Ok(metadata) = file.metadata() {
                if metadata.is_file() {
                    progress.set_total(metadata.len());
                }
            }
        }
        apply(
            base,
            options,
            &mut file,
            &mut output,
            progress.as_mut(),
            &mut errors,
        )
    } else {
        apply(
            base,
            options,
            &mut stdin.lock(),
            &mut output,
            progress.as_mut(),
            &mut errors,
        )
    };
    output.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    result
}

fn apply(
    base: Base,
    options: &Options,
    input: &mut impl Read,
    output: &mut impl Write,
    mut progress: Option<&mut ProgressReport>,
    errors: &mut impl Write,
) -> io::Result<()> {
    let mut advance = |bytes: usize| {
        if let Some(progress) = &mut progress {
            progress.advance(bytes as u64, errors);
        }
    };
    match options.mode {
        Mode::Encode => stream::encode(base, options.wrap, input, output, &mut advance),
        Mode::Decode | Mode::DecodeIgnoringGarbage => stream::decode(
            base,
            options.mode == Mode::DecodeIgnoringGarbage,
            input,
            output,
            &mut advance,
        ),
    }
}
