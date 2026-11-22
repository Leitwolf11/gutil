use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::jobs;
use crate::mkdir::mode::{InvalidMode, Mode};
use crate::presentation::Presentation;

#[cfg(unix)]
mod unix;

const LABEL: &str = "gutil mkfifo";
const HELP: &str = "Usage: gutil mkfifo [OPTIONS] NAME...\n\nCreate named pipes (FIFOs) on Unix.\n\n  -m, --mode MODE      Set octal or symbolic permissions\n  -Z, --context[=CTX]  Request a security context\n  -j, --jobs, --threads N  Limit runtime workers\n      --help           Print help\n      --version        Print version\n";

enum Context {
    Inherited,
    Default,
    Explicit(String),
}

struct Options {
    paths: Vec<PathBuf>,
    mode: Option<Mode>,
    context: Context,
    workers: jobs::Count,
}

enum Request {
    Help,
    Version,
    Create(Options),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("missing FIFO operand")]
    MissingOperand,
    #[error("--mode requires a permission mode")]
    MissingMode,
    #[error(transparent)]
    Mode(#[from] InvalidMode),
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

impl Request {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Self, ParseError> {
        let mut paths = Vec::new();
        let mut mode = None;
        let mut context = Context::Inherited;
        let mut workers = workers;
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(argument) = args.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
                paths.push(PathBuf::from(argument));
                flags &= !posix;
                continue;
            }
            if let Some(count) = jobs::option(bytes, &mut args) {
                workers = count?;
                continue;
            }
            match bytes {
                b"--" => flags = false,
                [b'-', b'-', long @ ..] => {
                    let (name, value) = long
                        .iter()
                        .position(|&byte| byte == b'=')
                        .map_or((long, None), |index| {
                            (&long[..index], Some(&long[index + 1..]))
                        });
                    let abbreviated = |option: &[u8]| !name.is_empty() && option.starts_with(name);
                    match name {
                        _ if abbreviated(b"help") && value.is_none() => return Ok(Self::Help),
                        _ if abbreviated(b"version") && value.is_none() => {
                            return Ok(Self::Version)
                        }
                        _ if abbreviated(b"mode") => {
                            mode = Some(option_value(value, &mut args)?);
                        }
                        _ if abbreviated(b"context") => {
                            context = value.map_or(Context::Default, |value| {
                                Context::Explicit(String::from_utf8_lossy(value).into_owned())
                            });
                        }
                        _ => return Err(ParseError::Option(argument)),
                    }
                }
                [b'-', short @ ..] => {
                    for (index, flag) in short.iter().enumerate() {
                        let rest = &short[index + 1..];
                        match flag {
                            b'Z' => context = Context::Default,
                            b'm' => {
                                mode = Some(option_value(
                                    (!rest.is_empty()).then_some(rest),
                                    &mut args,
                                )?);
                                break;
                            }
                            b'j' => {
                                workers = jobs::short_value(rest, &mut args)?;
                                break;
                            }
                            _ => return Err(ParseError::Option(argument)),
                        }
                    }
                }
                _ => return Err(ParseError::Option(argument)),
            }
        }
        if paths.is_empty() {
            return Err(ParseError::MissingOperand);
        }
        let mode = mode
            .as_deref()
            .map(|bytes| Mode::parse(std::str::from_utf8(bytes).map_err(|_| InvalidMode)?))
            .transpose()?;
        Ok(Self::Create(Options {
            paths,
            mode,
            context,
            workers,
        }))
    }
}

fn option_value(
    attached: Option<&[u8]>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<Vec<u8>, ParseError> {
    attached.map_or_else(
        || {
            args.next()
                .map(OsString::into_encoded_bytes)
                .ok_or(ParseError::MissingMode)
        },
        |bytes| Ok(bytes.to_vec()),
    )
}

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let request = match Request::parse(args, workers) {
        Ok(request) => request,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            return Ok(1);
        }
    };
    match request {
        Request::Help => io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Request::Version => {
            writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        Request::Create(options) => create(options, ui),
    }
}

fn create(options: Options, ui: Presentation) -> io::Result<u8> {
    let mut errors = io::stderr();
    if !matches!(options.context, Context::Inherited) {
        if crate::mkdir::active_labels()? {
            let _ = ui.error(
                &mut errors,
                LABEL,
                "SELinux/SMACK context assignment is unavailable",
            );
            return Ok(1);
        }
        if let Context::Explicit(context) = &options.context {
            let _ = ui.warning(
                &mut errors,
                LABEL,
                &format!("security labeling is unavailable; ignoring context {context:?}"),
            );
        }
    }
    #[cfg(unix)]
    match unix::prepare(options.mode.as_ref()) {
        Ok(permissions) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(options.workers.get())
                .build()?;
            runtime.block_on(async move {
                tokio::task::spawn_blocking(move || {
                    unix::create_all(&options.paths, permissions, ui)
                })
                .await
                .map_err(io::Error::other)
            })
        }
        Err(error) => {
            let _ = ui.error(&mut errors, LABEL, &error.to_string());
            Ok(1)
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (options.paths, options.mode, options.workers);
        let _ = ui.error(
            &mut errors,
            LABEL,
            "POSIX FIFO creation is unavailable on this platform",
        );
        Ok(1)
    }
}
