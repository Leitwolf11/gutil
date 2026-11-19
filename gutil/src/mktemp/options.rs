use std::ffi::OsString;
use std::path::PathBuf;

use super::{
    native,
    template::{self, Template},
};
use crate::jobs;

#[derive(Clone, Copy)]
pub(super) enum Action {
    File,
    Directory,
    NameOnly,
}

pub(super) enum Location {
    AsWritten,
    Temporary(Option<PathBuf>),
    Legacy(Option<PathBuf>),
}

pub(super) struct Options {
    pub(super) template: Template,
    pub(super) location: Location,
    pub(super) action: Action,
    pub(super) quiet: bool,
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
    #[error("extra operand {0:?}")]
    Operand(OsString),
    #[error("{0} requires a value")]
    Missing(&'static str),
    #[error("the final template component must contain a run of at least three 'X' characters")]
    Template,
    #[error("an explicit --suffix requires a template ending in 'X'")]
    SuffixTemplate,
    #[error("a suffix must not contain a directory separator or NUL")]
    Suffix,
    #[cfg(windows)]
    #[error("temporary templates and suffixes must not name alternate data streams")]
    AlternateStream,
    #[error("--tmpdir requires a relative template")]
    Absolute,
    #[error("-t requires a single filename component without a drive prefix")]
    Legacy,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
}

enum Creation {
    File,
    Directory,
}

enum Root {
    AsWritten,
    Default,
    Explicit(PathBuf),
}

impl Root {
    fn directory(self) -> Option<PathBuf> {
        match self {
            Self::Explicit(path) => Some(path),
            Self::AsWritten | Self::Default => None,
        }
    }
}

struct Parser {
    template: Option<OsString>,
    suffix: Option<OsString>,
    kind: Creation,
    dry_run: bool,
    quiet: bool,
    legacy: bool,
    root: Root,
    workers: jobs::Count,
}

impl Options {
    pub(super) fn parse(
        mut args: impl Iterator<Item = OsString>,
        workers: jobs::Count,
    ) -> Result<Request, Error> {
        let mut parser = Parser {
            template: None,
            suffix: None,
            kind: Creation::File,
            dry_run: false,
            quiet: false,
            legacy: false,
            root: Root::AsWritten,
            workers,
        };
        let mut flags = true;
        while let Some(argument) = args.next() {
            let bytes = argument.as_encoded_bytes();
            if !flags || argument == "-" || !bytes.starts_with(b"-") {
                if parser.template.replace(argument.clone()).is_some() {
                    return Err(Error::Operand(argument));
                }
                continue;
            }
            if let Some(count) = jobs::option(bytes, &mut args) {
                parser.workers = count?;
                continue;
            }
            match bytes {
                b"--" => flags = false,
                b"--help" => return Ok(Request::Help),
                b"--version" => return Ok(Request::Version),
                b"--directory" => parser.kind = Creation::Directory,
                b"--dry-run" => parser.dry_run = true,
                b"--quiet" => parser.quiet = true,
                b"--tmpdir" => parser.root = Root::Default,
                b"--suffix" => parser.suffix = Some(args.next().ok_or(Error::Missing("--suffix"))?),
                _ if bytes.starts_with(b"--tmpdir=") => {
                    parser.root = Root::Explicit(PathBuf::from(native::suffix(&argument, 9)));
                }
                _ if bytes.starts_with(b"--suffix=") => {
                    parser.suffix = Some(native::suffix(&argument, 9));
                }
                [b'-', short @ ..] if !short.starts_with(b"-") => {
                    parser.short(&argument, short, &mut args)?;
                }
                _ => return Err(Error::Option(argument)),
            }
        }
        parser.finish().map(Request::Create)
    }
}

impl Parser {
    fn short(
        &mut self,
        argument: &std::ffi::OsStr,
        flags: &[u8],
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), Error> {
        for (index, flag) in flags.iter().enumerate() {
            let rest = &flags[index + 1..];
            match flag {
                b'd' => self.kind = Creation::Directory,
                b'u' => self.dry_run = true,
                b'q' => self.quiet = true,
                b't' => self.legacy = true,
                b'p' => {
                    let root = if rest.is_empty() {
                        args.next().ok_or(Error::Missing("-p"))?
                    } else {
                        native::suffix(argument, index + 2)
                    };
                    self.root = Root::Explicit(PathBuf::from(root));
                    break;
                }
                b'j' => {
                    self.workers = jobs::short_value(rest, args)?;
                    break;
                }
                _ => return Err(Error::Option(argument.to_os_string())),
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Options, Error> {
        let default = self.template.is_none();
        let value = self.template.unwrap_or_else(|| "tmp.XXXXXXXXXX".into());
        let location = if self.legacy {
            if template::has_separator(&value) || !directory_relative(&value) {
                return Err(Error::Legacy);
            }
            Location::Legacy(self.root.directory())
        } else if !matches!(self.root, Root::AsWritten) || default {
            if !directory_relative(&value) {
                return Err(Error::Absolute);
            }
            Location::Temporary(self.root.directory())
        } else {
            Location::AsWritten
        };
        let action = if self.dry_run {
            Action::NameOnly
        } else if matches!(self.kind, Creation::Directory) {
            Action::Directory
        } else {
            Action::File
        };
        Ok(Options {
            template: Template::parse(&value, self.suffix)?,
            location,
            action,
            quiet: self.quiet,
            workers: self.workers,
        })
    }
}

fn directory_relative(value: &std::ffi::OsStr) -> bool {
    let path = std::path::Path::new(value);
    !path.has_root()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::Prefix(_)))
}
