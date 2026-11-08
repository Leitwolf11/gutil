use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use smallvec::SmallVec;

use crate::jobs;

mod long;

const HELP: &str = "Usage: gutil ls [OPTION]... [FILE]...\n\nList information about files (the current directory by default).\n\n  -a, --all          Include entries beginning with .\n  -A, --almost-all   Include hidden entries except . and ..\n  -d, --directory    List directory operands themselves\n  -l                 Use long format\n  -1                 Use one entry per line\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n      --help         Print help\n      --version      Print version\n";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Hidden {
    Omit,
    AlmostAll,
    All,
}

#[derive(Clone, Copy)]
struct Options {
    hidden: Hidden,
    directory: bool,
    long: bool,
    job_count: usize,
}

enum Request {
    List(Options, SmallVec<[PathBuf; 4]>),
    Help,
    Version,
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    InvalidOption(OsString),
    #[error(transparent)]
    InvalidJobs(#[from] jobs::InvalidCount),
}

fn parse(
    mut args: impl Iterator<Item = OsString>,
    job_count: usize,
) -> Result<Request, ParseError> {
    let mut options = Options {
        hidden: Hidden::Omit,
        directory: false,
        long: false,
        job_count,
    };
    let mut paths = SmallVec::new();
    let mut parse_options = true;
    while let Some(arg) = args.next() {
        if parse_options {
            if let Some(count) = jobs::option(arg.as_encoded_bytes(), &mut args) {
                options.job_count = count?;
                continue;
            }
        }
        match arg.as_encoded_bytes() {
            b"--" if parse_options => parse_options = false,
            b"--help" if parse_options => return Ok(Request::Help),
            b"--version" if parse_options => return Ok(Request::Version),
            b"--all" if parse_options => options.hidden = Hidden::All,
            b"--almost-all" if parse_options => {
                options.hidden = options.hidden.max(Hidden::AlmostAll);
            }
            b"--directory" if parse_options => options.directory = true,
            [b'-', b'-', ..] if parse_options => return Err(ParseError::InvalidOption(arg)),
            [b'-', flags @ ..] if parse_options && !flags.is_empty() => {
                for (index, flag) in flags.iter().enumerate() {
                    match flag {
                        b'a' => options.hidden = Hidden::All,
                        b'A' => options.hidden = options.hidden.max(Hidden::AlmostAll),
                        b'd' => options.directory = true,
                        b'l' => options.long = true,
                        b'1' => {}
                        b'j' => {
                            options.job_count = jobs::short_value(&flags[index + 1..], &mut args)?;
                            break;
                        }
                        _ => return Err(ParseError::InvalidOption(arg)),
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
    args: impl Iterator<Item = OsString>,
    job_count: usize,
    terminal: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let (options, paths) = match parse(args, job_count) {
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

    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.job_count)
        .build()?;
    runtime.block_on(list(options, paths, terminal, output, errors))
}

async fn list(
    options: Options,
    mut paths: SmallVec<[PathBuf; 4]>,
    terminal: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
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
    let operands = jobs::map(paths.into_vec(), options.job_count, move |path| {
        let entry = read_operand(&path, options);
        (path, entry)
    })
    .await?;
    for (path, entry) in operands {
        match entry {
            Ok(None) => directories.push(path),
            Ok(Some(entry)) => {
                listing.write_entry(&path, path.as_os_str(), &entry)?;
                wrote_section = true;
            }
            Err(error) => listing.report(&path, &error, 2)?,
        }
    }

    for path in directories {
        let directory = path.clone();
        let entries = tokio::task::spawn_blocking(move || read_names(&directory, options.hidden))
            .await
            .map_err(io::Error::other)?;
        match entries {
            Ok((names, errors)) => {
                if wrote_section {
                    writeln!(listing.output)?;
                }
                if headers {
                    write_name(listing.output, path.as_os_str(), terminal)?;
                    writeln!(listing.output, ":")?;
                }
                for error in errors {
                    listing.report(&path, &error, 1)?;
                }
                listing.write_directory(&path, names).await?;
                wrote_section = true;
            }
            Err(error) => listing.report(&path, &error, 2)?,
        }
    }
    Ok(listing.status)
}

fn read_operand(path: &Path, options: Options) -> io::Result<Option<Entry>> {
    let metadata = fs::symlink_metadata(path)?;
    let is_directory = !options.directory
        && (metadata.is_dir()
            || (!options.long
                && metadata.is_symlink()
                && fs::metadata(path).is_ok_and(|target| target.is_dir())));
    Ok((!is_directory).then(|| Entry::new(path, metadata, options.long)))
}

fn read_names(directory: &Path, hidden: Hidden) -> io::Result<(Vec<OsString>, Vec<io::Error>)> {
    let mut names = Vec::new();
    let mut errors = Vec::new();
    if hidden == Hidden::All {
        names.extend([OsString::from("."), OsString::from("..")]);
    }
    for entry in fs::read_dir(directory)? {
        match entry {
            Ok(entry) => {
                let name = entry.file_name();
                if hidden != Hidden::Omit || !name.as_encoded_bytes().starts_with(b".") {
                    names.push(name);
                }
            }
            Err(error) => errors.push(error),
        }
    }
    names.sort_unstable();
    Ok((names, errors))
}

struct Entry {
    metadata: Metadata,
    target: Option<io::Result<PathBuf>>,
}

impl Entry {
    fn new(path: &Path, metadata: Metadata, long: bool) -> Self {
        let target = (long && metadata.is_symlink()).then(|| fs::read_link(path));
        Self { metadata, target }
    }
}

struct Listing<'a, O, E> {
    options: Options,
    terminal: bool,
    output: &'a mut O,
    errors: &'a mut E,
    status: u8,
}

impl<O: Write, E: Write> Listing<'_, O, E> {
    #[allow(clippy::unnecessary_debug_formatting)] // Escape control characters in paths.
    fn report(&mut self, path: &Path, error: &io::Error, status: u8) -> io::Result<()> {
        self.status = self.status.max(status);
        writeln!(self.errors, "gutil ls: {path:?}: {error}")
    }

    async fn write_directory(&mut self, directory: &Path, names: Vec<OsString>) -> io::Result<()> {
        if self.options.long {
            let root = directory.to_path_buf();
            let entries = jobs::map(names, self.options.job_count, move |name| {
                let path = root.join(&name);
                let entry =
                    fs::symlink_metadata(&path).map(|metadata| Entry::new(&path, metadata, true));
                (name, path, entry)
            })
            .await?;

            let mut successful = Vec::with_capacity(entries.len());
            for (name, path, entry) in entries {
                match entry {
                    Ok(entry) => successful.push((name, path, entry)),
                    Err(error) => self.report(&path, &error, 1)?,
                }
            }
            long::write_total(
                self.output,
                successful.iter().map(|(_, _, entry)| &entry.metadata),
            )?;
            for (name, path, entry) in successful {
                self.write_entry(&path, &name, &entry)?;
            }
        } else {
            for name in names {
                write_name(self.output, &name, self.terminal)?;
                writeln!(self.output)?;
            }
        }
        Ok(())
    }

    fn write_entry(&mut self, path: &Path, name: &OsStr, entry: &Entry) -> io::Result<()> {
        if self.options.long {
            long::write_metadata(self.output, &entry.metadata)?;
        }
        write_name(self.output, name, self.terminal)?;
        match &entry.target {
            Some(Ok(target)) => {
                self.output.write_all(b" -> ")?;
                write_name(self.output, target.as_os_str(), self.terminal)?;
            }
            Some(Err(error)) => self.report(path, error, 1)?,
            None => {}
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
