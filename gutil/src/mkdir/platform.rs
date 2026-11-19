use std::fs;
use std::io;
use std::path::Path;

#[cfg(unix)]
use nix::sys::stat::{umask, Mode};

pub(super) struct Mask {
    original: u32,
    current: u32,
}

impl Mask {
    #[allow(
        clippy::unnecessary_cast,
        clippy::cast_lossless,
        reason = "mode_t is u32 on Linux and u16 on macOS."
    )]
    pub(super) fn capture() -> Self {
        // Dispatch has not started a runtime or filesystem workers yet.
        #[cfg(unix)]
        let original = {
            let previous = umask(Mode::empty());
            umask(previous);
            previous.bits() as u32
        };
        #[cfg(windows)]
        let original = 0;
        Self {
            original,
            current: original,
        }
    }

    pub(super) fn original(&self) -> u32 {
        self.original
    }

    pub(super) fn create(&mut self, path: &Path, permissions: u32, mask: u32) -> io::Result<()> {
        // The command has one ordered filesystem worker, and every user-file
        // creation goes through it. Keep the selected mask until the next
        // change, avoiding two extra syscalls for each ordinary directory.
        // Let the kernel apply inherited ACLs instead of pre-masking the mode.
        if mask != self.current {
            #[cfg(unix)]
            umask(native_mode(mask));
            self.current = mask;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(permissions).create(path)
        }
        #[cfg(windows)]
        {
            let _ = permissions;
            fs::create_dir(path)
        }
    }
}

#[cfg(unix)]
impl Drop for Mask {
    fn drop(&mut self) {
        if self.current != self.original {
            umask(native_mode(self.original));
        }
    }
}

#[cfg(unix)]
#[allow(
    clippy::cast_possible_truncation,
    reason = "Only the low twelve permission bits are converted to the platform mode_t."
)]
fn native_mode(bits: u32) -> Mode {
    Mode::from_bits_truncate((bits & 0o7777) as _)
}

#[cfg(unix)]
pub(super) fn permissions(path: &Path) -> io::Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::other("created directory was replaced"));
    }
    Ok(metadata.permissions().mode() & 0o7777)
}

#[cfg(unix)]
pub(super) fn set_permissions(path: &Path, permissions: u32) -> io::Result<()> {
    use nix::fcntl::AT_FDCWD;
    use nix::sys::stat::{fchmodat, FchmodatFlags};
    fchmodat(
        AT_FDCWD,
        path,
        native_mode(permissions),
        FchmodatFlags::NoFollowSymlink,
    )
    .map_err(io::Error::from)
}

pub(super) fn active_labels() -> io::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        if Path::new("/sys/fs/selinux/enforce").try_exists()? {
            return Ok(true);
        }
        match fs::read_to_string("/proc/self/attr/smack/current") {
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound
                        | io::ErrorKind::InvalidInput
                        | io::ErrorKind::Unsupported
                ) =>
            {
                Ok(false)
            }
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(true),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(false)
    }
}
