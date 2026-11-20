use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use smallvec::SmallVec;

use crate::jobs;

type Delimiter = SmallVec<[u8; 4]>;

pub(super) struct Options {
    pub(super) paths: Vec<PathBuf>,
    pub(super) serial: bool,
    pub(super) terminator: u8,
    pub(super) delimiters: Vec<Delimiter>,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Run(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("missing delimiter list")]
    MissingDelimiters,
    #[error("delimiter list ends with an unescaped backslash")]
    TrailingBackslash,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

const LONG_OPTIONS: &[&str] = &["delimiters", "serial", "zero-terminated", "help", "version"];

impl Options {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, Error> {
        let mut options = Self {
            paths: Vec::new(),
            serial: false,
            terminator: b'\n',
            delimiters: vec![SmallVec::from_slice(b"\t")],
            workers,
        };
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(arg) = args.next() {
            let bytes = arg.as_encoded_bytes();
            if !flags || arg == "-" || !bytes.starts_with(b"-") {
                options.paths.push(PathBuf::from(arg));
                flags &= !posix;
                continue;
            }
            if bytes == b"--" {
                flags = false;
                continue;
            }
            if let Some(count) = jobs::option(bytes, &mut args) {
                options.workers = count?;
                continue;
            }
            if let Some(long) = bytes.strip_prefix(b"--") {
                if let Some(info) = options.long(&arg, long, &mut args)? {
                    return Ok(info);
                }
            } else {
                for (index, flag) in bytes[1..].iter().enumerate() {
                    let rest = &bytes[index + 2..];
                    match flag {
                        b's' => options.serial = true,
                        b'z' => options.terminator = 0,
                        b'd' => {
                            options
                                .set_delimiters((!rest.is_empty()).then_some(rest), &mut args)?;
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
        }
        if options.paths.is_empty() {
            options.paths.push(PathBuf::from("-"));
        }
        Ok(Request::Run(options))
    }

    fn long(
        &mut self,
        argument: &OsStr,
        token: &[u8],
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<Option<Request>, Error> {
        let split = token.iter().position(|&byte| byte == b'=');
        let (prefix, attached) = split.map_or((token, None), |index| {
            (&token[..index], Some(&token[index + 1..]))
        });
        let invalid = || Error::Option(argument.to_os_string());
        let name = LONG_OPTIONS
            .iter()
            .copied()
            .find(|name| !prefix.is_empty() && name.as_bytes().starts_with(prefix))
            .ok_or_else(invalid)?;
        match (name, attached) {
            ("delimiters", value) => self.set_delimiters(value, args)?,
            ("serial", None) => self.serial = true,
            ("zero-terminated", None) => self.terminator = 0,
            ("help", None) => return Ok(Some(Request::Help)),
            ("version", None) => return Ok(Some(Request::Version)),
            _ => return Err(invalid()),
        }
        Ok(None)
    }

    fn set_delimiters(
        &mut self,
        attached: Option<&[u8]>,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Error> {
        let separate;
        let list = if let Some(attached) = attached {
            attached
        } else {
            separate = args.next().ok_or(Error::MissingDelimiters)?;
            separate.as_encoded_bytes()
        };
        self.delimiters = parse_delimiters(list, utf8_locale())?;
        Ok(())
    }

    pub(super) fn delimiter(&self, index: usize) -> &[u8] {
        &self.delimiters[index % self.delimiters.len()]
    }
}

fn utf8_locale() -> bool {
    cfg!(windows)
        || ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .is_some_and(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("utf-8") || value.contains("utf8")
            })
}

fn parse_delimiters(mut list: &[u8], unicode: bool) -> Result<Vec<Delimiter>, Error> {
    let mut result = Vec::new();
    while let Some((&first, tail)) = list.split_first() {
        if first == b'\\' {
            let Some((&escaped, rest)) = tail.split_first() else {
                return Err(Error::TrailingBackslash);
            };
            let byte = match escaped {
                b'0' => {
                    result.push(Delimiter::new());
                    list = rest;
                    continue;
                }
                b'b' => b'\x08',
                b'f' => b'\x0c',
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => b'\x0b',
                other if !other.is_ascii() && unicode => {
                    let width = delimiter_width(tail);
                    result.push(Delimiter::from_slice(&tail[..width]));
                    list = &tail[width..];
                    continue;
                }
                other => other,
            };
            result.push(Delimiter::from_slice(&[byte]));
            list = rest;
        } else {
            let width = if unicode { delimiter_width(list) } else { 1 };
            result.push(Delimiter::from_slice(&list[..width]));
            list = &list[width..];
        }
    }
    if result.is_empty() {
        result.push(Delimiter::new());
    }
    Ok(result)
}

fn delimiter_width(list: &[u8]) -> usize {
    let width = match list[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 1,
    };
    if list
        .get(..width)
        .is_some_and(|bytes| std::str::from_utf8(bytes).is_ok())
    {
        width
    } else {
        1
    }
}
