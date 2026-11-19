use std::ffi::OsString;
use std::path::PathBuf;

use super::mode::{InvalidMode, Mode};
use crate::jobs;

#[derive(Default)]
pub(super) enum Context {
    #[default]
    Inherited,
    Default,
    Explicit(String),
}

pub(super) struct Options {
    pub(super) paths: Vec<PathBuf>,
    pub(super) parents: bool,
    pub(super) verbose: bool,
    pub(super) mode: Option<Mode>,
    pub(super) context: Context,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Create(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("missing directory operand")]
    MissingOperand,
    #[error("--mode requires a permission mode")]
    MissingMode,
    #[error(transparent)]
    Mode(#[from] InvalidMode),
    #[error("invalid security context")]
    Context,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

impl Options {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, Error> {
        let mut options = Self {
            paths: Vec::new(),
            parents: false,
            verbose: false,
            mode: None,
            context: Context::Inherited,
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
                b"--parents" => options.parents = true,
                b"--verbose" => options.verbose = true,
                b"--mode" => options.mode = Some(read_mode(None, &mut args)?),
                b"--context" => options.context = Context::Default,
                _ if bytes.starts_with(b"--mode=") => {
                    options.mode = Some(read_mode(Some(&bytes[7..]), &mut args)?);
                }
                _ if bytes.starts_with(b"--context=") => {
                    options.context = Context::Explicit(
                        std::str::from_utf8(&bytes[10..])
                            .map_err(|_| Error::Context)?
                            .to_owned(),
                    );
                }
                [b'-', short @ ..] if !short.starts_with(b"-") => {
                    for (index, flag) in short.iter().enumerate() {
                        let rest = &short[index + 1..];
                        match flag {
                            b'p' => options.parents = true,
                            b'v' => options.verbose = true,
                            b'Z' => options.context = Context::Default,
                            b'm' => {
                                options.mode =
                                    Some(read_mode((!rest.is_empty()).then_some(rest), &mut args)?);
                                break;
                            }
                            b'j' => {
                                options.workers = jobs::short_value(rest, &mut args)?;
                                break;
                            }
                            _ => return Err(Error::Option(arg)),
                        }
                    }
                }
                _ => return Err(Error::Option(arg)),
            }
        }
        if options.paths.is_empty() {
            return Err(Error::MissingOperand);
        }
        Ok(Request::Create(options))
    }
}

fn read_mode(
    attached: Option<&[u8]>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<Mode, Error> {
    let separate;
    let bytes = if let Some(bytes) = attached {
        bytes
    } else {
        separate = args.next().ok_or(Error::MissingMode)?;
        separate.as_encoded_bytes()
    };
    Ok(Mode::parse(
        std::str::from_utf8(bytes).map_err(|_| InvalidMode)?,
    )?)
}
