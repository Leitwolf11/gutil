use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use crate::jobs;
use crate::presentation::Presentation;

mod native;
mod options;
mod template;

use options::{Action, Location, Options, Request};

const HELP: &str = "Usage: gutil mktemp [OPTIONS] [TEMPLATE]\n\nCreate a temporary file or directory and print its name.\nThe final component must contain at least three consecutive X characters.\nThe default template is tmp.XXXXXXXXXX in the temporary directory.\n\n  -d, --directory    Create a directory\n  -u, --dry-run      Print an unused name without reserving it (unsafe to reuse)\n  -q, --quiet        Suppress creation-failure diagnostics\n      --suffix=TEXT  Append a suffix to a template ending in X\n  -p DIR, --tmpdir[=DIR]  Resolve a relative template under DIR\n  -t                Use TMPDIR before -p DIR (legacy single-component mode)\n  -j, --jobs, --threads N  Limit runtime workers\n      --help        Print help\n      --version     Print version\n\nWithout an explicit directory, use TMPDIR or the native temporary directory.\nUnix files use mode 600 and directories use 700, restricted by umask.\nWindows objects retain native inherited ACLs. Creation is atomic; dry-run\nnames are not reserved and can be claimed by another process.\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "gutil mktemp {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        Ok(Request::Create(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), "gutil mktemp", &error.to_string());
            return Ok(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(&options, ui))
            .await
            .map_err(io::Error::other)
    })
}

fn execute(options: &Options, ui: Presentation) -> u8 {
    let mut progress = ui.progress("gutil mktemp", "names", true);
    if let Some(progress) = &mut progress {
        progress.set_total(1);
    }
    let root = root(&options.location);
    let created = reserve(
        options.action,
        238_328,
        || options.template.name(root.as_deref()),
        close,
    );
    let path = match created {
        Ok(path) => path,
        Err(error) => {
            if !options.quiet {
                let _ = ui.error(&mut io::stderr(), "gutil mktemp", &error.to_string());
            }
            return 1;
        }
    };
    if let Err(error) = native::write_path(&mut io::stdout().lock(), &path) {
        let _ = ui.error(&mut io::stderr(), "gutil mktemp", &error.to_string());
        return 1;
    }
    if let Some(mut progress) = progress {
        progress.advance(1, &mut io::stderr());
        progress.finish(&mut io::stderr());
    }
    0
}

fn root(location: &Location) -> Option<PathBuf> {
    let environment = || {
        std::env::var_os("TMPDIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let explicit = |root: &Option<PathBuf>| {
        root.as_ref()
            .filter(|path| !path.as_os_str().is_empty())
            .cloned()
    };
    match location {
        Location::AsWritten => None,
        Location::Temporary(directory) => Some(
            explicit(directory)
                .or_else(environment)
                .unwrap_or_else(native_temp_dir),
        ),
        Location::Legacy(directory) => Some(
            environment()
                .or_else(|| explicit(directory))
                .unwrap_or_else(native_temp_dir),
        ),
    }
}

fn native_temp_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from("/tmp")
    }
    #[cfg(windows)]
    {
        std::env::temp_dir()
    }
}

fn reserve(
    action: Action,
    attempts: usize,
    mut name: impl FnMut() -> io::Result<PathBuf>,
    mut finish: impl FnMut(File) -> io::Result<()>,
) -> io::Result<PathBuf> {
    for _ in 0..attempts {
        let path = name()?;
        let result = match action {
            Action::File => create_file(&path).map(Some),
            Action::Directory => create_directory(&path).map(|()| None),
            Action::NameOnly => match fs::symlink_metadata(&path) {
                Ok(_) => Err(io::Error::from(io::ErrorKind::AlreadyExists)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(file) => {
                if let Some(file) = file {
                    // A close error happens after reservation and must never
                    // be mistaken for an open-time name collision.
                    finish(file)?;
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "cannot create temporary name '{}': {error}",
                        path.as_os_str().to_string_lossy().escape_debug()
                    ),
                ))
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not find an unused temporary name",
    ))
}

fn create_file(path: &std::path::Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn create_directory(path: &std::path::Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)
    }
    #[cfg(windows)]
    {
        fs::create_dir(path)
    }
}

fn close(file: File) -> io::Result<()> {
    #[cfg(unix)]
    {
        nix::unistd::close(file).map_err(io::Error::from)
    }
    #[cfg(windows)]
    {
        use io_close::Close;
        file.close()
    }
}

#[cfg(test)]
mod tests;
