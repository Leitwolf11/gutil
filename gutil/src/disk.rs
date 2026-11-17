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
    device: OsString,
    target: OsString,
    filesystem: OsString,
    file: OsString,
    kind: Kind,
    options: OsString,
    usage: Option<Usage>,
}

impl Row {
    fn new(mount: &Mount, file: &OsStr, usage: Option<Usage>) -> Self {
        Self {
            device: mount.device.clone(),
            target: mount.path.as_os_str().to_owned(),
            filesystem: mount.filesystem.clone(),
            file: if file.is_empty() {
                "-".into()
            } else {
                file.to_owned()
            },
            kind: mount.kind,
            options: mount.options.clone(),
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
            options: OsString::new(),
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
        let size = options
            .size
            .unwrap_or_else(|| options::default_size(options.layout.portable));
        render::df(&rows, &options.layout, size, &mut output)?;
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
        selected = inventory(&mounts, options)
            .into_iter()
            .map(|index| (index, OsStr::new("")))
            .collect();
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
    let (rows, stats_failed) = rows(command, options, &mounts, selected, &usages, ui, errors)?;
    Ok((rows, failure || stats_failed))
}

fn inventory(mounts: &[Mount], options: &Options) -> Vec<usize> {
    if options.include_all {
        return mounts
            .iter()
            .enumerate()
            .filter_map(|(index, mount)| matches(options, mount).then_some(index))
            .collect();
    }
    let mut paths = HashSet::new();
    let mut identities = FxHashMap::<&OsStr, usize>::default();
    let mut selected = Vec::new();
    for (index, mount) in mounts.iter().enumerate().rev() {
        if !paths.insert(&mount.path) || !matches(options, mount) {
            continue;
        }
        if let Some(identity) = mount.identity.as_deref() {
            let candidate = identities.entry(identity).or_insert(index);
            if preference(mount) < preference(&mounts[*candidate]) {
                *candidate = index;
            }
        } else {
            selected.push(index);
        }
    }
    selected.extend(identities.into_values());
    selected.sort_unstable();
    selected
}

fn preference(mount: &Mount) -> (bool, usize) {
    (!mount.primary, mount.path.as_os_str().len())
}

fn rows(
    command: Command,
    options: &Options,
    mounts: &[Mount],
    selected: Vec<(usize, &OsStr)>,
    usages: &[Option<io::Result<Usage>>],
    ui: Presentation,
    errors: &mut impl Write,
) -> io::Result<(Vec<Row>, bool)> {
    let mut failure = false;
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
                failure = true;
                let message = format!("{}: {error}", mount.path.display());
                if options.layout.duf.warnings && options.include_all {
                    ui.warning(errors, command.label(), &message)?;
                } else {
                    ui.error(errors, command.label(), &message)?;
                }
                if options.include_all {
                    rows.push(Row::new(mount, file, None));
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
    if let Some(identity) = source::block_device_identity(&path) {
        if let Some(index) = device_mount(mounts, &identity) {
            return Ok(index);
        }
    }
    let identity = source::file_identity(&path);
    let has_identity = |mount: &Mount| {
        identity
            .as_ref()
            .is_some_and(|id| mount.identity.as_ref() == Some(id))
    };
    mounts
        .iter()
        .enumerate()
        .filter(|(_, mount)| {
            path.starts_with(&mount.path) || mount.device == path.as_os_str() || has_identity(mount)
        })
        .max_by_key(|(_, mount)| {
            (
                mount.device == path.as_os_str(),
                has_identity(mount),
                mount.path.as_os_str().len(),
            )
        })
        .map(|(index, _)| index)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no mounted filesystem"))
}

fn device_mount(mounts: &[Mount], identity: &OsStr) -> Option<usize> {
    mounts
        .iter()
        .enumerate()
        .filter(|(_, mount)| mount.identity.as_deref() == Some(identity))
        .min_by_key(|(_, mount)| preference(mount))
        .map(|(index, _)| index)
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{find_mount, options, rows, Command, Kind, Mount, Presentation};
    use crate::config::Settings;
    use crate::jobs::Count;
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::path::Path;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn handles_inaccessible_filesystems_as_failed_operands_or_placeholder_rows() {
        let broken = Mount {
            path: "/test/mount".into(),
            device: "example".into(),
            kind: Kind::Network,
            filesystem: "nfs".into(),
            options: "ro".into(),
            identity: None,
            primary: true,
        };
        let usage = [Some(Err(io::Error::from(io::ErrorKind::PermissionDenied)))];
        let mounts = [broken];
        for (arguments, label, failed, count) in [
            (&["--warnings", "--all", "file"][..], "gutil duf", true, 1),
            (&["--warnings", "file"], "gutil duf", true, 0),
            (&["file"], "gutil duf", true, 0),
            (&[], "gutil duf", true, 0),
        ] {
            let args = arguments.iter().map(OsString::from);
            let options::Request::List(options) =
                options::parse(Command::Duf, args, Count::DEFAULT_AUTO_LIMIT).unwrap()
            else {
                panic!("expected listing");
            };
            let mut errors = Vec::new();
            let (records, failure) = rows(
                Command::Duf,
                &options,
                &mounts,
                vec![(0, OsStr::new("file"))],
                &usage,
                Presentation::new(&Settings::default()),
                &mut errors,
            )
            .unwrap();
            assert_eq!((failure, records.len()), (failed, count));
            assert!(String::from_utf8(errors).unwrap().contains(label));
            if let Some(row) = records.first() {
                assert!(row.usage.is_none());
            }
        }
        assert!(find_mount(&mounts, Path::new(".")).is_err());
    }
    #[test]
    #[allow(clippy::unwrap_used)]
    fn selects_primary_mounts_and_keeps_topmost_overlapping_mounts() {
        let mount = |path: &str, identity: &str, primary| Mount {
            path: path.into(),
            device: "device".into(),
            kind: if primary { Kind::Local } else { Kind::Binds },
            filesystem: "ext4".into(),
            options: "rw".into(),
            identity: Some(identity.into()),
            primary,
        };
        let mounts = [
            mount("/", "8:1", true),
            mount("/later-bind", "8:1", false),
            mount("/z-long-bind", "8:2", false),
            mount("/short", "8:2", false),
            mount("/overlay", "8:3", true),
            mount("/overlay", "8:4", true),
        ];
        let options::Request::List(options) =
            options::parse(Command::Df, std::iter::empty(), Count::DEFAULT_AUTO_LIMIT).unwrap()
        else {
            panic!("expected inventory")
        };
        assert_eq!(super::inventory(&mounts, &options), [0, 3, 5]);
        assert_eq!(super::device_mount(&mounts, OsStr::new("8:1")), Some(0));
        assert_eq!(super::device_mount(&mounts, OsStr::new("8:2")), Some(3));
        assert_eq!(super::device_mount(&mounts, OsStr::new("99:1")), None);
    }
}
