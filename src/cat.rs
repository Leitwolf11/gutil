use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use same_file::Handle;
use smallvec::SmallVec;

use crate::jobs;

const HELP: &str = "Usage: gutil cat [OPTION]... [FILE]...\n\nCopy files to standard output. With no files, or when FILE is -, read standard input.\n\n  -A, --show-all         Show nonprinting bytes, ends, and tabs\n  -b, --number-nonblank  Number nonempty lines and override -n\n  -e                     Equivalent to -vE\n  -E, --show-ends        Mark line endings with $\n  -n, --number           Number all lines\n  -s, --squeeze-blank    Collapse repeated empty lines\n  -t                     Equivalent to -vT\n  -T, --show-tabs        Mark tabs with ^I\n  -u                     Accepted for POSIX compatibility\n  -v, --show-nonprinting Show nonprinting bytes\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n      --help             Print help\n      --version          Print version\n";

#[derive(Default, PartialEq, Eq)]
enum Number {
    #[default]
    None,
    All,
    Nonempty,
}

#[derive(Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Display options can be combined independently.
struct Format {
    number: Number,
    squeeze: bool,
    ends: bool,
    tabs: bool,
    nonprinting: bool,
}

impl Format {
    fn flag(&mut self, flag: u8) -> bool {
        match flag {
            b'A' => {
                self.ends = true;
                self.tabs = true;
                self.nonprinting = true;
            }
            b'b' => self.number = Number::Nonempty,
            b'e' => {
                self.ends = true;
                self.nonprinting = true;
            }
            b'E' => self.ends = true,
            b'n' if self.number != Number::Nonempty => self.number = Number::All,
            b's' => self.squeeze = true,
            b't' => {
                self.tabs = true;
                self.nonprinting = true;
            }
            b'T' => self.tabs = true,
            b'v' => self.nonprinting = true,
            b'n' | b'u' => {}
            _ => return false,
        }
        true
    }
}

enum Request {
    Copy(Format, SmallVec<[OsString; 4]>, usize),
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    InvalidOption(OsString),
    #[error(transparent)]
    InvalidJobs(#[from] jobs::InvalidCount),
}

fn parse(
    mut args: impl Iterator<Item = OsString>,
    mut job_count: usize,
) -> Result<Request, ParseError> {
    let mut format = Format::default();
    let mut files = SmallVec::new();
    let mut parse_options = true;
    while let Some(arg) = args.next() {
        if !parse_options {
            files.push(arg);
            continue;
        }
        let bytes = arg.as_encoded_bytes();
        if let Some(count) = jobs::option(bytes, &mut args) {
            job_count = count?;
            continue;
        }
        let flag = match bytes {
            b"--" => {
                parse_options = false;
                continue;
            }
            b"--help" => return Ok(Request::Help),
            b"--version" => return Ok(Request::Version),
            b"--show-all" => b'A',
            b"--number-nonblank" => b'b',
            b"--show-ends" => b'E',
            b"--number" => b'n',
            b"--squeeze-blank" => b's',
            b"--show-tabs" => b'T',
            b"--show-nonprinting" => b'v',
            [b'-', b'-', ..] => return Err(ParseError::InvalidOption(arg)),
            [b'-', flags @ ..] if !flags.is_empty() => {
                for (index, &flag) in flags.iter().enumerate() {
                    if flag == b'j' {
                        job_count = jobs::short_value(&flags[index + 1..], &mut args)?;
                        break;
                    }
                    if !format.flag(flag) {
                        return Err(ParseError::InvalidOption(arg));
                    }
                }
                continue;
            }
            _ => {
                files.push(arg);
                continue;
            }
        };
        format.flag(flag);
    }
    if files.is_empty() {
        files.push(OsString::from("-"));
    }
    Ok(Request::Copy(format, files, job_count))
}

pub(super) fn run(args: impl Iterator<Item = OsString>, job_count: usize) -> io::Result<u8> {
    match parse(args, job_count) {
        Ok(Request::Help) => io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            writeln!(io::stdout(), "gutil cat {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        Ok(Request::Copy(format, files, count)) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(count)
                .build()?;
            runtime.block_on(async move {
                tokio::task::spawn_blocking(move || copy_files(format, files))
                    .await
                    .map_err(io::Error::other)?
            })
        }
        Err(error) => {
            writeln!(
                io::stderr(),
                "gutil cat: {error}\nTry 'gutil cat --help' for more information."
            )?;
            Ok(1)
        }
    }
}

fn copy_files(format: Format, files: SmallVec<[OsString; 4]>) -> io::Result<u8> {
    let destination = Handle::stdout().ok().filter(|handle| {
        handle
            .as_file()
            .metadata()
            .is_ok_and(|metadata| metadata.is_file())
    });
    let mut output = io::stdout().lock();
    let mut errors = io::stderr().lock();
    let mut stream = Stream::new(format);
    let mut status = 0;

    for operand in files {
        let result = if operand == "-" {
            let is_destination = destination.as_ref().is_some_and(|destination| {
                Handle::stdin().is_ok_and(|input| &input == destination)
            });
            if is_destination {
                Some(same_file_error())
            } else {
                stream.copy(&mut io::stdin().lock(), &mut output)?
            }
        } else {
            match File::open(Path::new(&operand)).and_then(Handle::from_file) {
                Ok(input) if destination.as_ref() == Some(&input) => Some(same_file_error()),
                Ok(input) => stream.copy(&mut input.as_file(), &mut output)?,
                Err(error) => Some(error),
            }
        };

        if let Some(error) = result {
            report(&mut errors, &operand, &error)?;
            status = 1;
        }
    }

    if stream.pending_cr {
        output.write_all(b"\r")?;
    }
    output.flush()?;
    Ok(status)
}

fn same_file_error() -> io::Error {
    io::Error::other("input file is output file")
}

#[allow(clippy::unnecessary_debug_formatting)] // Escape control characters in operands.
fn report(output: &mut impl Write, operand: &OsStr, error: &io::Error) -> io::Result<()> {
    writeln!(output, "gutil cat: {operand:?}: {error}")
}

struct Stream {
    format: Format,
    input: Vec<u8>,
    output: Vec<u8>,
    line_number: u64,
    line_start: bool,
    previous_blank: bool,
    pending_cr: bool,
}

impl Stream {
    fn new(format: Format) -> Self {
        Self {
            format,
            input: vec![0; 64 * 1024],
            output: Vec::new(),
            line_number: 1,
            line_start: true,
            previous_blank: false,
            pending_cr: false,
        }
    }

