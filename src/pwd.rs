use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::jobs;

const HELP: &str = "Usage: gutil pwd [OPTION]...\n\nPrint the absolute path of the current working directory.\n\n  -L, --logical          Use PWD when it names the current directory\n  -P, --physical         Resolve symbolic links\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n      --help             Print help\n      --version          Print version\n\nPhysical mode is the default unless POSIXLY_CORRECT is set.\n";

enum Request {
    Print {
        logical: bool,
        extra_operands: bool,
        jobs: usize,
    },
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
    mut jobs: usize,
    posix: bool,
) -> Result<Request, ParseError> {
    let mut logical = posix;
    let mut extra_operands = false;
    while let Some(arg) = args.next() {
        let bytes = arg.as_encoded_bytes();
        if let Some(count) = jobs::option(bytes, &mut args) {
            jobs = count?;
            continue;
        }
        match bytes {
            b"--" => {
                extra_operands |= args.next().is_some();
                break;
            }
            [b'-', b'-', name @ ..] if b"logical".starts_with(name) => logical = true,
            [b'-', b'-', name @ ..] if b"physical".starts_with(name) => logical = false,
            [b'-', b'-', name @ ..] if b"help".starts_with(name) => return Ok(Request::Help),
            [b'-', b'-', name @ ..] if b"version".starts_with(name) => return Ok(Request::Version),
            [b'-', b'-', ..] => return Err(ParseError::InvalidOption(arg)),
            [b'-', flags @ ..] if !flags.is_empty() => {
                for (index, &flag) in flags.iter().enumerate() {
                    match flag {
                        b'L' => logical = true,
                        b'P' => logical = false,
                        b'j' => {
                            jobs = jobs::short_value(&flags[index + 1..], &mut args)?;
                            break;
                        }
                        _ => return Err(ParseError::InvalidOption(arg)),
                    }
                }
            }
            _ => {
                extra_operands = true;
                if posix {
                    break;
                }
            }
        }
    }
    Ok(Request::Print {
        logical,
        extra_operands,
        jobs,
    })
}

pub(super) fn run(
    args: impl Iterator<Item = OsString>,
    job_count: usize,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let (logical, jobs) = match parse(args, job_count, env::var_os("POSIXLY_CORRECT").is_some()) {
        Ok(Request::Help) => return output.write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(output, "gutil pwd {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Print {
            logical,
            extra_operands,
            jobs,
        }) => {
            if extra_operands {
                writeln!(errors, "gutil pwd: ignoring non-option arguments")?;
            }
            (logical, jobs)
        }
        Err(error) => {
            writeln!(
                errors,
                "gutil pwd: {error}\nTry 'gutil pwd --help' for more information."
            )?;
            return Ok(1);
        }
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(jobs)
        .build()?;
    let path = runtime.block_on(async move {
        tokio::task::spawn_blocking(move || working_directory(logical))
            .await
            .map_err(io::Error::other)?
    });

    match path {
        Ok(path) => {
            write_path(output, &path)?;
            output.write_all(b"\n")?;
            Ok(0)
        }
        Err(error) => {
            writeln!(
                errors,
                "gutil pwd: cannot determine the working directory: {error}"
            )?;
            Ok(1)
        }
    }
}

fn working_directory(logical: bool) -> io::Result<PathBuf> {
    if logical {
        if let Some(path) = env::var_os("PWD")
            .map(PathBuf::from)
            .filter(|path| valid_pwd(path))
        {
            return Ok(path);
        }
    }
    physical_path()
}

fn valid_pwd(path: &Path) -> bool {
    // Path::components normalizes interior dots, so inspect the original spelling.
    let has_dot_component = path
        .as_os_str()
        .as_encoded_bytes()
        .split(|&byte| byte == b'/' || (cfg!(windows) && byte == b'\\'))
        .any(|component| matches!(component, b"." | b".."));
    path.is_absolute() && !has_dot_component && identifies_current_dir(path).unwrap_or(false)
}

#[cfg(unix)]
fn identifies_current_dir(path: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let candidate = fs::metadata(path)?;
    let current = fs::metadata(".")?;
    Ok(candidate.dev() == current.dev() && candidate.ino() == current.ino())
}

#[cfg(not(unix))]
fn identifies_current_dir(path: &Path) -> io::Result<bool> {
    same_file::is_same_file(path, ".")
}

#[cfg(unix)]
fn physical_path() -> io::Result<PathBuf> {
    env::current_dir()
}

#[cfg(not(unix))]
fn physical_path() -> io::Result<PathBuf> {
    fs::canonicalize(".")
}

fn write_path(output: &mut impl Write, path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.write_all(path.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        let path = path.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "working directory path is not valid UTF-8",
            )
        })?;
        output.write_all(path.as_bytes())
    }
}
