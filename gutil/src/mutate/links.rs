use std::ffi::OsString;
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

pub(super) use backup::Policy as Backup;

mod backup;
mod identity;
mod replace;

use replace::install;

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
        if self.relative && self.kind != Kind::Symbolic {
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
        if self.follow == Follow::Logical && self.kind != Kind::Symbolic {
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

    fn link(
        &self,
        source: &Path,
        destination: &Path,
        errors: &mut impl Write,
    ) -> io::Result<Option<PathBuf>> {
        let original = source;
        let source = self.source(source, destination)?;
        // First try the operation directly. Force replacement prepares the new
        // link before changing an existing destination.
        match self.create(&source, destination) {
            Ok(()) => {
                return Ok(Some(if self.verbose && self.kind == Kind::Hard {
                    original.to_path_buf()
                } else {
                    source
                }))
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        if fs::symlink_metadata(destination)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "cannot replace a directory with a link",
            ));
        }
        let backup = backup::Plan::new(destination, self.backup, self.suffix.as_deref())?;
        if matches!(self.replacement, Replacement::Refuse) && backup.is_none() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        let follows_source = self.kind == Kind::Symbolic || self.follow == Follow::Logical;
        if (self.kind == Kind::Hard || backup.is_none())
            && identity::loses_source(original, destination, follows_source)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source and destination refer to the same file",
            ));
        }
        if matches!(self.replacement, Replacement::Interactive) {
            write!(errors, "gutil ln: replace {}? ", super::quote(destination))?;
            errors.flush()?;
            let mut response = String::new();
            io::stdin().lock().read_line(&mut response)?;
            if !response.trim_start().starts_with(['y', 'Y']) {
                return Ok(None);
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
        let backup = backup
            .map(|plan| plan.save(original, destination))
            .transpose()?;
        install(&staged, destination, backup.as_deref())?;
        Ok(Some(if self.verbose && self.kind == Kind::Hard {
            original.to_path_buf()
        } else {
            source
        }))
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn same_entry(left: &Path, right: &Path) -> bool {
    identity::same_entry(left, right)
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
    let plan = options.plan(paths)?;
    let protect_created = plan.len() > 1
        && options.kind == Kind::Hard
        && matches!(options.replacement, Replacement::Force)
        && options.backup != Some(Backup::Numbered);
    let mut created = rustc_hash::FxHashSet::default();
    for (source, target) in plan {
        let result = if protect_created
            && identity::key(&target).is_some_and(|key| created.contains(&key))
        {
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "will not overwrite a destination created earlier in this invocation",
            ))
        } else {
            options.link(&source, &target, errors)
        };
        match result {
            Ok(Some(text)) => {
                if protect_created {
                    if let Some(key) = identity::key(&target) {
                        created.insert(key);
                    }
                }
                if options.verbose {
                    writeln!(
                        output,
                        "{} {} {}",
                        super::quote(&target),
                        if options.kind == Kind::Symbolic {
                            "->"
                        } else {
                            "=>"
                        },
                        super::quote(&text)
                    )?;
                }
            }
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
