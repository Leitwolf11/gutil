use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use super::{links, size, Command, Error, Removal, Request, Work};
use crate::jobs;

enum Value {
    Size,
    Reference,
    Directory,
    Suffix,
}

impl Value {
    const fn flag(&self) -> &'static str {
        match self {
            Self::Size => "--size",
            Self::Reference => "--reference",
            Self::Directory => "--target-directory",
            Self::Suffix => "--suffix",
        }
    }
}

struct Builder {
    command: Command,
    workers: jobs::Count,
    removal: Removal,
    truncate: size::Options,
    links: links::Options,
}

pub(super) fn parse(
    command: Command,
    mut args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
) -> Result<Request, Error> {
    let mut options = Builder {
        command,
        workers,
        removal: Removal {
            parents: false,
            verbose: false,
            ignore_nonempty: false,
        },
        truncate: size::Options {
            size: None,
            reference: None,
            no_create: false,
            io_blocks: false,
        },
        links: links::Options::default(),
    };
    let mut paths = Vec::new();
    let mut flags = true;
    while let Some(argument) = args.next() {
        let bytes = argument.as_encoded_bytes();
        if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
            paths.push(PathBuf::from(argument));
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
            [b'-', b'-', ..] => options.long(&argument, &mut args)?,
            _ => options.short(&argument, &mut args)?,
        }
    }
    options.finish(paths)
}

impl Builder {
    fn size(&mut self, input: OsString) -> Result<(), Error> {
        self.truncate.size =
            Some(size::Size::parse(input.as_encoded_bytes()).ok_or(Error::Size(input))?);
        Ok(())
    }

