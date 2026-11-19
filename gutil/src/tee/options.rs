use std::ffi::OsString;
use std::path::PathBuf;

use crate::jobs;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ErrorMode {
    #[default]
    Default,
    Warn,
    WarnNoPipe,
    Exit,
    ExitNoPipe,
}

impl ErrorMode {
    fn parse(value: &[u8]) -> Result<Self, ParseError> {
        match value {
            b"warn" => Ok(Self::Warn),
            b"warn-nopipe" => Ok(Self::WarnNoPipe),
            b"exit" => Ok(Self::Exit),
            b"exit-nopipe" => Ok(Self::ExitNoPipe),
            _ => Err(ParseError::Mode),
        }
    }

    pub(super) fn exits(self) -> bool {
        matches!(self, Self::Exit | Self::ExitNoPipe)
    }

    pub(super) fn ignores_pipe(self) -> bool {
        matches!(self, Self::WarnNoPipe | Self::ExitNoPipe)
    }
}

pub(super) struct Options {
    pub(super) append: bool,
    pub(super) ignore_interrupts: bool,
    pub(super) mode: ErrorMode,
    pub(super) paths: Vec<PathBuf>,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Copy(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum ParseError {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("--output-error expects warn, warn-nopipe, exit, or exit-nopipe")]
    Mode,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

impl Options {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, ParseError> {
        let mut options = Self {
            append: false,
            ignore_interrupts: false,
            mode: ErrorMode::Default,
            paths: Vec::new(),
            workers,
        };
        let mut flags = true;
        while let Some(arg) = args.next() {
            let bytes = arg.as_encoded_bytes();
            if !flags || arg == "-" || !bytes.starts_with(b"-") {
                options.paths.push(PathBuf::from(arg));
                continue;
            }
            if let Some(count) = jobs::option(bytes, &mut args) {
                options.workers = count?;
                continue;
            }
            match bytes {
                b"--" => flags = false,
                b"--help" => return Ok(Request::Help),
                b"--version" => return Ok(Request::Version),
                b"--append" => options.append = true,
                b"--ignore-interrupts" => options.ignore_interrupts = true,
                b"--output-error" => options.mode = ErrorMode::WarnNoPipe,
                _ if bytes.starts_with(b"--output-error=") => {
                    options.mode = ErrorMode::parse(&bytes[15..])?;
                }
                [b'-', short @ ..] if !short.starts_with(b"-") => {
                    for (index, flag) in short.iter().enumerate() {
                        match flag {
                            b'a' => options.append = true,
                            b'i' => options.ignore_interrupts = true,
                            b'p' => options.mode = ErrorMode::WarnNoPipe,
                            b'j' => {
                                options.workers =
                                    jobs::short_value(&short[index + 1..], &mut args)?;
                                break;
                            }
                            _ => return Err(ParseError::Option(arg)),
                        }
                    }
                }
                _ => return Err(ParseError::Option(arg)),
            }
        }
        Ok(Request::Copy(options))
    }
}
