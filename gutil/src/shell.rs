use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, Write};

use crate::presentation::Presentation;

mod output;
mod system;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    True,
    False,
    Echo,
    Yes,
    Printenv,
    Nproc,
    Sleep,
}

impl Command {
    pub(crate) fn parse(name: &OsStr) -> Option<Self> {
        match name.as_encoded_bytes() {
            b"true" => Some(Self::True),
            b"false" => Some(Self::False),
            b"echo" => Some(Self::Echo),
            b"yes" => Some(Self::Yes),
            b"printenv" => Some(Self::Printenv),
            b"nproc" => Some(Self::Nproc),
            b"sleep" => Some(Self::Sleep),
            _ => None,
        }
    }

    const fn definition(self) -> (&'static str, &'static str) {
        match self {
            Self::True => ("true", "[ARGUMENTS]...\n\nReturn success."),
            Self::False => ("false", "[ARGUMENTS]...\n\nReturn failure."),
            Self::Echo => ("echo", "[-n] [-e|-E] [STRING]...\n\nWrite arguments separated by spaces.\n\n  -n  Omit the final newline\n  -e  Interpret backslash escapes\n  -E  Print backslashes literally"),
            Self::Yes => ("yes", "[--] [STRING]...\n\nRepeat a line of STRING values, or y when no arguments are given."),
            Self::Printenv => ("printenv", "[-0] [VARIABLE]...\n\nPrint selected environment values, or every name and value.\n\n  -0, --null  Delimit entries with NUL"),
            Self::Nproc => ("nproc", "[OPTION]...\n\nPrint the available processor count.\n\n  --all       Query installed processors when supported\n  --ignore=N  Subtract N, with a minimum result of one"),
            Self::Sleep => ("sleep", "NUMBER[SUFFIX]...\n\nPause for the sum of the supplied durations.\nSuffixes s, m, h, and d denote seconds, minutes, hours, and days."),
        }
    }

    fn label(self) -> String {
        format!("gutil {}", self.definition().0)
    }
}

#[derive(Clone, Copy)]
enum Info {
    Help,
    Version,
}

impl Info {
    fn parse(value: &OsStr) -> Option<Self> {
        match value.as_encoded_bytes() {
            b"--help" => Some(Self::Help),
            b"--version" => Some(Self::Version),
            _ => None,
        }
    }

    fn write(self, command: Command, output: &mut impl Write) -> io::Result<()> {
        let (name, usage) = command.definition();
        match self {
            Self::Help => writeln!(output, "Usage: gutil {name} {usage}\n\n  --help     Print help\n  --version  Print version"),
            Self::Version => writeln!(output, "gutil {name} {}", env!("CARGO_PKG_VERSION")),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("extra operand {0:?}")]
    Operand(OsString),
    #[error("{0} requires a value")]
    MissingValue(&'static str),
    #[error("invalid count {0:?}")]
    Count(OsString),
    #[error("missing duration")]
    MissingDuration,
    #[error("invalid duration {0:?}")]
    Duration(OsString),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub(crate) fn run(
    command: Command,
    args: impl Iterator<Item = OsString>,
    ui: Presentation,
) -> io::Result<u8> {
    let mut args = args.peekable();
    if matches!(command, Command::True | Command::False) {
        let status = u8::from(command == Command::False);
        if let Some(first) = args.next() {
            if let Some(info) = Info::parse(&first).filter(|_| args.peek().is_none()) {
                info.write(command, &mut io::stdout().lock())?;
            }
        }
        return Ok(status);
    }

    let mut output = BufWriter::new(io::stdout().lock());
    let result = match command {
        Command::Echo => output::echo(args, &mut output).map(|()| 0),
        Command::Yes => output::yes(args, &mut output).map(|()| 0),
        Command::Printenv => output::printenv(args, &mut output),
        Command::Nproc => system::nproc(args, &mut output).map(|()| 0),
        Command::Sleep => system::sleep(args, &mut output).map(|()| 0),
        _ => unreachable!(),
    };
    match result.and_then(|status| output.flush().map(|()| status).map_err(Error::from)) {
        Ok(status) => Ok(status),
        Err(Error::Io(error))
            if error.kind() == io::ErrorKind::BrokenPipe && command != Command::Yes =>
        {
            Err(error)
        }
        Err(error) => {
            ui.error(&mut io::stderr(), &command.label(), &error.to_string())?;
            Ok(if command == Command::Printenv { 2 } else { 1 })
        }
    }
}

fn posix() -> bool {
    env::var_os("POSIXLY_CORRECT").is_some()
}
