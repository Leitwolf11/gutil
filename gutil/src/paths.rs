use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, Write};

use smallvec::SmallVec;

use crate::{jobs, presentation::Presentation};

pub(crate) mod canonical;
mod names;
mod resolve;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Basename,
    Dirname,
    Readlink,
    Realpath,
}

impl Command {
    const fn label(self) -> &'static str {
        match self {
            Self::Basename => "gutil basename",
            Self::Dirname => "gutil dirname",
            Self::Readlink => "gutil readlink",
            Self::Realpath => "gutil realpath",
        }
    }

    const fn help(self) -> &'static str {
        match self {
            Self::Basename => "Usage: gutil basename NAME [SUFFIX]\n       gutil basename [OPTION]... NAME...\n\nPrint the final component of each name.\n\n  -a, --multiple       Accept multiple names\n  -s, --suffix=SUFFIX  Remove a suffix and accept multiple names\n  -z, --zero           End each result with NUL\n",
            Self::Dirname => "Usage: gutil dirname [OPTION]... NAME...\n\nPrint each name's directory portion.\n\n  -z, --zero  End each result with NUL\n",
            Self::Readlink => "Usage: gutil readlink [OPTION]... FILE...\n\nPrint symbolic link targets or resolved file names.\n\n  -f, --canonicalize           Allow a missing final component\n  -e, --canonicalize-existing  Require every component to exist\n  -m, --canonicalize-missing   Allow missing components\n  -n, --no-newline             Omit the delimiter for one operand\n  -q, -s, --quiet, --silent    Suppress diagnostics\n  -v, --verbose                Report failures\n  -z, --zero                   End each result with NUL\n",
            Self::Realpath => "Usage: gutil realpath [OPTION]... FILE...\n\nPrint resolved absolute or relative file names.\n\n  -E, --canonicalize           Allow a missing final component (default)\n  -e, --canonicalize-existing  Require every component to exist\n  -m, --canonicalize-missing   Allow missing components\n  -L, --logical                Process parent components before expanding links\n  -P, --physical               Expand links as they are encountered\n  -s, --strip, --no-symlinks   Skip symbolic link expansion\n  -q, --quiet                  Suppress diagnostics\n  -z, --zero                   End each result with NUL\n      --relative-to=DIR        Print paths relative to DIR\n      --relative-base=DIR      Keep paths outside DIR absolute\n",
        }
    }

    const fn is_name(self) -> bool {
        matches!(self, Self::Basename | Self::Dirname)
    }
}

pub(crate) fn run(
    command: Command,
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let request = match parse(command, args, workers) {
        Ok(request) => request,
        Err(error) => {
            ui.error(&mut io::stderr(), command.label(), &error.to_string())?;
            return Ok(1);
        }
    };
    let mut output = BufWriter::new(io::stdout().lock());
    let status = match request {
        Request::Help => {
            output.write_all(command.help().as_bytes())?;
            output.write_all(b"      --help     Print help\n      --version  Print version\n")?;
            0
        }
        Request::Version => {
            writeln!(output, "{} {}", command.label(), env!("CARGO_PKG_VERSION"))?;
            0
        }
        Request::Names(options) => names::write(command, &options, &mut output)?,
        Request::Resolve(options, workers) => {
            resolve::write(command, *options, workers, ui, &mut output)?
        }
    };
    output.flush()?;
    Ok(status)
}

enum Request {
    Help,
    Version,
    Names(names::Options),
    Resolve(Box<resolve::Options>, jobs::Count),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    Unknown(OsString),
    #[error("{0} requires an argument")]
    MissingValue(&'static str),
    #[error("missing operand")]
    MissingOperand,
    #[error("extra operand {0:?}")]
    ExtraOperand(OsString),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

impl ParseError {
    fn unknown(argument: &OsStr) -> Self {
        Self::Unknown(argument.into())
    }
}

type Operands = SmallVec<[OsString; 4]>;

fn parse(
    command: Command,
    mut args: impl Iterator<Item = OsString>,
    mut workers: jobs::Count,
) -> Result<Request, ParseError> {
    let mut names = names::Options::default();
    let mut resolve = (!command.is_name()).then(|| resolve::Options::new(command));
    let mut operands = Operands::new();
    let mut flags = true;
    while let Some(arg) = args.next() {
        let bytes = arg.as_encoded_bytes();
        if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
            operands.push(arg);
            // basename keeps its optional positional suffix after the first operand.
            flags &= command != Command::Basename;
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
            _ => {
                if let Some(options) = &mut resolve {
                    options.option(command, &arg, &mut args, &mut workers)?;
                } else {
                    names.option(command, &arg, &mut args, &mut workers)?;
                }
            }
        }
    }
    if operands.is_empty() {
        return Err(ParseError::MissingOperand);
    }
    if let Some(mut options) = resolve {
        options.paths = operands;
        Ok(Request::Resolve(Box::new(options), workers))
    } else {
        names.finish(command, operands).map(Request::Names)
    }
}

fn long_value(
    arg: &OsStr,
    flag: &'static str,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<OsString, ParseError> {
    if arg.as_encoded_bytes().len() == flag.len() {
        return args.next().ok_or(ParseError::MissingValue(flag));
    }
    Ok(encoded_name(&arg.as_encoded_bytes()[flag.len() + 1..]).into_owned())
}

fn encoded_name(bytes: &[u8]) -> Cow<'_, OsStr> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Cow::Borrowed(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        match String::from_utf8_lossy(bytes) {
            Cow::Borrowed(value) => Cow::Borrowed(OsStr::new(value)),
            Cow::Owned(value) => Cow::Owned(value.into()),
        }
    }
}

fn output_path(output: &mut impl Write, path: &OsStr, delimiter: Option<u8>) -> io::Result<()> {
    output.write_all(path.as_encoded_bytes())?;
    if let Some(delimiter) = delimiter {
        output.write_all(&[delimiter])?;
    }
    Ok(())
}
