use std::env;
use std::ffi::OsString;
use std::io::{self, Write};
use std::iter::Peekable;

use super::{posix, Command, Error, Info};

pub(super) fn echo(
    mut args: Peekable<impl Iterator<Item = OsString>>,
    output: &mut impl Write,
) -> Result<(), Error> {
    let mut newline = true;
    let posix = posix();
    let mut escapes = posix;
    let first = args.next();
    if !posix && args.peek().is_none() {
        if let Some(info) = first.as_deref().and_then(Info::parse) {
            return info.write(Command::Echo, output).map_err(Error::from);
        }
    }

    let parse_options = !posix || first.as_deref().is_some_and(|argument| argument == "-n");
    let mut args = first.into_iter().chain(args).peekable();
    if parse_options {
        while let Some(argument) = args.peek() {
            let Some(flags) = argument.as_encoded_bytes().strip_prefix(b"-") else {
                break;
            };
            if flags.is_empty() || !flags.iter().all(|flag| matches!(flag, b'n' | b'e' | b'E')) {
                break;
            }
            for flag in flags {
                match flag {
                    b'n' => newline = false,
                    b'e' => escapes = true,
                    b'E' => escapes = posix,
                    _ => unreachable!(),
                }
            }
            args.next();
        }
    }
    for (index, argument) in args.enumerate() {
        if index != 0 {
            output.write_all(b" ")?;
        }
        if escapes {
            if !escaped(argument.as_encoded_bytes(), output)? {
                return Ok(());
            }
        } else {
            output.write_all(argument.as_encoded_bytes())?;
        }
    }
    if newline {
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn escaped(mut source: &[u8], output: &mut impl Write) -> io::Result<bool> {
    while let Some(position) = memchr::memchr(b'\\', source) {
        output.write_all(&source[..position])?;
        let rest = &source[position + 1..];
        let Some((&escape, following)) = rest.split_first() else {
            output.write_all(b"\\")?;
            return Ok(true);
        };
        source = following;
        let byte = match escape {
            b'c' => return Ok(false),
            b'a' => 7,
            b'b' => 8,
            b'e' => 27,
            b'f' => 12,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 11,
            b'\\' => b'\\',
            b'0' => parse_digits(&mut source, 8, 3).unwrap_or(0),
            b'1'..=b'7' => {
                source = rest;
                parse_digits(&mut source, 8, 3).unwrap_or(0)
            }
            b'x' => {
                if let Some(byte) = parse_digits(&mut source, 16, 2) {
                    byte
                } else {
                    output.write_all(b"\\x")?;
                    continue;
                }
            }
            _ => {
                output.write_all(&[b'\\', escape])?;
                continue;
            }
        };
        output.write_all(&[byte])?;
    }
    output.write_all(source)?;
    Ok(true)
}

fn parse_digits(source: &mut &[u8], radix: u8, maximum: usize) -> Option<u8> {
    let mut value = None;
    for _ in 0..maximum {
        let Some((&byte, remainder)) = source.split_first() else {
            break;
        };
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        value = Some(value.unwrap_or(0u8).wrapping_mul(radix).wrapping_add(digit));
        *source = remainder;
    }
    value
}

pub(super) fn yes(
    args: impl Iterator<Item = OsString>,
    output: &mut impl Write,
) -> Result<(), Error> {
    let mut flags = true;
    let mut line = Vec::new();
    let mut first = true;
    let posix = posix();
    for arg in args {
        if flags {
            if let Some(info) = Info::parse(&arg) {
                return info.write(Command::Yes, output).map_err(Error::from);
            }
            if arg == "--" {
                flags = false;
                continue;
            }
            if arg.as_encoded_bytes().starts_with(b"-") && arg != "-" {
                return Err(Error::Option(arg));
            }
            flags = !posix;
        }
        if !first {
            line.push(b' ');
        }
        first = false;
        line.extend_from_slice(arg.as_encoded_bytes());
    }
    if first {
        line.push(b'y');
    }
    line.push(b'\n');
    repeat(&line, output).map_err(Error::from)
}

fn repeat(line: &[u8], output: &mut impl Write) -> io::Result<()> {
    const BLOCK: usize = 64 * 1024;
    let count = (BLOCK / line.len()).max(1);
    if count == 1 {
        loop {
            output.write_all(line)?;
        }
    }
    let block = line.repeat(count);
    loop {
        output.write_all(&block)?;
    }
}

pub(super) fn printenv(
    mut args: Peekable<impl Iterator<Item = OsString>>,
    output: &mut impl Write,
) -> Result<u8, Error> {
    let mut delimiter = b'\n';
    while let Some(argument) = args.peek() {
        if let Some(info) = Info::parse(argument) {
            info.write(Command::Printenv, output)?;
            return Ok(0);
        }
        let bytes = argument.as_encoded_bytes();
        match bytes {
            b"--" => {
                args.next();
                break;
            }
            b"--null" => delimiter = 0,
            [b'-', flags @ ..] if !flags.is_empty() => {
                if flags.iter().all(|&flag| flag == b'0') {
                    delimiter = 0;
                } else {
                    return Err(Error::Option(args.next().unwrap_or_default()));
                }
            }
            _ => break,
        }
        args.next();
    }
    let mut status = 0;
    if args.peek().is_none() {
        for (name, value) in env::vars_os() {
            output.write_all(name.as_encoded_bytes())?;
            output.write_all(b"=")?;
            output.write_all(value.as_encoded_bytes())?;
            output.write_all(&[delimiter])?;
        }
    } else {
        for name in args {
            if let Some(value) = env::var_os(name) {
                output.write_all(value.as_encoded_bytes())?;
                output.write_all(&[delimiter])?;
            } else {
                status = 1;
            }
        }
    }
    Ok(status)
}
