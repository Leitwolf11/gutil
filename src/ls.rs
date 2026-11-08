use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use smallvec::SmallVec;

mod long;

const HELP: &str = "Usage: gutil ls [OPTION]... [FILE]...\n\nList information about files (the current directory by default).\n\n  -a, --all          Include entries beginning with .\n  -A, --almost-all   Include hidden entries except . and ..\n  -d, --directory    List directory operands themselves\n  -l                 Use long format\n  -1                 Use one entry per line\n      --help         Print help\n      --version      Print version\n";

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Hidden {
    #[default]
    Omit,
    AlmostAll,
    All,
}

#[derive(Default)]
struct Options {
    hidden: Hidden,
    directory: bool,
    long: bool,
}

enum Request {
    List(Options, SmallVec<[PathBuf; 4]>),
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognized option {0:?}")]
struct InvalidOption(OsString);

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Request, InvalidOption> {
    let mut options = Options::default();
    let mut paths = SmallVec::new();
    let mut parse_options = true;
    for arg in args {
        match arg.as_encoded_bytes() {
            b"--" if parse_options => parse_options = false,
            b"--help" if parse_options => return Ok(Request::Help),
            b"--version" if parse_options => return Ok(Request::Version),
            b"--all" if parse_options => options.hidden = Hidden::All,
            b"--almost-all" if parse_options => {
                options.hidden = options.hidden.max(Hidden::AlmostAll);
            }
            b"--directory" if parse_options => options.directory = true,
            [b'-', b'-', ..] if parse_options => return Err(InvalidOption(arg)),
            [b'-', flags @ ..] if parse_options && !flags.is_empty() => {
                for flag in flags {
                    match flag {
                        b'a' => options.hidden = Hidden::All,
                        b'A' => options.hidden = options.hidden.max(Hidden::AlmostAll),
                        b'd' => options.directory = true,
                        b'l' => options.long = true,
                        b'1' => {}
                        _ => return Err(InvalidOption(arg)),
                    }
                }
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() {
        paths.push(PathBuf::from("."));
    }
    Ok(Request::List(options, paths))
}

pub(super) fn run(
    args: impl IntoIterator<Item = OsString>,
    terminal: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let (options, mut paths) = match parse(args) {
        Ok(Request::Help) => return output.write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(output, "gutil ls {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::List(options, paths)) => (options, paths),
        Err(error) => {
            writeln!(
                errors,
                "gutil ls: {error}\nTry 'gutil ls --help' for more information."
            )?;
            return Ok(2);
        }
    };

    let headers = paths.len() > 1;
    paths.sort_unstable();
    let mut listing = Listing {
        options,
        terminal,
        output,
        errors,
        status: 0,
    };
    let mut directories = SmallVec::<[PathBuf; 4]>::new();
    let mut wrote_section = false;
    for path in paths {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if listing.is_directory_operand(&path, &metadata) => {
                directories.push(path);
            }
            Ok(metadata) => {
                listing.write_entry(&path, path.as_os_str(), &metadata)?;
                wrote_section = true;
            }
            Err(error) => listing.report(&path, &error, 2)?,
        }
    }

    for path in directories {
        match fs::read_dir(&path) {
            Ok(entries) => {
                if wrote_section {
                    writeln!(listing.output)?;
                }
                if headers {
                    write_name(listing.output, path.as_os_str(), terminal)?;
                    writeln!(listing.output, ":")?;
                }
                listing.write_directory(&path, entries)?;
                wrote_section = true;
            }
            Err(error) => listing.report(&path, &error, 2)?,
        }
    }
    Ok(listing.status)
}

struct Listing<'a, O, E> {
    options: Options,
    terminal: bool,
    output: &'a mut O,
    errors: &'a mut E,
    status: u8,
}

impl<O: Write, E: Write> Listing<'_, O, E> {
    fn is_directory_operand(&self, path: &Path, metadata: &Metadata) -> bool {
        !self.options.directory
            && (metadata.is_dir()
                || (!self.options.long
                    && metadata.is_symlink()
                    && fs::metadata(path).is_ok_and(|target| target.is_dir())))
    }

    #[allow(clippy::unnecessary_debug_formatting)] // Escape control characters in paths.
    fn report(&mut self, path: &Path, error: &io::Error, status: u8) -> io::Result<()> {
        self.status = self.status.max(status);
        writeln!(self.errors, "gutil ls: {path:?}: {error}")
    }

    fn write_directory(&mut self, directory: &Path, entries: fs::ReadDir) -> io::Result<()> {
        let mut names = Vec::new();
        if self.options.hidden == Hidden::All {
            names.extend([OsString::from("."), OsString::from("..")]);
        }
        for entry in entries {
            match entry {
                Ok(entry) => {
                    let name = entry.file_name();
                    if self.options.hidden != Hidden::Omit
                        || !name.as_encoded_bytes().starts_with(b".")
                    {
                        names.push(name);
                    }
                }
                Err(error) => self.report(directory, &error, 1)?,
            }
        }
        names.sort_unstable();

        if self.options.long {
            let mut entries = Vec::with_capacity(names.len());
            for name in names {
                let path = directory.join(&name);
                match fs::symlink_metadata(&path) {
                    Ok(metadata) => entries.push((name, metadata)),
                    Err(error) => self.report(&path, &error, 1)?,
                }
            }
            long::write_total(self.output, entries.iter().map(|(_, metadata)| metadata))?;
            for (name, metadata) in entries {
                self.write_entry(&directory.join(&name), &name, &metadata)?;
            }
        } else {
            for name in names {
                write_name(self.output, &name, self.terminal)?;
                writeln!(self.output)?;
            }
        }
        Ok(())
    }

    fn write_entry(&mut self, path: &Path, name: &OsStr, metadata: &Metadata) -> io::Result<()> {
        if self.options.long {
            long::write_metadata(self.output, metadata)?;
            write_name(self.output, name, self.terminal)?;
            if metadata.is_symlink() {
                match fs::read_link(path) {
                    Ok(target) => {
                        self.output.write_all(b" -> ")?;
                        write_name(self.output, target.as_os_str(), self.terminal)?;
                    }
                    Err(error) => self.report(path, &error, 1)?,
                }
            }
        } else {
            write_name(self.output, name, self.terminal)?;
        }
        writeln!(self.output)
    }
}

fn write_name(output: &mut impl Write, name: &OsStr, terminal: bool) -> io::Result<()> {
    if terminal {
        let name = name.to_string_lossy();
        for character in name.chars() {
            let printable = if character.is_control() {
                '?'
            } else {
                character
            };
            write!(output, "{printable}")?;
        }
        return Ok(());
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.write_all(name.as_bytes())
    }
    #[cfg(not(unix))]
    {
        output.write_all(name.to_string_lossy().as_bytes())
    }
}
