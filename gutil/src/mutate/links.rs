use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::paths::canonical::{self, Existence, Mode};
use crate::presentation::{Presentation, ProgressReport};

#[derive(Clone, Copy, Default)]
pub(super) enum Replacement {
    #[default]
    Refuse,
    Force,
    Interactive,
}

#[derive(Clone, Copy)]
pub(super) enum Backup {
    Off,
    Simple,
    Numbered,
    Existing,
}

impl Backup {
    pub(super) fn parse(value: &OsStr) -> Option<Self> {
        match value.as_encoded_bytes() {
            b"none" | b"off" => Some(Self::Off),
            b"simple" | b"never" => Some(Self::Simple),
            b"numbered" | b"t" => Some(Self::Numbered),
            b"existing" | b"nil" => Some(Self::Existing),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Hard,
    Symbolic,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Follow {
    #[default]
    Physical,
    Logical,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Destination {
    #[default]
    Detect,
    Direct,
}

#[derive(Default)]
pub(super) struct Options {
    kind: Kind,
    follow: Follow,
    pub(super) relative: bool,
    pub(super) no_dereference: bool,
    destination: Destination,
    pub(super) target_directory: Option<PathBuf>,
    pub(super) verbose: bool,
    pub(super) replacement: Replacement,
    pub(super) backup: Option<Backup>,
    pub(super) suffix: Option<OsString>,
}

impl Options {
    pub(super) fn flag(&mut self, flag: u8) -> bool {
        match flag {
            b's' => self.kind = Kind::Symbolic,
            b'L' => self.follow = Follow::Logical,
            b'P' => self.follow = Follow::Physical,
            b'r' => self.relative = true,
            b'n' => self.no_dereference = true,
            b'T' => self.destination = Destination::Direct,
            b'v' => self.verbose = true,
            b'f' => self.replacement = Replacement::Force,
            b'i' => self.replacement = Replacement::Interactive,
            b'd' | b'F' => {}
            _ => return false,
        }
        true
    }

    fn plan(&self, mut paths: Vec<PathBuf>) -> io::Result<Vec<(PathBuf, PathBuf)>> {
        let invalid = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
        if self.relative && !(self.kind == Kind::Symbolic) {
            return Err(invalid("--relative requires --symbolic"));
        }
        if (self.destination == Destination::Direct) && self.target_directory.is_some() {
            return Err(invalid(
                "--target-directory conflicts with --no-target-directory",
            ));
        }
        let directory = if let Some(directory) = &self.target_directory {
            if !directory.is_dir() {
                return Err(invalid(
                    "target directory does not exist or is not a directory",
                ));
            }
            Some(directory.clone())
        } else if paths.len() == 1 && !(self.destination == Destination::Direct) {
            Some(PathBuf::from("."))
        } else if !(self.destination == Destination::Direct)
            && paths.last().is_some_and(|path| {
                path.is_dir()
                    && !(self.no_dereference
                        && fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_symlink()))
            })
        {
            paths.pop()
        } else {
            None
        };
        if let Some(directory) = directory {
            return paths
                .into_iter()
                .map(|source| {
                    let name = source.file_name().unwrap_or(source.as_os_str());
                    let destination = directory.join(name);
                    Ok((source, destination))
                })
                .collect();
        }
        if paths.len() != 2 {
            return Err(invalid(
                "expected a source and destination, or a target directory",
            ));
        }
        let destination = paths.pop().ok_or_else(|| invalid("missing destination"))?;
        let source = paths.pop().ok_or_else(|| invalid("missing source"))?;
        Ok(vec![(source, destination)])
    }

    fn source(&self, source: &Path, target: &Path) -> io::Result<PathBuf> {
        if (self.kind == Kind::Symbolic) && self.relative {
            let cwd = canonical::current_directory()?;
            let mode = Mode {
                existence: Existence::AnyMayBeMissing,
                ..Mode::default()
            };
            let source = canonical::resolve(source, &cwd, mode)?;
            let parent = canonical::resolve(parent(target), &cwd, mode)?;
            return Ok(canonical::relative(&source, &parent).unwrap_or(source));
        }
        if (self.follow == Follow::Logical) && !(self.kind == Kind::Symbolic) {
            return fs::canonicalize(source);
        }
        Ok(source.to_path_buf())
    }

    fn create(&self, source: &Path, target: &Path) -> io::Result<()> {
        if self.kind == Kind::Symbolic {
            symbolic(source, target)
        } else {
            fs::hard_link(source, target)
        }
    }

    fn link(&self, source: &Path, destination: &Path, errors: &mut impl Write) -> io::Result<bool> {
        let original = source;
        let source = self.source(source, destination)?;
        // First try the operation directly. Force replacement prepares the new
        // link before changing an existing destination.
        match self.create(&source, destination) {
            Ok(()) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if fs::symlink_metadata(destination)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "cannot replace a directory with a link",
            ));
        }
        let backup = self.backup(destination)?;
        if matches!(self.replacement, Replacement::Refuse) && backup.is_none() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let identity_source = if self.kind == Kind::Hard && self.follow == Follow::Logical {
            &source
        } else {
            original
        };
        if (self.kind == Kind::Hard || backup.is_none()) && same_entry(identity_source, destination)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source and destination are the same file name",
            ));
        }
        if matches!(self.replacement, Replacement::Interactive) {
            write!(errors, "gutil ln: replace {}? ", super::quote(destination))?;
            errors.flush()?;
            let mut response = String::new();
            io::stdin().lock().read_line(&mut response)?;
            if !response.trim_start().starts_with(['y', 'Y']) {
                return Ok(false);
            }
        }
        let staging = tempfile::Builder::new()
            .prefix(".gutil-link-")
            .tempdir_in(parent(destination))?;
        let staged = staging.path().join("link");
        // Relative symbolic link text is interpreted at its final location.
        if self.kind == Kind::Symbolic {
            symbolic_typed(&source, &staged, destination)?;
        } else {
            fs::hard_link(&source, &staged)?;
        }
        if let Some(backup) = &backup {
            if same_entry(original, backup) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "backup would replace the source file",
                ));
            }
            fs::rename(destination, backup)?;
        }
        install(&staged, destination, backup.as_deref())?;
        Ok(true)
    }

    fn backup(&self, destination: &Path) -> io::Result<Option<PathBuf>> {
        let backup = self.backup.unwrap_or(Backup::Off);
        if matches!(backup, Backup::Off) {
            return Ok(None);
        }
        let mut numbered = 0u64;
        if matches!(backup, Backup::Numbered | Backup::Existing) {
            let mut prefix = destination.file_name().unwrap_or_default().to_os_string();
            prefix.push(".~");
            for entry in fs::read_dir(parent(destination))? {
                let name = entry?.file_name();
                if let Some(number) = name
                    .as_encoded_bytes()
                    .strip_prefix(prefix.as_encoded_bytes())
                    .and_then(|value| value.strip_suffix(b"~"))
                {
                    if let Ok(number) = std::str::from_utf8(number).unwrap_or("").parse::<u64>() {
                        numbered = numbered.max(number);
                    }
                }
            }
        }
        let suffix = if matches!(backup, Backup::Numbered) || numbered != 0 {
            let next = numbered
                .checked_add(1)
                .ok_or_else(|| io::Error::other("backup number overflow"))?;
            OsString::from(format!(".~{next}~"))
        } else {
            self.suffix
                .clone()
                .or_else(|| std::env::var_os("SIMPLE_BACKUP_SUFFIX"))
                .unwrap_or_else(|| "~".into())
        };
        let mut name = destination.as_os_str().to_os_string();
        name.push(suffix);
        let backup = PathBuf::from(name);
        if backup == destination {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup suffix must change the destination name",
            ));
        }
        Ok(Some(backup))
    }
}

