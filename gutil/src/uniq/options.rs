use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::jobs;

#[derive(Clone, Copy)]
pub(super) enum Separator {
    None,
    Between,
    Before,
    After,
    Both,
}

#[derive(Clone, Copy)]
pub(super) enum Case {
    Exact,
    IgnoreAscii,
}

pub(super) struct Options {
    pub(super) input: PathBuf,
    pub(super) output: PathBuf,
    pub(super) count: bool,
    pub(super) repeated_only: bool,
    pub(super) unique_only: bool,
    pub(super) repeated: Option<Separator>,
    pub(super) group: Option<Separator>,
    pub(super) case: Case,
    pub(super) fields: usize,
    pub(super) skip: usize,
    pub(super) width: usize,
    pub(super) end: u8,
    pub(super) workers: jobs::Count,
}

pub(super) enum Request {
    Help,
    Version,
    Run(Options),
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Invalid {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("extra operand {0:?}")]
    Operand(OsString),
    #[error("a numeric option requires a value")]
    MissingNumber,
    #[error("expected a nonnegative number")]
    Number,
    #[error("invalid group delimiter method")]
    Method,
    #[error("--group is mutually exclusive with -c/-d/-D/-u")]
    Group,
    #[error("printing every duplicate and a repeat count cannot be combined")]
    Count,
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

const LONG: &[&str] = &[
    "all-repeated",
    "check-chars",
    "count",
    "group",
    "help",
    "ignore-case",
    "repeated",
    "skip-chars",
    "skip-fields",
    "unique",
    "version",
    "zero-terminated",
];

impl Options {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, Invalid> {
        let mut options = Self {
            input: PathBuf::new(),
            output: PathBuf::new(),
            count: false,
            repeated_only: false,
            unique_only: false,
            repeated: None,
            group: None,
            case: Case::Exact,
            fields: 0,
            skip: 0,
            width: usize::MAX,
            end: b'\n',
            workers,
        };
        let mut operands = Vec::with_capacity(2);
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(argument) = args.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || argument == "-" || !bytes.starts_with(b"-") {
                if operands.len() == 2 {
                    return Err(Invalid::Operand(argument));
                }
                operands.push(PathBuf::from(argument));
                flags &= !posix;
            } else if bytes == b"--" {
                flags = false;
            } else if let Some(count) = jobs::option(bytes, &mut args) {
                options.workers = count?;
            } else if let Some(long) = bytes.strip_prefix(b"--") {
                if let Some(info) = options.long(&argument, long, &mut args)? {
                    return Ok(info);
                }
            } else {
                options.short(&argument, &bytes[1..], &mut args)?;
            }
        }
        if options.group.is_some()
            && (options.count
                || options.repeated_only
                || options.unique_only
                || options.repeated.is_some())
        {
            return Err(Invalid::Group);
        }
        if options.count && options.repeated.is_some() {
            return Err(Invalid::Count);
        }
        let mut operands = operands.into_iter();
        options.input = operands.next().unwrap_or_else(|| PathBuf::from("-"));
        options.output = operands.next().unwrap_or_else(|| PathBuf::from("-"));
        Ok(Request::Run(options))
    }

    fn long(
        &mut self,
        argument: &OsStr,
        token: &[u8],
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<Option<Request>, Invalid> {
        let equals = token.iter().position(|&byte| byte == b'=');
        let (prefix, value) = equals.map_or((token, None), |index| {
            (&token[..index], Some(&token[index + 1..]))
        });
        let invalid = || Invalid::Option(argument.to_os_string());
        let mut names = LONG
            .iter()
            .copied()
            .filter(|name| !prefix.is_empty() && name.as_bytes().starts_with(prefix));
        let name = names.next().ok_or_else(invalid)?;
        if name.as_bytes() != prefix && names.next().is_some() {
            return Err(invalid());
        }
        match (name, value) {
            ("all-repeated", method) => self.repeated = Some(separator(method, false)?),
            ("group", method) => self.group = Some(separator(method, true)?),
            ("skip-fields", value) => self.fields = number(value, args)?,
            ("skip-chars", value) => self.skip = number(value, args)?,
            ("check-chars", value) => self.width = number(value, args)?,
            ("count", None) => self.count = true,
            ("repeated", None) => self.repeated_only = true,
            ("unique", None) => self.unique_only = true,
            ("ignore-case", None) => self.case = Case::IgnoreAscii,
            ("zero-terminated", None) => self.end = 0,
            ("help", None) => return Ok(Some(Request::Help)),
            ("version", None) => return Ok(Some(Request::Version)),
            _ => return Err(invalid()),
        }
        Ok(None)
    }

    fn short(
        &mut self,
        argument: &OsStr,
        flags: &[u8],
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Invalid> {
        for (index, &flag) in flags.iter().enumerate() {
            let rest = &flags[index + 1..];
            let value = (!rest.is_empty()).then_some(rest);
            match flag {
                b'c' => self.count = true,
                b'd' => self.repeated_only = true,
                b'D' => self.repeated = Some(Separator::None),
                b'u' => self.unique_only = true,
                b'i' => self.case = Case::IgnoreAscii,
                b'z' => self.end = 0,
                b'f' | b's' | b'w' => {
                    let count = number(value, args)?;
                    match flag {
                        b'f' => self.fields = count,
                        b's' => self.skip = count,
                        _ => self.width = count,
                    }
                    break;
                }
                b'0'..=b'9' if flags[index..].iter().all(u8::is_ascii_digit) => {
                    self.fields = number(Some(&flags[index..]), args)?;
                    break;
                }
                b'j' => {
                    self.workers = jobs::short_value(rest, args)?;
                    break;
                }
                _ => return Err(Invalid::Option(argument.to_os_string())),
            }
        }
        Ok(())
    }
}

fn number(
    value: Option<&[u8]>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<usize, Invalid> {
    let separate;
    let bytes = if let Some(value) = value {
        value
    } else {
        separate = args.next().ok_or(Invalid::MissingNumber)?;
        separate.as_encoded_bytes()
    };
    let text = std::str::from_utf8(bytes).map_err(|_| Invalid::Number)?;
    let digits = text
        .trim_start()
        .strip_prefix('+')
        .unwrap_or(text.trim_start());
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Invalid::Number);
    }
    Ok(digits.parse().unwrap_or(usize::MAX))
}

fn separator(method: Option<&[u8]>, group: bool) -> Result<Separator, Invalid> {
    match method {
        None if group => Ok(Separator::Between),
        None => Ok(Separator::None),
        Some(b"none") if !group => Ok(Separator::None),
        Some(b"separate") => Ok(Separator::Between),
        Some(b"prepend") => Ok(Separator::Before),
        Some(b"append") if group => Ok(Separator::After),
        Some(b"both") if group => Ok(Separator::Both),
        _ => Err(Invalid::Method),
    }
}
