use std::cmp::{Ordering, Reverse};
use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use smallvec::SmallVec;

use crate::config::{self, Hidden, Layout, Ls, Settings, Sort};
use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

mod display;
mod long;

const HELP: &str = "Usage: gutil ls [OPTION]... [FILE]...\n\nList information about files (the current directory by default).\n\n  -a, --all          Include entries beginning with .\n  -A, --almost-all   Include hidden entries except . and ..\n  -d, --directory    List directory operands themselves\n  -l                 Use long format\n  -1                 Use one entry per line\n  -C                 Use columns\n  -r, --reverse      Reverse the sort order\n      --layout=MODE  one-per-line, columns, or long\n      --hidden=MODE  omit, almost-all, or all\n      --sort=MODE    name, size, modified, extension, or none\n      --quote=MODE   auto, literal, or shell\n      --directories-first, --no-directories-first\n      --size-style=MODE, --time-style=MODE, --timezone=MODE, --owner-format=MODE\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n      --help         Print help\n      --version      Print version\n";

#[derive(Clone, Copy)]
struct Options {
    settings: Ls,
    directory: bool,
    job_count: usize,
}

impl Options {
    fn long(self) -> bool {
        self.settings.layout == Layout::Long
    }

    fn set_value(&mut self, argument: &str) -> Result<(), serde::de::value::Error> {
        let (key, value) = argument.split_once('=').unwrap_or((argument, ""));
        match key {
            "--layout" => self.settings.layout = config::option_value(value)?,
            "--hidden" => self.settings.hidden = config::option_value(value)?,
            "--sort" => self.settings.sort = config::option_value(value)?,
            "--quote" => self.settings.quote = config::option_value(value)?,
            "--size-style" => self.settings.long.size_style = config::option_value(value)?,
            "--time-style" => self.settings.long.time_style = config::option_value(value)?,
            "--timezone" => self.settings.long.timezone = config::option_value(value)?,
            "--owner-format" => self.settings.long.owner_format = config::option_value(value)?,
            _ => return Err(serde::de::Error::custom("unknown option")),
        }
        Ok(())
    }
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
    settings: Ls,
) -> Result<Request, ParseError> {
    let mut options = Options {
        settings,
        directory: false,
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
            b"--all" if parse_options => options.settings.hidden = Hidden::All,
            b"--almost-all" if parse_options => {
                options.settings.hidden = Hidden::AlmostAll;
            }
            b"--directory" if parse_options => options.directory = true,
            b"--reverse" if parse_options => options.settings.reverse = true,
            b"--no-reverse" if parse_options => options.settings.reverse = false,
            b"--directories-first" if parse_options => options.settings.directories_first = true,
            b"--no-directories-first" if parse_options => {
                options.settings.directories_first = false;
            }
            [b'-', b'-', ..] if parse_options => {
                if arg
                    .to_str()
                    .is_none_or(|value| options.set_value(value).is_err())
                {
                    return Err(ParseError::InvalidOption(arg));
                }
            }
            [b'-', flags @ ..] if parse_options && !flags.is_empty() => {
                for (index, flag) in flags.iter().enumerate() {
                    match flag {
                        b'a' => options.settings.hidden = Hidden::All,
                        b'A' => options.settings.hidden = Hidden::AlmostAll,
                        b'd' => options.directory = true,
                        b'l' => options.settings.layout = Layout::Long,
                        b'1' => options.settings.layout = Layout::OnePerLine,
                        b'C' => options.settings.layout = Layout::Columns,
                        b'r' => options.settings.reverse = true,
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
    settings: &Settings,
    terminal: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let ui = Presentation::new(*settings);
    let (options, paths) = match parse(args, job_count, settings.ls) {
        Ok(Request::Help) => return output.write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(output, "gutil ls {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::List(options, paths)) => (options, paths),
        Err(error) => {
            ui.error(
                errors,
                "gutil ls",
                &format!("{error}\nTry 'gutil ls --help' for more information."),
            )?;
            return Ok(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.job_count)
        .build()?;
    let mut listing = Listing {
        options,
        display: display::Names::new(settings.output, terminal, options.settings.quote),
        ui,
        progress: ui.progress("gutil ls", "entries"),
        output,
        errors,
        status: 0,
    };
    listing.advance(0);
    let result = runtime.block_on(list(options, paths, &mut listing));
    if result.is_ok() {
        if let Some(progress) = &mut listing.progress {
            progress.finish(listing.errors);
        }
    }
    result
}

async fn list<O: Write, E: Write>(
    options: Options,
    paths: SmallVec<[PathBuf; 4]>,
    listing: &mut Listing<'_, O, E>,
) -> io::Result<u8> {
    let headers = paths.len() > 1;
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let operands = jobs::map(paths.into_vec(), options.job_count, move |path| {
        let entry = read_operand(&path, options);
        (path, entry)
    })
    .await?;
    for (path, entry) in operands {
        match entry {
            Ok((true, entry)) => directories.push((path, entry)),
            Ok((false, entry)) => files.push((path, entry)),
            Err(error) => listing.report(&path, &error, 2)?,
        }
    }
    sort_entries(&mut files, options.settings);
    sort_entries(&mut directories, options.settings);
    let mut wrote_section = !files.is_empty();
    if options.settings.layout == Layout::Columns {
        let count = files.len() as u64;
        let cells = files
            .iter()
            .map(|(path, entry)| {
                listing
                    .display
                    .cell(path.as_os_str(), path, Some(&entry.metadata))
            })
            .collect();
        display::write_columns(listing.output, cells)?;
        listing.advance(count);
    } else {
        for (path, entry) in files {
            listing.write_entry(&path, path.as_os_str(), &entry)?;
        }
    }

    for (path, _) in directories {
        let directory = path.clone();
        let entries =
            tokio::task::spawn_blocking(move || read_names(&directory, options.settings.hidden))
                .await
                .map_err(io::Error::other)?;
        match entries {
            Ok((names, errors)) => {
                if wrote_section {
                    writeln!(listing.output)?;
                }
                if headers {
                    listing
                        .display
                        .write(listing.output, path.as_os_str(), &path, None)?;
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

fn read_operand(path: &Path, options: Options) -> io::Result<(bool, Entry)> {
    let metadata = fs::symlink_metadata(path)?;
    let is_directory = !options.directory
        && (metadata.is_dir()
            || (!options.long()
                && metadata.is_symlink()
                && fs::metadata(path).is_ok_and(|target| target.is_dir())));
    Ok((
        is_directory,
        Entry::new(path, metadata, options.long() && !is_directory),
    ))
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

fn compare_names(left: &Path, right: &Path, mode: Sort) -> Ordering {
    match mode {
        Sort::None => Ordering::Equal,
        Sort::Extension => left
            .extension()
            .cmp(&right.extension())
            .then_with(|| left.as_os_str().cmp(right.as_os_str())),
        _ => left.as_os_str().cmp(right.as_os_str()),
    }
}

fn sort_entries(entries: &mut [(PathBuf, Entry)], settings: Ls) {
    if settings.sort != Sort::None || settings.directories_first {
        entries.sort_by(|(left, a), (right, b)| {
            let group = if settings.directories_first {
                b.metadata.is_dir().cmp(&a.metadata.is_dir())
            } else {
                Ordering::Equal
            };
            group.then_with(|| {
                let order = match settings.sort {
                    Sort::Size => Reverse(a.metadata.len()).cmp(&Reverse(b.metadata.len())),
                    Sort::Modified => Reverse(a.metadata.modified().ok())
                        .cmp(&Reverse(b.metadata.modified().ok())),
                    _ => Ordering::Equal,
                }
                .then_with(|| compare_names(left, right, settings.sort));
                if settings.reverse {
                    order.reverse()
                } else {
                    order
                }
            })
        });
    }
    if settings.reverse && settings.sort == Sort::None {
        if settings.directories_first {
            let first_file = entries.partition_point(|(_, entry)| entry.metadata.is_dir());
            entries[..first_file].reverse();
            entries[first_file..].reverse();
        } else {
            entries.reverse();
        }
    }
}

struct Listing<'a, O, E> {
    options: Options,
    display: display::Names,
    ui: Presentation,
    progress: Option<ProgressReport>,
    output: &'a mut O,
    errors: &'a mut E,
    status: u8,
}

impl<O: Write, E: Write> Listing<'_, O, E> {
    fn advance(&mut self, amount: u64) {
        if let Some(progress) = &mut self.progress {
            progress.advance(amount, self.errors);
        }
    }

    #[allow(clippy::unnecessary_debug_formatting)] // Escape control characters in paths.
    fn report(&mut self, path: &Path, error: &io::Error, status: u8) -> io::Result<()> {
        self.status = self.status.max(status);
        self.ui
            .error(self.errors, "gutil ls", &format!("{path:?}: {error}"))
    }

    async fn write_directory(
        &mut self,
        directory: &Path,
        mut names: Vec<OsString>,
    ) -> io::Result<()> {
        let settings = self.options.settings;
        if self.options.long()
            || matches!(settings.sort, Sort::Size | Sort::Modified)
            || settings.directories_first
            || self.display.color()
        {
            let root = directory.to_path_buf();
            let long = self.options.long();
            let entries = jobs::map(names, self.options.job_count, move |name| {
                let path = root.join(&name);
                let entry =
                    fs::symlink_metadata(&path).map(|metadata| Entry::new(&path, metadata, long));
                (PathBuf::from(name), entry)
            })
            .await?;
            let mut successful = Vec::with_capacity(entries.len());
            for (name, entry) in entries {
                match entry {
                    Ok(entry) => successful.push((name, entry)),
                    Err(error) => self.report(&directory.join(&name), &error, 1)?,
                }
            }
            sort_entries(&mut successful, settings);
            if settings.layout == Layout::Columns {
                let cells = successful
                    .iter()
                    .map(|(name, entry)| {
                        self.display.cell(
                            name.as_os_str(),
                            &directory.join(name),
                            Some(&entry.metadata),
                        )
                    })
                    .collect();
                display::write_columns(self.output, cells)?;
                self.advance(successful.len() as u64);
                return Ok(());
            }
            if self.options.long() {
                long::write_total(
                    self.output,
                    successful.iter().map(|(_, entry)| &entry.metadata),
                )?;
            }
            for (name, entry) in successful {
                self.write_entry(&directory.join(&name), name.as_os_str(), &entry)?;
            }
        } else {
            if settings.sort != Sort::None {
                names.sort_unstable_by(|left, right| {
                    compare_names(Path::new(left), Path::new(right), settings.sort)
                });
            }
            if settings.reverse {
                names.reverse();
            }
            if settings.layout == Layout::Columns {
                let cells = names
                    .iter()
                    .map(|name| self.display.cell(name, &directory.join(name), None))
                    .collect();
                display::write_columns(self.output, cells)?;
                self.advance(names.len() as u64);
                return Ok(());
            }
            for name in names {
                let path = self.display.links().then(|| directory.join(&name));
                self.display.write(
                    self.output,
                    &name,
                    path.as_deref().unwrap_or(directory),
                    None,
                )?;
                writeln!(self.output)?;
                self.advance(1);
            }
        }
        Ok(())
    }

    fn write_entry(&mut self, path: &Path, name: &OsStr, entry: &Entry) -> io::Result<()> {
        if self.options.long() {
            long::write_metadata(self.output, &entry.metadata, self.options.settings.long)?;
        }
        self.display
            .write(self.output, name, path, Some(&entry.metadata))?;
        match &entry.target {
            Some(Ok(target)) => {
                self.output.write_all(b" -> ")?;
                let target_path = path.parent().unwrap_or_else(|| Path::new(".")).join(target);
                self.display
                    .write(self.output, target.as_os_str(), &target_path, None)?;
            }
            Some(Err(error)) => self.report(path, error, 1)?,
            None => {}
        }
        writeln!(self.output)?;
        self.advance(1);
        Ok(())
    }
}
