use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub(super) struct Failure {
    pub(super) error: io::Error,
    pub(super) retained: Option<PathBuf>,
}

pub(super) fn install(
    staged: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> Result<(), Failure> {
    if let Err(error) = fs::rename(staged, destination) {
        #[cfg(windows)]
        if backup.is_none() {
            if let Some(result) = replace_directory_link(staged, destination) {
                return result;
            }
        }
        return Err(match backup {
            Some(backup) => restore(error, backup, destination),
            None => Failure {
                error,
                retained: None,
            },
        });
    }
    Ok(())
}

fn restore(error: io::Error, backup: &Path, destination: &Path) -> Failure {
    match super::backup::move_no_replace(backup, destination) {
        Ok(()) => Failure {
            error,
            retained: None,
        },
        Err(restore) => {
            let message = if restore.kind() == io::ErrorKind::AlreadyExists {
                format!("{error}; backup restoration conflict: destination exists; backup retained at {}", backup.display())
            } else {
                format!(
                    "{error}; backup restoration failed: {restore}; backup retained at {}",
                    backup.display()
                )
            };
            Failure {
                error: io::Error::new(error.kind(), message),
                retained: Some(backup.to_path_buf()),
            }
        }
    }
}

#[cfg(windows)]
fn replace_directory_link(staged: &Path, destination: &Path) -> Option<Result<(), Failure>> {
    use std::os::windows::fs::FileTypeExt;
    let staged_type = fs::symlink_metadata(staged).ok()?.file_type();
    let destination_type = fs::symlink_metadata(destination).ok()?.file_type();
    if destination_type.is_dir()
        || staged_type.is_symlink_dir() == destination_type.is_symlink_dir()
    {
        return None;
    }
    let previous = staged.with_file_name("previous");
    if let Err(error) = super::backup::move_no_replace(destination, &previous) {
        return Some(Err(Failure {
            error,
            retained: None,
        }));
    }
    // Verify the moved entry, since the destination could have changed after
    // the first metadata query. A real directory must never be replaced.
    match fs::symlink_metadata(&previous) {
        Ok(metadata) if !metadata.is_dir() => Some(install(staged, destination, Some(&previous))),
        Ok(_) => Some(Err(restore(
            io::Error::new(
                io::ErrorKind::IsADirectory,
                "cannot replace a directory with a link",
            ),
            &previous,
            destination,
        ))),
        Err(error) => Some(Err(restore(error, &previous, destination))),
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
        let conflict = install(&staged, &destination, Some(&backup)).unwrap_err();
        assert!(conflict.to_string().contains("backup restoration conflict"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"concurrent-write");
        assert_eq!(std::fs::read(&backup).unwrap(), b"original-backup");
        assert!(install(&staged, &destination, None).is_err());
        std::fs::remove_file(&destination).unwrap();
        std::fs::remove_file(&backup).unwrap();
        let error = install(&staged, &destination, Some(&backup)).unwrap_err();
        assert!(error.to_string().contains("backup restoration failed"));
    }
}
