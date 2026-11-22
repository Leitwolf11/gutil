use std::ffi::OsString;
use std::path::PathBuf;

use crate::jobs;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Count {
    Columns,
    Bytes,
    Characters,
}

pub(super) struct Options {
    pub(super) count: Count,
    pub(super) width: usize,
    pub(super) spaces: bool,
    pub(super) paths: Vec<PathBuf>,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Fold(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("--width requires a number")]
    MissingWidth,
    #[error("invalid number of columns")]
    Width,
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

impl Options {
    pub(super) fn parse(
        mut arguments: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, Error> {
        let mut options = Self {
            count: Count::Columns,
            width: 80,
            spaces: false,
            paths: Vec::new(),
            workers,
        };
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(argument) = arguments.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
                options.paths.push(PathBuf::from(argument));
                flags &= !posix;
                continue;
            }
            if bytes == b"--" {
                flags = false;
            } else if let Some(count) = jobs::option(bytes, &mut arguments) {
                options.workers = count?;
            } else if let Some(long) = bytes.strip_prefix(b"--") {
                if let Some(request) = options.long(long, &argument, &mut arguments)? {
                    return Ok(request);
                }
            } else {
                options.short(&bytes[1..], &argument, &mut arguments)?;
            }
        }
        if options.paths.is_empty() {
            options.paths.push(PathBuf::from("-"));
        }
        Ok(Request::Fold(options))
    }

    fn long(
        &mut self,
        token: &[u8],
        argument: &OsString,
        arguments: &mut impl Iterator<Item = OsString>,
    ) -> Result<Option<Request>, Error> {
        let (name, value) = token
            .iter()
            .position(|&byte| byte == b'=')
            .map_or((token, None), |index| {
                (&token[..index], Some(&token[index + 1..]))
            });
        let matches = |option: &[u8]| !name.is_empty() && option.starts_with(name);
        if matches(b"width") {
            self.width = width(value, arguments)?;
        } else if value.is_some() {
            return Err(Error::Option(argument.clone()));
        } else if matches(b"bytes") {
            self.count = Count::Bytes;
        } else if matches(b"characters") {
            self.count = Count::Characters;
        } else if matches(b"spaces") {
            self.spaces = true;
        } else if matches(b"help") {
            return Ok(Some(Request::Help));
        } else if matches(b"version") {
            return Ok(Some(Request::Version));
        } else {
            return Err(Error::Option(argument.clone()));
        }
        Ok(None)
    }

    fn short(
        &mut self,
        token: &[u8],
        argument: &OsString,
        arguments: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Error> {
        for (index, &flag) in token.iter().enumerate() {
            let rest = &token[index + 1..];
            match flag {
                b'b' => self.count = Count::Bytes,
                b'c' => self.count = Count::Characters,
                b's' => self.spaces = true,
                b'w' => {
                    self.width = width((!rest.is_empty()).then_some(rest), arguments)?;
                    break;
                }
                b'0'..=b'9' => {
                    self.width = width(Some(&token[index..]), arguments)?;
                    break;
                }
                b'j' => {
                    self.workers = jobs::short_value(rest, arguments)?;
                    break;
                }
                _ => return Err(Error::Option(argument.clone())),
            }
        }
        Ok(())
    }
}

fn width(
    attached: Option<&[u8]>,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<usize, Error> {
    let separate;
    let bytes = if let Some(bytes) = attached {
        bytes
    } else {
        separate = arguments.next().ok_or(Error::MissingWidth)?;
        separate.as_encoded_bytes()
    };
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Width)?;
    let digits = text
        .trim_start()
        .strip_prefix('+')
        .unwrap_or(text.trim_start());
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::Width);
    }
    digits
        .parse::<usize>()
        .ok()
        .filter(|width| (1..=usize::MAX - 9).contains(width))
        .ok_or(Error::Width)
}
