#[cfg(any(target_os = "linux", windows))]
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

    pub(super) fn create(
        &mut self,
        entry: &Entry<'_>,
        permissions: u32,
        mask: u32,
    ) -> io::Result<()> {
        // The command has one ordered filesystem worker, and every user-file
        // creation goes through it. Keep the selected mask until the next
        // change, avoiding two extra syscalls for each ordinary directory.
        // Let the kernel apply inherited ACLs instead of pre-masking the mode.
        if mask != self.current {
            #[cfg(unix)]
            umask(native_mode(mask));
            self.current = mask;
        }
        entry.create(permissions)
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

// Keep the parent directory open from creation through permission adjustment.
// A renamed or replaced ancestor cannot redirect a later chmod to another tree.
#[cfg(unix)]
pub(super) struct Entry<'a> {
    parent: Option<std::os::fd::OwnedFd>,
    name: &'a std::ffi::OsStr,
}

#[cfg(unix)]
impl<'a> Entry<'a> {
    // Inherited-mode creation never performs chmod. Resolve that single mkdirat
    // normally rather than opening a parent that no later operation needs.
    pub(super) fn without_adjustment(path: &'a Path) -> Self {
        Self {
            parent: None,
            name: path.as_os_str(),
        }
    }

    pub(super) fn new(path: &'a Path) -> io::Result<Self> {
        use nix::fcntl::{openat, OFlag, AT_FDCWD};
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        let end = bytes
            .iter()
            .rposition(|byte| *byte != b'/')
            .map_or(0, |index| index + 1);
        let trimmed = &bytes[..end];
        let (parent, name) = if !bytes.is_empty() && trimmed.is_empty() {
            (Some(path.as_os_str()), std::ffi::OsStr::new("."))
        } else if let Some(index) = trimmed.iter().rposition(|byte| *byte == b'/') {
            (
                Some(std::ffi::OsStr::from_bytes(&trimmed[..=index])),
                std::ffi::OsStr::from_bytes(&trimmed[index + 1..]),
            )
        } else {
            (None, std::ffi::OsStr::from_bytes(trimmed))
        };
        #[cfg(target_os = "linux")]
        let access = OFlag::O_PATH;
        #[cfg(target_vendor = "apple")]
        let access = OFlag::O_SEARCH;
        #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
        let access = OFlag::O_RDONLY;
        let parent = parent
            .map(|path| {
                openat(
                    AT_FDCWD,
                    path,
                    access | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
                    Mode::empty(),
                )
            })
            .transpose()?;
        Ok(Self { parent, name })
    }

    fn parent(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.parent
            .as_ref()
            .map_or(nix::fcntl::AT_FDCWD, AsFd::as_fd)
    }

    pub(super) fn create(&self, permissions: u32) -> io::Result<()> {
        nix::sys::stat::mkdirat(self.parent(), self.name, native_mode(permissions))
            .map_err(io::Error::from)
    }

    pub(super) fn is_dir(&self) -> bool {
        self.metadata(nix::fcntl::AtFlags::empty())
            .is_ok_and(|metadata| {
                nix::sys::stat::SFlag::from_bits_truncate(metadata.st_mode)
                    == nix::sys::stat::SFlag::S_IFDIR
            })
    }

    fn metadata(&self, flags: nix::fcntl::AtFlags) -> io::Result<nix::sys::stat::FileStat> {
        nix::sys::stat::fstatat(self.parent(), self.name, flags).map_err(io::Error::from)
    }

    #[allow(
        clippy::unnecessary_cast,
        clippy::cast_lossless,
        reason = "mode_t is u32 on Linux and u16 on macOS."
    )]
    pub(super) fn permissions(&self) -> io::Result<u32> {
        let metadata = self.metadata(nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW)?;
        if nix::sys::stat::SFlag::from_bits_truncate(metadata.st_mode)
            != nix::sys::stat::SFlag::S_IFDIR
        {
            return Err(io::Error::other("created directory was replaced"));
        }
        Ok(metadata.st_mode as u32 & 0o7777)
    }

    pub(super) fn set_permissions(&self, permissions: u32) -> io::Result<()> {
        nix::sys::stat::fchmodat(
            self.parent(),
            self.name,
            native_mode(permissions),
            nix::sys::stat::FchmodatFlags::NoFollowSymlink,
        )
        .map_err(io::Error::from)
    }
}

#[cfg(windows)]
pub(super) struct Entry<'a>(&'a Path);

#[cfg(windows)]
impl<'a> Entry<'a> {
    pub(super) fn without_adjustment(path: &'a Path) -> Self {
        Self(path)
    }

    #[allow(
        clippy::unnecessary_wraps,
        reason = "Opening a parent directory can fail on Unix."
    )]
    pub(super) fn new(path: &'a Path) -> io::Result<Self> {
        Ok(Self(path))
    }
    fn create(&self, _permissions: u32) -> io::Result<()> {
        fs::create_dir(self.0)
    }
    pub(super) fn is_dir(&self) -> bool {
        self.0.is_dir()
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "The capability interface is fallible on Linux and shared across native platforms."
)]
pub(super) fn active_labels() -> io::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        active_labels_at(
            Path::new("/sys/fs/selinux/enforce"),
            Path::new("/proc/self/attr/smack/current"),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn active_labels_at(selinux: &Path, smack: &Path) -> io::Result<bool> {
    if selinux.try_exists()? {
        return Ok(true);
    }
    match fs::read_to_string(smack) {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
            ) =>
        {
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(true),
        Err(error) => Err(error),
    }
}