    fn long(
        &mut self,
        argument: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Error> {
        let bytes = argument.as_encoded_bytes();
        let split = bytes.iter().position(|byte| *byte == b'=');
        let flag = &bytes[..split.unwrap_or(bytes.len())];
        if split.is_none() && self.boolean(flag) {
            return Ok(());
        }
        let attached = split.map(|offset| native_suffix(argument, offset + 1));
        match (self.command, flag) {
            (Command::Truncate, b"--size") => self.size(value("--size", attached, args)?),
            (Command::Truncate, b"--reference") => {
                self.truncate.reference = Some(value("--reference", attached, args)?.into());
                Ok(())
            }
            (Command::Ln, b"--target-directory") => {
                self.links.target_directory =
                    Some(value("--target-directory", attached, args)?.into());
                Ok(())
            }
            (Command::Ln, b"--suffix") => {
                self.links.suffix = Some(value("--suffix", attached, args)?);
                self.enable_backup()?;
                Ok(())
            }
            (Command::Ln, b"--backup") => {
                self.links.backup = Some(backup(attached)?);
                Ok(())
            }
            _ => Err(Error::Option(argument.into())),
        }
    }

    fn boolean(&mut self, flag: &[u8]) -> bool {
        let short = match (self.command, flag) {
            (Command::Rmdir, b"--parents") => b'p',
            (Command::Rmdir | Command::Ln, b"--verbose") => b'v',
            (Command::Rmdir, b"--ignore-fail-on-non-empty") => {
                self.removal.ignore_nonempty = true;
                return true;
            }
            (Command::Truncate, b"--no-create") => b'c',
            (Command::Truncate, b"--io-blocks") => b'o',
            (Command::Ln, b"--symbolic") => b's',
            (Command::Ln, b"--force") => b'f',
            (Command::Ln, b"--interactive") => b'i',
            (Command::Ln, b"--no-dereference") => b'n',
            (Command::Ln, b"--no-target-directory") => b'T',
            (Command::Ln, b"--logical") => b'L',
            (Command::Ln, b"--physical") => b'P',
            (Command::Ln, b"--relative") => b'r',
            (Command::Ln, b"--directory") => b'd',
            _ => return false,
        };
        self.flag(short)
    }

    fn flag(&mut self, flag: u8) -> bool {
        match (self.command, flag) {
            (Command::Rmdir, b'p') => self.removal.parents = true,
            (Command::Rmdir, b'v') => self.removal.verbose = true,
            (Command::Truncate, b'c') => self.truncate.no_create = true,
            (Command::Truncate, b'o') => self.truncate.io_blocks = true,
            (Command::Ln, flag) => return self.links.flag(flag),
            _ => return false,
        }
        true
    }

    fn short(
        &mut self,
        argument: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Error> {
        let short = &argument.as_encoded_bytes()[1..];
        for (index, &flag) in short.iter().enumerate() {
            let rest = &short[index + 1..];
            if flag == b'j' {
                self.workers = jobs::short_value(rest, args)?;
                return Ok(());
            }
            if self.command == Command::Ln && flag == b'b' {
                self.links.backup = Some(backup(None)?);
                continue;
            }
            let name = match (self.command, flag) {
                (Command::Truncate, b's') => Value::Size,
                (Command::Truncate, b'r') => Value::Reference,
                (Command::Ln, b't') => Value::Directory,
                (Command::Ln, b'S') => Value::Suffix,
                _ if self.flag(flag) => continue,
                _ => return Err(Error::Option(argument.into())),
            };
            let attached = (!rest.is_empty()).then(|| native_suffix(argument, index + 2));
            let input = value(name.flag(), attached, args)?;
            match name {
                Value::Size => self.size(input)?,
                Value::Reference => self.truncate.reference = Some(input.into()),
                Value::Directory => self.links.target_directory = Some(input.into()),
                Value::Suffix => {
                    self.links.suffix = Some(input);
                    self.enable_backup()?;
                }
            }
            return Ok(());
        }
        Ok(())
    }

    fn enable_backup(&mut self) -> Result<(), Error> {
        if self.links.backup.is_none() {
            self.links.backup = Some(backup(None)?);
        }
        Ok(())
    }

    fn finish(self, mut paths: Vec<PathBuf>) -> Result<Request, Error> {
        if paths.is_empty() {
            return Err(Error::MissingOperand);
        }
        let work = match self.command {
            Command::Link => {
                if paths.len() < 2 {
                    return Err(Error::MissingOperand);
                }
                if paths.len() > 2 {
                    return Err(Error::ExtraOperand(paths.remove(2)));
                }
                let target = paths.pop().ok_or(Error::MissingOperand)?;
                let source = paths.pop().ok_or(Error::MissingOperand)?;
                Work::Link { source, target }
            }
            Command::Unlink => {
                if paths.len() > 1 {
                    return Err(Error::ExtraOperand(paths.remove(1)));
                }
                Work::Unlink(paths.pop().ok_or(Error::MissingOperand)?)
            }
            Command::Rmdir => Work::Rmdir(self.removal, paths),
            Command::Ln => Work::Ln(self.links, paths),
            Command::Truncate => {
                if self.truncate.size.is_none() && self.truncate.reference.is_none() {
                    return Err(Error::MissingSize);
                }
                if self.truncate.io_blocks && self.truncate.size.is_none() {
                    return Err(Error::MissingBlockSize);
                }
                if self.truncate.reference.is_some()
                    && self.truncate.size.is_some_and(|size| !size.relative())
                {
                    return Err(Error::AbsoluteReference);
                }
                Work::Truncate(self.truncate, paths)
            }
        };
        Ok(Request::Run(work, self.workers))
    }
}

fn backup(value: Option<OsString>) -> Result<links::Backup, Error> {
    let value = value
        .filter(|value| !value.is_empty())
        .or_else(|| std::env::var_os("VERSION_CONTROL"))
        .unwrap_or_else(|| "existing".into());
    links::Backup::parse(&value).ok_or(Error::Backup(value))
}

fn value(
    flag: &'static str,
    attached: Option<OsString>,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<OsString, Error> {
    attached
        .or_else(|| args.next())
        .ok_or(Error::MissingValue(flag))
}

#[cfg(unix)]
fn native_suffix(argument: &OsStr, offset: usize) -> OsString {
    use std::os::unix::ffi::OsStrExt;
    OsStr::from_bytes(&argument.as_bytes()[offset..]).to_os_string()
}

#[cfg(windows)]
fn native_suffix(argument: &OsStr, offset: usize) -> OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    OsString::from_wide(&argument.encode_wide().skip(offset).collect::<Vec<_>>())
}

#[cfg(not(any(unix, windows)))]
fn native_suffix(argument: &OsStr, offset: usize) -> OsString {
    argument.to_string_lossy()[offset..].into()
}
