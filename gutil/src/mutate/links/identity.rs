use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) struct Info {
    pub(super) identity: (u64, u64),
    pub(super) links: u64,
}

#[derive(PartialEq, Eq, Hash)]
pub(super) struct Key {
    identity: (u64, u64),
    name: PathBuf,
}

pub(super) fn key(path: &Path) -> Option<Key> {
    Some(Key {
        identity: query(path, false).ok()?.identity,
        name: name(path).ok()?,
    })
}

pub(super) fn same_entry(left: &Path, right: &Path) -> bool {
    key(left)
        .zip(key(right))
        .is_some_and(|(left, right)| left == right)
}

pub(super) fn loses_source(source: &Path, destination: &Path, follow_source: bool) -> bool {
    let (Ok(source_info), Ok(destination_info)) =
        (query(source, follow_source), query(destination, false))
    else {
        return false;
    };
    source_info.identity == destination_info.identity
        && (source_info.links == 1
            || name(source)
                .ok()
                .zip(name(destination).ok())
                .is_some_and(|(source, destination)| source == destination))
}

fn name(path: &Path) -> io::Result<PathBuf> {
    if fs::symlink_metadata(path)?.is_symlink() {
        Ok(fs::canonicalize(super::parent(path))?
            .join(path.file_name().unwrap_or(path.as_os_str())))
    } else {
        fs::canonicalize(path)
    }
}

#[cfg(unix)]
fn query(path: &Path, follow: bool) -> io::Result<Info> {
    use std::os::unix::fs::MetadataExt;
    let metadata = if follow {
        fs::metadata(path)?
    } else {
        fs::symlink_metadata(path)?
    };
    Ok(Info {
        identity: (metadata.dev(), metadata.ino()),
        links: metadata.nlink(),
    })
}

#[cfg(windows)]
fn query(path: &Path, follow: bool) -> io::Result<Info> {
    use std::os::windows::fs::OpenOptionsExt;
    // Query the entry without requesting file-data access. Reparse points
    // remain distinct from their targets when tracking destination names.
    const BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let flags = BACKUP_SEMANTICS | if follow { 0 } else { OPEN_REPARSE_POINT };
    let file = fs::OpenOptions::new()
        .access_mode(0)
        .custom_flags(flags)
        .open(path)?;
    let info = winapi_util::file::information(file)?;
    Ok(Info {
        identity: (info.volume_serial_number(), info.file_index()),
        links: info.number_of_links(),
    })
}

#[cfg(not(any(unix, windows)))]
fn query(_path: &Path, _follow: bool) -> io::Result<Info> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file identity is unavailable on this platform",
    ))
}
