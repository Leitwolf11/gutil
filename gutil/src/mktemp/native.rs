use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
pub(super) type Unit = u8;
#[cfg(windows)]
pub(super) type Unit = u16;

pub(super) fn separator(unit: Unit) -> bool {
    unit == 47 || (cfg!(windows) && unit == 92)
}

#[cfg(unix)]
pub(super) fn units(value: &OsStr) -> Vec<Unit> {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes().to_vec()
}

#[cfg(windows)]
pub(super) fn units(value: &OsStr) -> Vec<Unit> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().collect()
}

#[cfg(unix)]
pub(super) fn string(value: &[Unit]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(value.to_vec())
}

#[cfg(windows)]
pub(super) fn string(value: &[Unit]) -> OsString {
    use std::os::windows::ffi::OsStringExt;
    OsString::from_wide(value)
}

pub(super) fn suffix(value: &OsStr, offset: usize) -> OsString {
    string(&units(value)[offset..])
}

pub(super) fn write_path(output: &mut impl Write, path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.write_all(path.as_os_str().as_bytes())?;
    }
    #[cfg(windows)]
    output.write_all(path.as_os_str().to_string_lossy().as_bytes())?;
    output.write_all(b"\n")
}

#[cfg(windows)]
pub(super) fn validate_missing_parent(path: &Path) -> io::Result<()> {
    // Windows can report a missing path even when an ancestor is a file.
    // A dry run may use absent directories, but cannot traverse a file.
    for parent in path.ancestors().skip(1) {
        match std::fs::metadata(parent) {
            Ok(metadata) if metadata.is_dir() => return Ok(()),
            Ok(_) => return Err(io::Error::from(io::ErrorKind::NotADirectory)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn validate_disk_file(file: &std::fs::File) -> io::Result<()> {
    if winapi_util::file::typ(file)?.is_disk() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "temporary names must refer to disk files",
        ))
    }
}
