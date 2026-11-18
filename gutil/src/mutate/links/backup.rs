use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum Policy {
    Off,
    Simple,
    Numbered,
    Existing,
}

impl Policy {
    pub(in super::super) fn parse(value: &OsStr) -> Option<Self> {
        match value.as_encoded_bytes() {
            b"none" | b"off" => Some(Self::Off),
            b"simple" | b"never" => Some(Self::Simple),
            b"numbered" | b"t" => Some(Self::Numbered),
            b"existing" | b"nil" => Some(Self::Existing),
            _ => None,
        }
    }
}

pub(super) struct Plan {
    path: PathBuf,
    number: Option<u64>,
}

impl Plan {
    pub(super) fn new(
        destination: &Path,
        policy: Option<Policy>,
        suffix: Option<&OsStr>,
    ) -> io::Result<Option<Self>> {
        let policy = policy.unwrap_or(Policy::Off);
        if policy == Policy::Off {
            return Ok(None);
        }
        let mut numbered = 0u64;
        if matches!(policy, Policy::Numbered | Policy::Existing) {
            let mut prefix = destination.file_name().unwrap_or_default().to_os_string();
            prefix.push(".~");
            for entry in fs::read_dir(super::parent(destination))? {
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
        if policy == Policy::Numbered || numbered != 0 {
            let number = numbered
                .checked_add(1)
                .ok_or_else(|| io::Error::other("backup number overflow"))?;
            return Ok(Some(Self {
                path: numbered_path(destination, number),
                number: Some(number),
            }));
        }
        let suffix = suffix
            .map(OsStr::to_os_string)
            .or_else(|| std::env::var_os("SIMPLE_BACKUP_SUFFIX"))
            .unwrap_or_else(|| "~".into());
        let mut path = destination.as_os_str().to_os_string();
        path.push(suffix);
        let path = PathBuf::from(path);
        if path == destination {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "backup suffix must change the destination name",
            ));
        }
        Ok(Some(Self { path, number: None }))
    }

    pub(super) fn save(mut self, source: &Path, destination: &Path) -> io::Result<PathBuf> {
        loop {
            if super::same_entry(source, &self.path) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "backup would replace the source file",
                ));
            }
            let result = if self.number.is_some() {
                move_no_replace(destination, &self.path)
            } else {
                fs::rename(destination, &self.path)
            };
            match result {
                Ok(()) => return Ok(self.path),
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists && self.number.is_some() =>
                {
                    let next = self
                        .number
                        .and_then(|number| number.checked_add(1))
                        .ok_or_else(|| io::Error::other("backup number overflow"))?;
                    self.path = numbered_path(destination, next);
                    self.number = Some(next);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

fn numbered_path(destination: &Path, number: u64) -> PathBuf {
    let mut path = destination.as_os_str().to_os_string();
    path.push(format!(".~{number}~"));
    path.into()
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
pub(super) fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{renameat_with, RenameFlags, CWD};
    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(windows)]
pub(super) fn move_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    let entry = |path: &Path| -> io::Result<PathBuf> {
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing file name"))?;
        Ok(fs::canonicalize(super::parent(path))?.join(name))
    };
    renamore::rename_exclusive(entry(source)?, entry(destination)?)
}

#[cfg(not(any(target_os = "linux", target_vendor = "apple", windows)))]
pub(super) fn move_no_replace(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace renaming is unavailable on this platform",
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    #[allow(clippy::unwrap_used)]
    fn retries_a_number_claimed_after_the_backup_plan_was_created() {
        let fixture = tempfile::tempdir().unwrap();
        let destination = fixture.path().join("target");
        let source = fixture.path().join("source");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&destination, b"original").unwrap();
        let plan = super::Plan::new(&destination, Some(super::Policy::Numbered), None)
            .unwrap()
            .unwrap();
        let collision = fixture.path().join("target.~1~");
        std::fs::write(&collision, b"concurrent").unwrap();
        let result = plan.save(&source, &destination).unwrap();
        assert_eq!(result, fixture.path().join("target.~2~"));
        assert_eq!(std::fs::read(result).unwrap(), b"original");
        assert_eq!(std::fs::read(collision).unwrap(), b"concurrent");
        assert!(!destination.exists());
    }
}
