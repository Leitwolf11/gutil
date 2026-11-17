use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::Path;

use rustc_hash::FxHashMap;

use crate::config::When;
use crate::jobs;
use crate::presentation::{color_enabled, Presentation};

mod options;
mod render;
mod source;

use options::{Options, Request};
use render::Column;
use source::{Kind, Mount, Usage};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Df,
    Duf,
}

impl Command {
    const fn label(self) -> &'static str {
        match self {
            Self::Df => "gutil df",
            Self::Duf => "gutil duf",
        }
    }

    const fn help(self) -> &'static str {
        match self {
            Self::Df => "Usage: gutil df [OPTIONS] [FILE]...\n\nReport disk space for mounted filesystems.\n\n  -a, --all              Include duplicate and special mounts\n  -B, --block-size=SIZE  Scale block counts by SIZE\n  -h, --human-readable   Use binary size prefixes\n  -H, --si               Use decimal size prefixes\n  -i, --inodes           Report inode counts\n  -k                     Use 1024-byte blocks\n  -l, --local            Limit results to local filesystems\n  -P, --portability      Use POSIX headings\n  -T, --print-type       Include filesystem type\n  -t, --type=TYPE        Include only TYPE\n  -x, --exclude-type=TYPE  Exclude TYPE\n      --output[=FIELDS]  Choose output columns\n      --total            Include a grand total\n      --sync, --no-sync  Control syncing before measurement on Linux\n      --help             Print help\n      --version          Print version\n",
            Self::Duf => "Usage: gutil duf [OPTIONS] [PATH]...\n\nShow mounted filesystem usage in a table or JSON.\n\n  --all              Include duplicate and special mounts\n  --json             Write a JSON array\n  --inodes           Report inode counts when available\n  --only GROUPS      Include these device groups\n  --hide GROUPS      Exclude these device groups\n  --only-fs TYPES    Include these filesystem types\n  --hide-fs TYPES    Exclude these filesystem types\n  --only-mp GLOBS    Include these mount points\n  --hide-mp GLOBS    Exclude these mount points\n  --output COLUMNS   Choose and order table columns\n  --sort COLUMN      Sort table rows by COLUMN\n  --style STYLE      Use unicode or ascii table borders\n  --width WIDTH      Set the table width\n  --warnings         Report unavailable mount statistics\n  --help             Print help\n  --version          Print version\n",
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("--{0} requires a value")]
    Missing(String),
    #[error("invalid value for --{0}")]
    Value(String),
    #[error("invalid or repeated column {0:?}")]
    Column(String),
    #[error("invalid device group {0:?}")]
    Group(String),
    #[error("--output cannot be combined with inode, portability, or type layout flags")]
    ConflictingColumns,
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
    #[error(transparent)]
    Glob(#[from] globset::Error),
}

struct Row {
    device: String,
    target: String,
    filesystem: String,
    file: String,
    kind: Kind,
    options: String,
    usage: Option<Usage>,
}

impl Row {
    fn new(mount: &Mount, file: &OsStr, usage: Option<Usage>) -> Self {
        Self {
            device: mount.device.to_string_lossy().into_owned(),
            target: mount.path.to_string_lossy().into_owned(),
            filesystem: mount.filesystem.to_string_lossy().into_owned(),
            file: if file.is_empty() {
                "-".into()
            } else {
                file.to_string_lossy().into_owned()
            },
            kind: mount.kind,
            options: mount.options.to_string_lossy().into_owned(),
            usage,
        }
    }

    fn total(rows: &[Self]) -> Self {
        let usage = rows
            .iter()
            .filter_map(|row| row.usage)
            .fold(Usage::default(), Usage::sum);
        Self {
            device: "total".into(),
            target: "-".into(),
            filesystem: "-".into(),
            file: "-".into(),
            kind: Kind::Special,
            options: String::new(),
            usage: Some(usage),
        }
    }
}

pub(crate) fn run(
    command: Command,
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    color: When,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match options::parse(command, args, workers) {
        Ok(Request::Help) => {
            return io::stdout()
                .write_all(command.help().as_bytes())
                .map(|()| 0)
        }
        Ok(Request::Version) => {
            return writeln!(
                io::stdout(),
                "{} {}",
                command.label(),
                env!("CARGO_PKG_VERSION")
            )
            .map(|()| 0)
        }
        Ok(Request::List(options)) => options,
        Err(error) => {
            ui.error(&mut io::stderr(), command.label(), &error.to_string())?;
            return Ok(1);
        }
    };
    let stdout = io::stdout();
    let color = color_enabled(color, stdout.is_terminal());
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();
    let (mut rows, failed) = collect(command, &options, ui, &mut errors)?;
    if command == Command::Df {
        if rows.is_empty() {
            ui.error(&mut errors, command.label(), "no filesystems processed")?;
            return Ok(1);
        }
        if options.total {
            rows.push(Row::total(&rows));
        }
        render::df(&rows, &options.layout, options.size, &mut output)?;
    } else {
        render::duf(&mut rows, &options.layout, color, &mut output)?;
    }
    output.flush()?;
    Ok(u8::from(failed))
}

