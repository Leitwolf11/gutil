use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use crate::jobs;
use crate::presentation::Presentation;

mod graph;

const LABEL: &str = "gutil tsort";
const BUFFER_SIZE: usize = 64 * 1024;
const HELP: &str = "Usage: gutil tsort [OPTIONS] [FILE]\n\nRead pairs of names and print them in dependency order. Use '-' for standard input.\n\n  -j, --jobs, --threads N  Limit runtime workers\n      --help               Print help\n      --version            Print version\n";

struct Options {
    path: PathBuf,
    workers: jobs::Count,
}

enum Request {
    Sort(Options),
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("extra operand {0:?}")]
    Extra(OsString),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

impl Request {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
    ) -> Result<Self, Error> {
        let mut operands = Vec::new();
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(argument) = args.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
                operands.push(argument);
                flags &= !posix;
            } else if bytes == b"--" {
                flags = false;
            } else if let Some(count) = jobs::option(bytes, &mut args) {
                workers = count?;
            } else if let Some(name) = bytes.strip_prefix(b"--") {
                if !name.is_empty() && b"help".starts_with(name) {
                    return Ok(Self::Help);
                }
                if !name.is_empty() && b"version".starts_with(name) {
                    return Ok(Self::Version);
                }
                return Err(Error::Option(argument));
            } else {
                for (index, &flag) in bytes[1..].iter().enumerate() {
                    match flag {
                        b'w' => {}
                        b'j' => {
                            workers = jobs::short_value(&bytes[index + 2..], &mut args)?;
                            break;
                        }
                        _ => return Err(Error::Option(argument)),
                    }
                }
            }
        }
        let mut operands = operands.into_iter();
        let path = operands
            .next()
            .map_or_else(|| PathBuf::from("-"), PathBuf::from);
        if let Some(extra) = operands.next() {
            return Err(Error::Extra(extra));
        }
        Ok(Self::Sort(Options { path, workers }))
    }
}

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Request::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Sort(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    runtime.block_on(async move {
        tokio::task::spawn_blocking(move || {
            let mut errors = io::stderr().lock();
            match sort(&options.path, ui, &mut errors) {
                Ok(status) => status,
                Err(error) => {
                    let _ = ui.error(
                        &mut errors,
                        LABEL,
                        &format!("{}: {error}", options.path.display()),
                    );
                    1
                }
            }
        })
        .await
        .map_err(io::Error::other)
    })
}

fn sort(path: &Path, ui: Presentation, errors: &mut impl Write) -> io::Result<u8> {
    let mut builder = graph::Builder::default();
    let mut progress = ui.progress(LABEL, "bytes", true);
    let mut parse = |input: &mut dyn Read| {
        read_pairs(input, &mut builder, |bytes| {
            if let Some(progress) = &mut progress {
                progress.advance(bytes, errors);
            }
        })
    };
    if path == Path::new("-") {
        parse(&mut io::stdin().lock())?;
    } else {
        parse(&mut File::open(path)?)?;
    }
    let mut graph = builder.finish()?;
    if let Some(progress) = &mut progress {
        progress.finish(errors);
    }

    let stdout = io::stdout();
    let capacity = if stdout.is_terminal() { 0 } else { BUFFER_SIZE };
    let mut output = BufWriter::with_capacity(capacity, stdout.lock());
    let clean = graph.order(&mut output, |names| {
        let _ = ui.error(
            errors,
            LABEL,
            &format!("{}: input contains a loop:", path.display()),
        );
        for name in names {
            let _ = ui.error(errors, LABEL, &String::from_utf8_lossy(name));
        }
    })?;
    output.flush()?;
    Ok(u8::from(!clean))
}

fn read_pairs(
    input: &mut (impl Read + ?Sized),
    builder: &mut graph::Builder,
    mut advance: impl FnMut(u64),
) -> io::Result<()> {
    let mut buffer = vec![0; BUFFER_SIZE];
    let mut partial = Vec::new();
    loop {
        let length = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => length,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let bytes = &buffer[..length];
        let mut start = 0;
        for end in memchr::memchr3_iter(b' ', b'\t', b'\n', bytes) {
            if partial.is_empty() {
                if start != end {
                    builder.token(&bytes[start..end]);
                }
            } else {
                partial.extend_from_slice(&bytes[start..end]);
                builder.token(&partial);
                partial.clear();
            }
            start = end + 1;
        }
        partial.extend_from_slice(&bytes[start..]);
        advance(length as u64);
    }
    if !partial.is_empty() {
        builder.token(&partial);
    }
    Ok(())
}

#[cfg(test)]
#[test]
#[allow(clippy::unwrap_used)]
fn retries_before_processing_an_interrupted_input() {
    struct InterruptedInput(bool, &'static [u8]);

    impl Read for InterruptedInput {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if std::mem::replace(&mut self.0, false) {
                Err(io::ErrorKind::Interrupted.into())
            } else {
                self.1.read(bytes)
            }
        }
    }

    let mut builder = graph::Builder::default();
    let mut total = 0;
    read_pairs(
        &mut InterruptedInput(true, b"c d"),
        &mut builder,
        |length| total += length,
    )
    .unwrap();
    let mut output = Vec::new();
    assert!(builder
        .finish()
        .unwrap()
        .order(&mut output, |_| {})
        .unwrap());
    assert_eq!(output, b"c\nd\n");
    assert_eq!(total, 3);
}