    fn copy(
        &mut self,
        input: &mut impl Read,
        output: &mut impl Write,
    ) -> io::Result<Option<io::Error>> {
        loop {
            let read = match input.read(&mut self.input) {
                Ok(0) => return Ok(None),
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Ok(Some(error)),
            };

            if self.format == Format::default() {
                output.write_all(&self.input[..read])?;
            } else {
                self.output.clear();
                for index in 0..read {
                    self.format_byte(self.input[index])?;
                }
                output.write_all(&self.output)?;
            }
            output.flush()?;
        }
    }

    fn format_byte(&mut self, byte: u8) -> io::Result<()> {
        if self.pending_cr {
            self.output
                .extend_from_slice(if byte == b'\n' { b"^M" } else { b"\r" });
            self.pending_cr = false;
        }

        if byte == b'\n' {
            if self.line_start && self.previous_blank && self.format.squeeze {
                return Ok(());
            }
            if self.line_start && self.format.number == Number::All {
                self.write_number()?;
            }
            if self.format.ends {
                self.output.push(b'$');
            }
            self.output.push(byte);
            self.previous_blank = self.line_start;
            self.line_start = true;
            return Ok(());
        }

        if self.line_start && self.format.number != Number::None {
            self.write_number()?;
        }
        self.line_start = false;
        match byte {
            b'\r' if self.format.ends && !self.format.nonprinting => self.pending_cr = true,
            b'\t' if self.format.tabs => self.output.extend_from_slice(b"^I"),
            b'\t' => self.output.push(byte),
            _ if self.format.nonprinting => show_byte(&mut self.output, byte),
            _ => self.output.push(byte),
        }
        Ok(())
    }

    fn write_number(&mut self) -> io::Result<()> {
        write!(self.output, "{:>6}\t", self.line_number)?;
        self.line_number = self.line_number.saturating_add(1);
        Ok(())
    }
}

fn show_byte(output: &mut Vec<u8>, mut byte: u8) {
    if byte >= 128 {
        output.extend_from_slice(b"M-");
        byte -= 128;
    }
    match byte {
        0..=31 => output.extend_from_slice(&[b'^', byte + b'@']),
        127 => output.extend_from_slice(b"^?"),
        _ => output.push(byte),
    }
}
