use std::ffi::OsString;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::{jobs, presentation::Presentation};

mod links;
mod options;
mod size;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Link,
    Ln,
    Unlink,
    Rmdir,
    Truncate,
}

impl Command {
    const fn label(self) -> &'static str {
        match self {
            Self::Link => "gutil link",
            Self::Ln => "gutil ln",
            Self::Unlink => "gutil unlink",
            Self::Rmdir => "gutil rmdir",
            Self::Truncate => "gutil truncate",
        }
    }

    const fn usage(self) -> &'static str {
        match self {
            Self::Ln => "[OPTIONS] TARGET... [LINK_NAME|DIRECTORY]\n\nCreate hard or symbolic links.\n\n  -s, --symbolic        Create symbolic links\n  -f, --force           Replace existing files\n  -i, --interactive     Ask before replacement\n  -L, --logical         Follow hard-link source symlinks\n  -P, --physical        Link source symlinks themselves\n  -n, --no-dereference  Treat destination symlinks as files\n  -r, --relative        Make symbolic link text relative\n  -t, --target-directory=DIR  Put links in DIR\n  -T, --no-target-directory  Treat destination as a file\n  -b, --backup[=CONTROL]  Back up existing destinations\n  -S, --suffix=SUFFIX   Set the backup suffix\n  -v, --verbose         Report created links\n\nOn Windows, enable Developer Mode to create symbolic links without elevation.",
            Self::Link => "FILE1 FILE2\n\nCreate a hard link named FILE2 to FILE1.",
            Self::Unlink => "FILE\n\nRemove one file name.",
            Self::Rmdir => "[OPTIONS] DIRECTORY...\n\nRemove empty directories.\n\n  -p, --parents  Remove each directory and its ancestors\n  -v, --verbose  Report every attempted removal\n      --ignore-fail-on-non-empty  Ignore nonempty directories",
            Self::Truncate => "[OPTIONS] FILE...\n\nShrink or extend files.\n\n  -s, --size=SIZE       Set or adjust size\n  -r, --reference=FILE  Use the reference size\n  -c, --no-create       Skip missing files\n  -o, --io-blocks       Express SIZE in filesystem I/O blocks\n\nSIZE accepts binary or decimal units and +, -, <, >, /, % modifiers.",
        }
    }
}

struct Removal {
    parents: bool,
    verbose: bool,
    ignore_nonempty: bool,
}

enum Work {
    Link { source: PathBuf, target: PathBuf },
    Unlink(PathBuf),
    Ln(links::Options, Vec<PathBuf>),
    Rmdir(Removal, Vec<PathBuf>),
    Truncate(size::Options, Vec<PathBuf>),
}