fn collect(
    command: Command,
    options: &Options,
    ui: Presentation,
    errors: &mut impl Write,
) -> io::Result<(Vec<Row>, bool)> {
    if options.sync {
        source::sync();
    }
    let mounts = source::mounts()?;
    let mut failure = false;
    let mut selected = Vec::new();
    if options.paths.is_empty() {
        let mut identities = HashSet::new();
        let mut paths = HashSet::new();
        for (index, mount) in mounts.iter().enumerate().rev() {
            if !options.include_all && !paths.insert(&mount.path) {
                continue;
            }
            if !matches(options, mount) {
                continue;
            }
            if options.include_all
                || mount
                    .identity
                    .as_ref()
                    .is_none_or(|identity| identities.insert(identity))
            {
                selected.push((index, OsStr::new("")));
            }
        }
        selected.reverse();
    } else {
        let mut resolved = FxHashMap::default();
        for path in &options.paths {
            let lookup = resolved
                .entry(path.as_path())
                .or_insert_with(|| find_mount(&mounts, path));
            match &*lookup {
                Ok(index) if matches(options, &mounts[*index]) => {
                    selected.push((*index, path.as_os_str()));
                }
                Ok(_) => {}
                Err(error) => {
                    failure = true;
                    ui.error(
                        errors,
                        command.label(),
                        &format!("{}: {error}", path.display()),
                    )?;
                }
            }
        }
    }
    if command == Command::Duf && !options.include_all {
        let mut seen = HashSet::new();
        selected.retain(|(index, _)| seen.insert(*index));
    }
    let mut unique: Vec<_> = selected.iter().map(|(index, _)| *index).collect();
    unique.sort_unstable();
    unique.dedup();
    let usages = measure(&mounts, unique, options.workers)?;
    let filter_zero = options.paths.is_empty() && !options.include_all;
    let mut rows = Vec::with_capacity(selected.len());
    for (index, file) in selected {
        let mount = &mounts[index];
        let Some(usage) = &usages[index] else {
            continue;
        };
        match usage {
            Ok(usage) if filter_zero && usage.size == 0 => {}
            Ok(usage) => rows.push(Row::new(mount, file, Some(*usage))),
            Err(error) => {
                if options.layout.duf.warnings {
                    ui.warning(
                        errors,
                        command.label(),
                        &format!("{}: {error}", mount.path.display()),
                    )?;
                }
                if options.include_all {
                    rows.push(Row::new(mount, file, None));
                } else if !options.paths.is_empty() {
                    failure = true;
                    ui.error(
                        errors,
                        command.label(),
                        &format!("{}: {error}", mount.path.display()),
                    )?;
                }
            }
        }
    }
    Ok((rows, failure))
}

fn matches(options: &Options, mount: &Mount) -> bool {
    options
        .filter
        .matches(mount.kind, &mount.filesystem, &mount.path)
}

fn find_mount(mounts: &[Mount], file: &Path) -> io::Result<usize> {
    let path = crate::paths::canonical::native_path(std::fs::canonicalize(file)?);
    mounts
        .iter()
        .enumerate()
        .filter(|(_, mount)| path.starts_with(&mount.path) || mount.device == path.as_os_str())
        .max_by_key(|(_, mount)| {
            (
                mount.device == path.as_os_str(),
                mount.path.as_os_str().len(),
            )
        })
        .map(|(index, _)| index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no mounted filesystem"))
}

fn measure(
    mounts: &[Mount],
    unique: Vec<usize>,
    workers: jobs::Count,
) -> io::Result<Vec<Option<io::Result<Usage>>>> {
    let mut usage: Vec<_> = std::iter::repeat_with(|| None).take(mounts.len()).collect();
    // Mount statistics are independent. Small lists stay inline to avoid
    // starting a worker per query. Repeated operands reuse the same snapshot.
    if workers.get() == 1 || unique.len() < 8 {
        for index in unique {
            usage[index] = Some(source::usage(&mounts[index]));
        }
    } else {
        let tasks = unique
            .into_iter()
            .map(|index| (index, mounts[index].clone()))
            .collect();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(workers.get())
            .build()?;
        for (index, stats) in runtime.block_on(jobs::map(tasks, workers, |(index, mount)| {
            (index, source::usage(&mount))
        }))? {
            usage[index] = Some(stats);
        }
    }
    Ok(usage)
}
