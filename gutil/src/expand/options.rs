use std::ffi::OsString;
use std::path::PathBuf;

use crate::jobs;

use super::stops::{self, Stops};
use super::Command;

pub(super) struct Options {
    pub(super) stops: Stops,
    pub(super) initial: bool,
    pub(super) paths: Vec<PathBuf>,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Convert(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("--tabs requires a tab size or a list of positions")]
    MissingTabs,
    #[error(transparent)]
    Tabs(#[from] stops::Invalid),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

impl Request {
    pub(super) fn parse(
        command: Command,
        mut arguments: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Self, Error> {
        let mut workers = workers;
        let unexpand = command == Command::Unexpand;
        let mut first_only = false;
        let mut all = !unexpand;
        let mut legacy = Vec::new();
        let mut stops = stops::Builder::default();
        let mut paths = Vec::new();
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(argument) = arguments.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
                paths.push(PathBuf::from(argument));
                flags &= !posix;
                continue;
            }
            if bytes == b"--" {
                flags = false;
            } else if let Some(count) = jobs::option(bytes, &mut arguments) {
                workers = count?;
            } else if let Some(long) = bytes.strip_prefix(b"--") {
                let (name, value) = long
                    .iter()
                    .position(|&byte| byte == b'=')
                    .map_or((long, None), |index| {
                        (&long[..index], Some(&long[index + 1..]))
                    });
                let matches = |option: &[u8]| !name.is_empty() && option.starts_with(name);
                if matches(b"tabs") {
                    tabs(&mut stops, value, &mut arguments)?;
                    all = true;
                } else if value.is_some() {
                    return Err(Error::Option(argument));
                } else if (!unexpand && matches(b"initial")) || (unexpand && matches(b"first-only"))
                {
                    first_only = true;
                } else if unexpand && matches(b"all") {
                    all = true;
                } else if matches(b"help") {
                    return Ok(Self::Help);
                } else if matches(b"version") {
                    return Ok(Self::Version);
                } else {
                    return Err(Error::Option(argument));
                }
            } else {
                for (index, &flag) in bytes[1..].iter().enumerate() {
                    let remainder = &bytes[index + 2..];
                    match flag {
                        b'i' if !unexpand => first_only = true,
                        b'a' if unexpand => all = true,
                        b't' => {
                            all = true;
                            tabs(
                                &mut stops,
                                (!remainder.is_empty()).then_some(remainder),
                                &mut arguments,
                            )?;
                            break;
                        }
                        b',' if unexpand => {
                            stops.add(&legacy)?;
                            legacy.clear();
                        }
                        b'0'..=b'9' if unexpand => legacy.push(flag),
                        b'0'..=b'9' => {
                            stops.add(&bytes[index + 1..])?;
                            break;
                        }
                        b'j' => {
                            workers = jobs::short_value(remainder, &mut arguments)?;
                            break;
                        }
                        _ => return Err(Error::Option(argument)),
                    }
                }
            }
        }
        if paths.is_empty() {
            paths.push(PathBuf::from("-"));
        }
        stops.add(&legacy)?;
        Ok(Self::Convert(Options {
            stops: stops.finish()?,
            initial: first_only || !all,
            paths,
            workers,
        }))
    }
}

fn tabs(
    stops: &mut stops::Builder,
    value: Option<&[u8]>,
    arguments: &mut impl Iterator<Item = OsString>,
) -> Result<(), Error> {
    if let Some(list) = value {
        stops.add(list)?;
    } else {
        let list = arguments.next().ok_or(Error::MissingTabs)?;
        stops.add(list.as_encoded_bytes())?;
    }
    Ok(())
}