enum Request {
    Help,
    Version,
    Run(Work, jobs::Count),
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("missing operand")]
    MissingOperand,
    #[error("extra operand {0:?}")]
    ExtraOperand(PathBuf),
    #[error("{0} requires a value")]
    MissingValue(&'static str),
    #[error("invalid size {0:?}")]
    Size(OsString),
    #[error("specify --size or --reference")]
    MissingSize,
    #[error("--reference requires a relative --size when both are specified")]
    AbsoluteReference,
    #[error("--io-blocks requires --size")]
    MissingBlockSize,
    #[error("invalid backup control {0:?}")]
    Backup(OsString),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

pub(crate) fn run(
    command: Command,
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let request = match options::parse(command, args, workers) {
        Ok(request) => request,
        Err(error) => {
            ui.error(&mut io::stderr(), command.label(), &error.to_string())?;
            return Ok(1);
        }
    };
    let (work, workers) = match request {
        Request::Help => return writeln!(io::stdout(), "Usage: {} {}\n\n  -j, --jobs, --threads N  Set the runtime worker limit\n      --help               Print help\n      --version            Print version", command.label(), command.usage()).map(|()| 0),
        Request::Version => return writeln!(io::stdout(), "{} {}", command.label(), env!("CARGO_PKG_VERSION")).map(|()| 0),
        Request::Run(work, workers) => (work, workers),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(workers.get())
        .build()?;
    runtime
        .block_on(
            async move { tokio::task::spawn_blocking(move || execute(command, work, ui)).await },
        )
        .map_err(io::Error::other)?
}

fn execute(command: Command, work: Work, ui: Presentation) -> io::Result<u8> {
    let mut output = BufWriter::new(io::stdout().lock());
    let mut errors = io::stderr().lock();
    let mut progress = ui.progress(command.label(), "operands", true);
    let mut failed = false;
    match work {
        Work::Link { source, target } => {
            if let Err(error) = std::fs::hard_link(&source, &target) {
                ui.error(
                    &mut errors,
                    command.label(),
                    &format!("{} -> {}: {error}", quote(&source), quote(&target)),
                )?;
                failed = true;
            }
            if let Some(progress) = &mut progress {
                progress.advance(1, &mut errors);
            }
        }
        Work::Unlink(path) => {
            if let Err(error) = std::fs::remove_file(&path) {
                ui.error(
                    &mut errors,
                    command.label(),
                    &format!("{}: {error}", quote(&path)),
                )?;
                failed = true;
            }
            if let Some(progress) = &mut progress {
                progress.advance(1, &mut errors);
            }
        }
        Work::Rmdir(options, paths) => {
            for path in paths {
                if let Err(error) = remove(&path, &options, &mut output) {
                    ui.error(&mut errors, command.label(), &error.to_string())?;
                    failed = true;
                }
                if let Some(progress) = &mut progress {
                    progress.advance(1, &mut errors);
                }
            }
        }
        Work::Ln(options, paths) => {
            match links::execute(
                &options,
                paths,
                ui,
                &mut output,
                &mut errors,
                progress.as_mut(),
            ) {
                Ok(status) => failed = status,
                Err(error) => {
                    ui.error(&mut errors, command.label(), &error.to_string())?;
                    failed = true;
                }
            }
        }
        Work::Truncate(options, paths) => {
            let reference = match options.prepare() {
                Ok(reference) => reference,
                Err(error) => {
                    ui.error(&mut errors, command.label(), &error.to_string())?;
                    return Ok(1);
                }
            };
            for path in paths {
                if let Err(error) = options.truncate(&path, reference) {
                    ui.error(
                        &mut errors,
                        command.label(),
                        &format!("{}: {error}", quote(&path)),
                    )?;
                    failed = true;
                }
                if let Some(progress) = &mut progress {
                    progress.advance(1, &mut errors);
                }
            }
        }
    }
    output.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(u8::from(failed))
}

fn remove(path: &Path, options: &Removal, output: &mut impl Write) -> io::Result<()> {
    let mut current = path.to_path_buf();
    loop {
        if options.verbose {
            writeln!(
                output,
                "gutil rmdir: removing directory, {}",
                quote(&current)
            )?;
        }
        if let Err(error) = std::fs::remove_dir(&current) {
            if options.ignore_nonempty
                && matches!(
                    error.kind(),
                    io::ErrorKind::DirectoryNotEmpty | io::ErrorKind::AlreadyExists
                )
            {
                return Ok(());
            }
            return Err(io::Error::new(
                error.kind(),
                format!("{}: {error}", quote(&current)),
            ));
        }
        if !options.parents {
            return Ok(());
        }
        let Some(parent) = lexical_parent(&current) else {
            return Ok(());
        };
        current = parent;
    }
}

#[cfg(unix)]
fn lexical_parent(path: &Path) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let end = bytes.iter().rposition(|byte| *byte != b'/')? + 1;
    let bytes = &bytes[..end];
    let separator = bytes.iter().rposition(|byte| *byte == b'/')?;
    let end = bytes[..separator]
        .iter()
        .rposition(|byte| *byte != b'/')
        .map_or(1, |position| position + 1);
    Some(std::ffi::OsStr::from_bytes(&bytes[..end]).into())
}

#[cfg(not(unix))]
fn lexical_parent(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.as_os_str().to_string_lossy().escape_debug())
}
