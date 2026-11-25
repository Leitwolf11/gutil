use std::ffi::OsString;
use std::ops::Range;
use std::path::PathBuf;

use memchr::memmem::Finder;

use crate::jobs;

use super::positions::{self, InvalidList};

pub(super) struct Options {
    pub(super) mode: Mode,
    pub(super) ranges: Vec<Range<usize>>,
    pub(super) separator: Option<Vec<u8>>,
    pub(super) terminator: u8,
    pub(super) paths: Vec<PathBuf>,
    pub(super) workers: jobs::Count,
    pub(super) utf8: bool,
}

pub(super) enum Mode {
    Bytes,
    Characters,
    WholeUnits,
    Fields { splitter: Splitter, suppress: bool },
}

pub(super) enum Splitter {
    Byte(u8),
    Text(Box<Finder<'static>>),
    Blanks { trim: bool },
}

pub(super) enum Request {
    Cut(Options),
    Help,
    Version,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flag {
    Bytes,
    Characters,
    Fields,
    BlankFields,
    Delimiter,
    OutputDelimiter,
    NoPartial,
    OnlyDelimited,
    Whitespace,
    Zero,
    Complement,
    Help,
    Version,
}

const LONG: &[(&[u8], Flag)] = &[
    (b"bytes", Flag::Bytes),
    (b"characters", Flag::Characters),
    (b"fields", Flag::Fields),
    (b"delimiter", Flag::Delimiter),
    (b"output-delimiter", Flag::OutputDelimiter),
    (b"no-partial", Flag::NoPartial),
    (b"only-delimited", Flag::OnlyDelimited),
    (b"whitespace-delimited", Flag::Whitespace),
    (b"zero-terminated", Flag::Zero),
    (b"complement", Flag::Complement),
    (b"help", Flag::Help),
    (b"version", Flag::Version),
];

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized or ambiguous option {0:?}")]
    Option(OsString),
    #[error("option {0:?} requires a value")]
    MissingValue(OsString),
    #[error("specify one list of bytes, characters, or fields")]
    MissingSelection,
    #[error("only one list may be specified")]
    RepeatedSelection,
    #[error("the input delimiter must be one character")]
    Delimiter,
    #[error("input delimiters and --only-delimited require field selection")]
    NeedsFields,
    #[error("-d and -w are mutually exclusive")]
    ConflictingDelimiters,
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
    #[error(transparent)]
    List(#[from] InvalidList),
}

#[derive(Default)]
struct Parser {
    selection: Option<(Flag, Vec<u8>)>,
    delim: Option<Vec<u8>>,
    output: Option<Vec<u8>>,
    whitespace: Option<bool>,
    suppress: bool,
    no_partial: bool,
    terminator: u8,
    complement: bool,
}

impl Request {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
        utf8: bool,
    ) -> Result<Self, Error> {
        let mut parser = Parser {
            terminator: b'\n',
            ..Parser::default()
        };
        let mut paths = Vec::new();
        let mut flags = true;
        let posix = std::env::var_os("POSIXLY_CORRECT").is_some();
        while let Some(token) = args.next() {
            let bytes = token.as_encoded_bytes();
            if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
                paths.push(PathBuf::from(token));
                flags &= !posix;
            } else if bytes == b"--" {
                flags = false;
            } else if let Some(count) = jobs::option(bytes, &mut args) {
                workers = count?;
            } else if let Some(long) = bytes.strip_prefix(b"--") {
                let (name, value) = memchr::memchr(b'=', long).map_or((long, None), |index| {
                    (&long[..index], Some(&long[index + 1..]))
                });
                let mut matches = LONG
                    .iter()
                    .filter(|(key, _)| !name.is_empty() && key.starts_with(name));
                let flag = matches
                    .next()
                    .filter(|_| matches.next().is_none())
                    .map(|(_, flag)| *flag);
                let flag = flag.ok_or_else(|| Error::Option(token.clone()))?;
                if let Some(request) = parser.apply(flag, value, &token, &mut args)? {
                    return Ok(request);
                }
            } else {
                for (index, &byte) in bytes[1..].iter().enumerate() {
                    let rest = &bytes[index + 2..];
                    if byte == b'j' {
                        workers = jobs::short_value(rest, &mut args)?;
                        break;
                    }
                    let flag = match byte {
                        b'b' => Flag::Bytes,
                        b'c' => Flag::Characters,
                        b'f' => Flag::Fields,
                        b'F' => Flag::BlankFields,
                        b'd' => Flag::Delimiter,
                        b'O' => Flag::OutputDelimiter,
                        b'n' => Flag::NoPartial,
                        b's' => Flag::OnlyDelimited,
                        b'w' => Flag::Whitespace,
                        b'z' => Flag::Zero,
                        _ => return Err(Error::Option(token)),
                    };
                    if flag.requires_value() {
                        parser.apply(
                            flag,
                            (!rest.is_empty()).then_some(rest),
                            &token,
                            &mut args,
                        )?;
                        break;
                    }
                    parser.apply(flag, None, &token, &mut args)?;
                }
            }
        }
        if paths.is_empty() {
            paths.push(PathBuf::from("-"));
        }
        parser.finish(paths, workers, utf8).map(Self::Cut)
    }
}

