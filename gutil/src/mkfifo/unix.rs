use std::ffi::OsStr;
use std::io::{self, stderr};
use std::path::{Path, PathBuf};

use nix::fcntl::{AtFlags, AT_FDCWD};
use nix::sys::stat::{fstatat, umask, FileStat, Mode, SFlag};
use nix::unistd::mkfifo;

use super::LABEL;
use crate::mkdir::mode::{InvalidMode, Mode as PermissionExpr};
use crate::presentation::Presentation;

struct CreationMask(Mode);

impl Drop for CreationMask {
    fn drop(&mut self) {
        umask(self.0);
    }
}

#[allow(
    clippy::unnecessary_cast,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    reason = "Unix mode_t varies by platform. FIFO permission bits fit every supported mode_t."
)]
fn prepare(mode: &PermissionExpr) -> Result<(Mode, CreationMask), InvalidMode> {
    // This ordered worker is the only thread creating filesystem objects.
    // Apply the requested mode at creation and let the kernel inherit ACLs.
    let mask = CreationMask(umask(Mode::empty()));
    let bits = mode.fifo_permissions(mask.0.bits() as u32)?;
    Ok((Mode::from_bits_truncate(bits as _), mask))
}

pub(super) fn create_all(
    paths: &[PathBuf],
    requested: Option<&PermissionExpr>,
    ui: Presentation,
) -> u8 {
    let mut errors = stderr().lock();
    let explicit = match requested.map(prepare).transpose() {
        Ok(permissions) => permissions,
        Err(error) => {
            let _ = ui.error(&mut errors, LABEL, &error.to_string());
            return 1;
        }
    };
    let mode = explicit.as_ref().map(|(mode, _mask)| *mode);
    let mut progress = ui.progress(LABEL, "FIFOs", true);
    let mut status = 0;
    for path in paths {
        let result = match mode {
            Some(permissions) => create_with_permissions(path, permissions),
            None => mkfifo(path, Mode::from_bits_truncate(0o666)).map_err(io::Error::from),
        };
        if let Err(error) = result {
            status = 1;
            let _ = ui.error(&mut errors, LABEL, &format!("{}: {error}", path.display()));
        }
        if let Some(progress) = &mut progress {
            progress.advance(1, &mut errors);
        }
    }
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    status
}

#[cfg(target_os = "linux")]
fn create_with_permissions(path: &Path, permissions: Mode) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;

    use nix::fcntl::{openat, OFlag};
    use nix::unistd::mkfifoat;

    let bytes = path.as_os_str().as_bytes();
    let (parent, name) = if let Some(slash) = bytes.iter().rposition(|&byte| byte == b'/') {
        let (parent, name) = bytes.split_at(slash + 1);
        let parent = openat(
            AT_FDCWD,
            OsStr::from_bytes(parent),
            OFlag::O_DIRECTORY | OFlag::O_CLOEXEC | OFlag::O_PATH,
            Mode::empty(),
        )?;
        (Some(parent), OsStr::from_bytes(name))
    } else {
        (None, path.as_os_str())
    };
    let directory = parent.as_ref().map_or(AT_FDCWD, std::os::fd::AsFd::as_fd);
    mkfifoat(directory, name, permissions)?;
    set_permissions(directory, name, permissions)
}

#[cfg(not(target_os = "linux"))]
fn create_with_permissions(path: &Path, permissions: Mode) -> io::Result<()> {
    mkfifo(path, permissions)?;
    set_permissions(AT_FDCWD, path.as_os_str(), permissions)
}

fn set_permissions(
    directory: std::os::fd::BorrowedFd<'_>,
    name: &OsStr,
    permissions: Mode,
) -> io::Result<()> {
    let metadata = fstatat(directory, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if SFlag::from_bits_truncate(metadata.st_mode) != SFlag::S_IFIFO {
        return Err(io::Error::other("created FIFO was replaced"));
    }
    if Mode::from_bits_truncate(metadata.st_mode) != permissions {
        adjust_permissions(directory, name, &metadata, permissions)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn adjust_permissions(
    directory: std::os::fd::BorrowedFd<'_>,
    name: &OsStr,
    metadata: &FileStat,
    permissions: Mode,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;

    use nix::fcntl::{openat, OFlag};
    use nix::sys::stat::fstat;

    let fifo = openat(
        directory,
        name,
        OFlag::O_PATH | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let pinned = fstat(&fifo)?;
    if pinned.st_dev != metadata.st_dev || pinned.st_ino != metadata.st_ino {
        return Err(io::Error::other("created FIFO was replaced"));
    }
    // An O_PATH descriptor pins the inode without opening a FIFO endpoint.
    // The procfs descriptor link allows chmod even when the FIFO mode is 000.
    let descriptor = format!("/proc/self/fd/{}", fifo.as_raw_fd());
    std::fs::set_permissions(
        descriptor,
        std::fs::Permissions::from_mode(permissions.bits()),
    )
}

#[cfg(not(target_os = "linux"))]
fn adjust_permissions(
    directory: std::os::fd::BorrowedFd<'_>,
    name: &OsStr,
    _metadata: &FileStat,
    permissions: Mode,
) -> io::Result<()> {
    use nix::sys::stat::{fchmodat, FchmodatFlags};

    fchmodat(directory, name, permissions, FchmodatFlags::NoFollowSymlink).map_err(io::Error::from)
}

#[cfg(all(test, target_os = "linux"))]
#[allow(clippy::unwrap_used)]
#[test]
fn permission_adjustment_rejects_replaced_objects() {
    let directory = tempfile::tempdir().unwrap();
    let ordinary = directory.path().join("file");
    std::fs::write(&ordinary, b"original").unwrap();
    let mode = Mode::from_bits_truncate(0o600);
    assert!(set_permissions(AT_FDCWD, ordinary.as_os_str(), mode).is_err());

    let checked = directory.path().join("checked");
    let replacement = directory.path().join("replacement");
    mkfifo(&checked, mode).unwrap();
    mkfifo(&replacement, mode).unwrap();
    let metadata = fstatat(AT_FDCWD, &checked, AtFlags::AT_SYMLINK_NOFOLLOW).unwrap();
    assert!(adjust_permissions(AT_FDCWD, replacement.as_os_str(), &metadata, mode).is_err());
    assert_eq!(std::fs::read(ordinary).unwrap(), b"original");
}
