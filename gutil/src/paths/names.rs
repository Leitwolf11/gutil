use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};

use super::{encoded_name, long_value, output_path, Command, Operands, ParseError};
use crate::jobs;

#[derive(Default)]
pub(super) struct Options {
    operands: Operands,
    suffix: OsString,
    multiple: bool,
    zero: bool,
}

impl Options {
    pub(super) fn option(
        &mut self,
        command: Command,
        arg: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
        workers: &mut jobs::Count,
    ) -> Result<(), ParseError> {
        let bytes = arg.as_encoded_bytes();
        match bytes {
            b"--zero" => self.zero = true,
            b"--multiple" if command == Command::Basename => self.multiple = true,
            _ if command == Command::Basename
                && (bytes == b"--suffix" || bytes.starts_with(b"--suffix=")) =>
            {
                self.suffix = long_value(arg, "--suffix", args)?;
                self.multiple = true;
            }
            [b'-', flags @ ..] if !flags.starts_with(b"-") => {
                for (index, flag) in flags.iter().enumerate() {
                    match flag {
                        b'a' if command == Command::Basename => self.multiple = true,
                        b'z' => self.zero = true,
                        b's' if command == Command::Basename => {
                            self.multiple = true;
                            self.suffix = if index + 1 == flags.len() {
                                args.next().ok_or(ParseError::MissingValue("-s"))?
                            } else {
                                encoded_name(&bytes[index + 2..]).into_owned()
                            };
                            break;
                        }
                        b'j' => {
                            *workers = jobs::short_value(&flags[index + 1..], args)?;
                            break;
                        }
                        _ => return Err(ParseError::unknown(arg)),
                    }
                }
            }
            _ => return Err(ParseError::unknown(arg)),
        }
        Ok(())
    }

    pub(super) fn finish(
        mut self,
        command: Command,
        mut operands: Operands,
    ) -> Result<Self, ParseError> {
        if command == Command::Basename && !self.multiple {
            if operands.len() > 2 {
                return Err(ParseError::ExtraOperand(operands.swap_remove(2)));
            }
            if operands.len() == 2 {
                self.suffix = operands.pop().unwrap_or_default();
            }
        }
        self.operands = operands;
        Ok(self)
    }
}

pub(super) fn write(command: Command, opts: &Options, output: &mut impl Write) -> io::Result<u8> {
    let delimiter = Some(if opts.zero { 0 } else { b'\n' });
    for operand in &opts.operands {
        let path = if command == Command::Basename {
            basename(operand, &opts.suffix)
        } else {
            dirname(operand)
        };
        output_path(output, &path, delimiter)?;
    }
    Ok(0)
}

const fn separator(byte: u8) -> bool {
    byte == b'/' || (cfg!(windows) && byte == b'\\')
}

#[cfg(windows)]
fn prefix_length(name: &OsStr) -> usize {
    match std::path::Path::new(name).components().next() {
        Some(std::path::Component::Prefix(prefix)) => prefix.as_os_str().as_encoded_bytes().len(),
        _ => 0,
    }
}

#[cfg(not(windows))]
const fn prefix_length(_name: &OsStr) -> usize {
    0
}

fn last_component(name: &OsStr) -> (usize, usize, usize) {
    let prefix = prefix_length(name);
    let path = &name.as_encoded_bytes()[prefix..];
    let end = path
        .iter()
        .rposition(|&byte| !separator(byte))
        .map_or(0, |i| i + 1);
    let start = path[..end]
        .iter()
        .rposition(|&byte| separator(byte))
        .map_or(0, |i| i + 1);
    (prefix, prefix + start, prefix + end)
}

fn basename<'a>(name: &'a OsStr, suffix: &OsStr) -> Cow<'a, OsStr> {
    let (prefix, start, end) = last_component(name);
    if name.is_empty() {
        return Cow::Borrowed(name);
    }
    if prefix == end {
        let bytes = name.as_encoded_bytes();
        return encoded_name(&bytes[..(end + 1).min(bytes.len())]);
    }
    let tail = &name.as_encoded_bytes()[start..end];
    let suffix = suffix.as_encoded_bytes();
    let end = if suffix.len() < tail.len() && tail.ends_with(suffix) {
        end - suffix.len()
    } else {
        end
    };
    encoded_name(&name.as_encoded_bytes()[start..end])
}

fn dirname(name: &OsStr) -> Cow<'_, OsStr> {
    let (prefix, start, end) = last_component(name);
    let bytes = name.as_encoded_bytes();
    if start == 0 && (end != 0 || bytes.is_empty()) {
        return Cow::Borrowed(OsStr::new("."));
    }
    let parent = bytes[prefix..start]
        .iter()
        .rposition(|&byte| !separator(byte))
        .map_or_else(
            || prefix + usize::from(bytes.get(prefix).is_some_and(|&byte| separator(byte))),
            |index| prefix + index + 1,
        );
    encoded_name(&bytes[..parent])
}
