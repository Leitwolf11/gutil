use std::fs;
use std::io;
use std::path::Path;

pub(super) fn install(staged: &Path, destination: &Path, backup: Option<&Path>) -> io::Result<()> {
    if let Err(error) = fs::rename(staged, destination) {
        if let Some(backup) = backup {
            if let Err(restore) = super::backup::move_no_replace(backup, destination) {
                let message = if restore.kind() == io::ErrorKind::AlreadyExists {
                    format!("{error}; backup restoration conflict: destination exists; backup retained at {}", backup.display())
                } else {
                    format!(
                        "{error}; backup restoration failed: {restore}; backup retained at {}",
                        backup.display()
                    )
                };
                return Err(io::Error::new(error.kind(), message));
            }
        }
        return Err(error);
    }
    Ok(())
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