fn install(staged: &Path, destination: &Path, backup: Option<&Path>) -> io::Result<()> {
    if let Err(error) = fs::rename(staged, destination) {
        if let Some(backup) = backup {
            // Restore only when no concurrent writer supplied a new name.
            if fs::symlink_metadata(destination)
                .is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
            {
                fs::rename(backup, destination).map_err(|restore| {
                    io::Error::new(
                        error.kind(),
                        format!("{error}; backup restoration failed: {restore}"),
                    )
                })?;
            }
        }
        return Err(error);
    }
    Ok(())
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn same_entry(left: &Path, right: &Path) -> bool {
    if left.file_name() != right.file_name() {
        return false;
    }
    fs::canonicalize(parent(left))
        .ok()
        .zip(fs::canonicalize(parent(right)).ok())
        .is_some_and(|(left, right)| left == right)
}

pub(super) fn execute(
    options: &Options,
    paths: Vec<PathBuf>,
    ui: Presentation,
    output: &mut impl Write,
    errors: &mut impl Write,
    mut progress: Option<&mut ProgressReport>,
) -> io::Result<bool> {
    let mut failed = false;
    for (source, target) in options.plan(paths)? {
        match options.link(&source, &target, errors) {
            Ok(created) if created && options.verbose => writeln!(
                output,
                "{} {} {}",
                super::quote(&target),
                if options.kind == Kind::Symbolic {
                    "->"
                } else {
                    "=>"
                },
                super::quote(&source)
            )?,
            Ok(_) => {}
            Err(error) => {
                ui.error(
                    errors,
                    "gutil ln",
                    &format!(
                        "{} -> {}: {error}",
                        super::quote(&source),
                        super::quote(&target)
                    ),
                )?;
                failed = true;
            }
        }
        if let Some(progress) = &mut progress {
            progress.advance(1, errors);
        }
    }
    Ok(failed)
}

fn symbolic(source: &Path, target: &Path) -> io::Result<()> {
    symbolic_typed(source, target, target)
}

#[cfg(unix)]
fn symbolic_typed(source: &Path, target: &Path, _final_target: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn symbolic_typed(source: &Path, target: &Path, final_target: &Path) -> io::Result<()> {
    let resolved = parent(final_target).join(source);
    let result = if resolved.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)
    } else {
        std::os::windows::fs::symlink_file(source, target)
    };
    result.map_err(windows_link_error)
}

