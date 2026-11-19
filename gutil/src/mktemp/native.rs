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