impl Flag {
    fn requires_value(self) -> bool {
        matches!(
            self,
            Self::Bytes
                | Self::Characters
                | Self::Fields
                | Self::BlankFields
                | Self::Delimiter
                | Self::OutputDelimiter
        )
    }
}

impl Parser {
    fn apply(
        &mut self,
        flag: Flag,
        value: Option<&[u8]>,
        option: &OsString,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<Option<Request>, Error> {
        if value.is_some() && !flag.requires_value() && flag != Flag::Whitespace {
            return Err(Error::Option(option.clone()));
        }
        match flag {
            Flag::Bytes | Flag::Characters | Flag::Fields | Flag::BlankFields => {
                if self.selection.is_some() {
                    return Err(Error::RepeatedSelection);
                }
                self.selection = Some((flag, required_value(value, option, args)?));
            }
            Flag::Delimiter | Flag::OutputDelimiter => {
                let mut delimiter = required_value(value, option, args)?;
                if delimiter.is_empty() {
                    delimiter.push(0);
                }
                if flag == Flag::Delimiter {
                    self.delim = Some(delimiter);
                } else {
                    self.output = Some(delimiter);
                }
            }
            Flag::Whitespace => match value {
                None => self.whitespace = Some(false),
                Some(value) if !value.is_empty() && b"trimmed".starts_with(value) => {
                    self.whitespace = Some(true);
                }
                _ => return Err(Error::Option(option.clone())),
            },
            Flag::Help => return Ok(Some(Request::Help)),
            Flag::Version => return Ok(Some(Request::Version)),
            Flag::NoPartial => self.no_partial = true,
            Flag::OnlyDelimited => self.suppress = true,
            Flag::Zero => self.terminator = 0,
            Flag::Complement => self.complement = true,
        }
        Ok(None)
    }

    fn finish(
        self,
        paths: Vec<PathBuf>,
        workers: jobs::Count,
        utf8: bool,
    ) -> Result<Options, Error> {
        let (selection, list) = self.selection.ok_or(Error::MissingSelection)?;
        let ranges = positions::parse(&list, self.complement)?;
        let fields = matches!(selection, Flag::Fields | Flag::BlankFields);
        if !fields && (self.delim.is_some() || self.whitespace.is_some() || self.suppress) {
            return Err(Error::NeedsFields);
        }
        if self.delim.is_some() && self.whitespace.is_some() {
            return Err(Error::ConflictingDelimiters);
        }
        let mut separator = self.output;
        let mode = if fields {
            let blanks = self.whitespace.is_some()
                || (selection == Flag::BlankFields && self.delim.is_none());
            let delim = self.delim.unwrap_or_else(|| vec![b'\t']);
            let splitter = if blanks {
                Splitter::Blanks {
                    trim: self.whitespace.unwrap_or(false),
                }
            } else if delim.len() == 1 {
                Splitter::Byte(delim[0])
            } else if utf8
                && std::str::from_utf8(&delim).is_ok_and(|text| text.chars().count() == 1)
            {
                Splitter::Text(Box::new(Finder::new(&delim).into_owned()))
            } else {
                return Err(Error::Delimiter);
            };
            separator.get_or_insert_with(|| {
                if selection == Flag::BlankFields {
                    vec![b' ']
                } else {
                    delim
                }
            });
            Mode::Fields {
                splitter,
                suppress: self.suppress,
            }
        } else if !utf8 || (selection == Flag::Bytes && !self.no_partial) {
            Mode::Bytes
        } else if selection == Flag::Characters {
            Mode::Characters
        } else {
            Mode::WholeUnits
        };
        Ok(Options {
            mode,
            ranges,
            separator,
            paths,
            workers,
            utf8,
            terminator: self.terminator,
        })
    }
}

fn required_value(
    value: Option<&[u8]>,
    option: &OsString,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<Vec<u8>, Error> {
    value.map_or_else(
        || {
            args.next()
                .map(OsString::into_encoded_bytes)
                .ok_or_else(|| Error::MissingValue(option.clone()))
        },
        |bytes| Ok(bytes.to_vec()),
    )
}