#[cfg(windows)]
fn windows_link_error(error: io::Error) -> io::Error {
    if error.raw_os_error() == Some(1314) {
        io::Error::new(error.kind(), "enable Windows Developer Mode in Windows Settings to create symbolic links without elevation, then retry")
    } else {
        error
    }
}

#[cfg(not(any(unix, windows)))]
fn symbolic_typed(_source: &Path, _target: &Path, _final_target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "symbolic links are unavailable on this platform",
    ))
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn explains_developer_mode_when_windows_denies_symbolic_link_creation() {
        let error = super::windows_link_error(std::io::Error::from_raw_os_error(1314));
        assert!(error.to_string().contains("Windows Developer Mode"));
        assert_eq!(
            super::windows_link_error(std::io::Error::from_raw_os_error(5)).raw_os_error(),
            Some(5)
        );
    }
}

#[cfg(test)]
mod replacement_tests {
    use super::install;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn restores_a_backup_when_installing_the_prepared_link_fails() {
        let fixture = tempfile::tempdir().unwrap();
        let backup = fixture.path().join("backup");
        let destination = fixture.path().join("destination");
        let staged = fixture.path().join("missing-staged-link");
        std::fs::write(&backup, b"original").unwrap();
        assert!(install(&staged, &destination, Some(&backup)).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
        assert!(!backup.exists());
        std::fs::write(&backup, b"original-backup").unwrap();
        std::fs::write(&destination, b"concurrent-write").unwrap();
        assert!(install(&staged, &destination, Some(&backup)).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"concurrent-write");
        assert_eq!(std::fs::read(&backup).unwrap(), b"original-backup");
        assert!(install(&staged, &destination, None).is_err());
        std::fs::remove_file(&destination).unwrap();
        std::fs::remove_file(&backup).unwrap();
        let error = install(&staged, &destination, Some(&backup)).unwrap_err();
        assert!(error.to_string().contains("backup restoration failed"));
    }
}
