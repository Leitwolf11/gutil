use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Local,
    Network,
    Fuse,
    Special,
    Loops,
    Binds,
}

impl Kind {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Network => "network",
            Self::Fuse => "fuse",
            Self::Special => "special",
            Self::Loops => "loops",
            Self::Binds => "binds",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "network" => Some(Self::Network),
            "fuse" => Some(Self::Fuse),
            "special" => Some(Self::Special),
            "loops" => Some(Self::Loops),
            "binds" => Some(Self::Binds),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct Mount {
    pub(super) path: PathBuf,
    pub(super) device: OsString,
    pub(super) kind: Kind,
    pub(super) filesystem: OsString,
    pub(super) options: OsString,
    pub(super) identity: Option<OsString>,
    #[cfg(not(target_os = "linux"))]
    usage: Usage,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Usage {
    pub(super) size: u64,
    pub(super) used: u64,
    pub(super) available: u64,
    pub(super) inodes: u64,
    pub(super) inodes_used: u64,
    pub(super) inodes_free: u64,
    pub(super) blocks: u64,
    pub(super) block_size: u64,
}

impl Usage {
    pub(super) fn sum(self, other: Self) -> Self {
        Self {
            size: self.size.saturating_add(other.size),
            used: self.used.saturating_add(other.used),
            available: self.available.saturating_add(other.available),
            inodes: self.inodes.saturating_add(other.inodes),
            inodes_used: self.inodes_used.saturating_add(other.inodes_used),
            inodes_free: self.inodes_free.saturating_add(other.inodes_free),
            ..Self::default()
        }
    }
}

fn kind(filesystem: &str, device: &Path, bound: bool) -> Kind {
    if matches!(
        filesystem,
        "nfs"
            | "nfs4"
            | "cifs"
            | "smb3"
            | "smbfs"
            | "sshfs"
            | "fuse.sshfs"
            | "9p"
            | "afpfs"
            | "ceph"
    ) {
        Kind::Network
    } else if filesystem.starts_with("fuse") && filesystem != "fuseblk" {
        Kind::Fuse
    } else if device
        .as_os_str()
        .as_encoded_bytes()
        .starts_with(b"/dev/loop")
    {
        Kind::Loops
    } else if matches!(
        filesystem,
        "proc"
            | "sysfs"
            | "tmpfs"
            | "ramfs"
            | "devtmpfs"
            | "devpts"
            | "cgroup"
            | "cgroup2"
            | "securityfs"
            | "efivarfs"
            | "debugfs"
            | "tracefs"
            | "autofs"
            | "bpf"
            | "pstore"
            | "configfs"
            | "mqueue"
            | "hugetlbfs"
            | "binfmt_misc"
            | "nsfs"
            | "fusectl"
    ) {
        Kind::Special
    } else if bound {
        Kind::Binds
    } else {
        Kind::Local
    }
}

#[cfg(target_os = "linux")]
pub(super) fn mounts() -> io::Result<Vec<Mount>> {
    let mut mounts = Vec::new();
    let data = std::fs::read("/proc/self/mountinfo")?;
    for line in data
        .split(|&byte| byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        if let Some(mount) = parse_mount(line) {
            mounts.push(mount);
        }
    }
    Ok(mounts)
}

#[cfg(target_os = "linux")]
fn parse_mount(line: &[u8]) -> Option<Mount> {
    let separator = memchr::memmem::find(line, b" - ")?;
    let mut before = line[..separator].split(|&byte| byte == b' ').skip(2);
    let identity = native(before.next()?);
    let root = before.next()?;
    let path = PathBuf::from(native(before.next()?));
    let options = native(before.next()?);
    let mut after = line[separator + 3..].split(|&byte| byte == b' ');
    let filesystem = native(after.next()?);
    let device = native(after.next()?);
    let class = kind(
        &filesystem.to_string_lossy(),
        Path::new(&device),
        root != b"/" && filesystem != "btrfs",
    );
    Some(Mount {
        path,
        device,
        kind: class,
        filesystem,
        options,
        identity: Some(identity),
    })
}

#[cfg(target_os = "linux")]
fn native(mut bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    let mut path = Vec::with_capacity(bytes.len());
    while let Some(offset) = memchr::memchr(b'\\', bytes) {
        let remainder = &bytes[offset..];
        path.extend_from_slice(&bytes[..offset]);
        if let [b'\\', a @ b'0'..=b'3', b @ b'0'..=b'7', c @ b'0'..=b'7', tail @ ..] = remainder {
            path.push((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'));
            bytes = tail;
        } else {
            path.push(b'\\');
            bytes = &remainder[1..];
        }
    }
    path.extend_from_slice(bytes);
    OsString::from_vec(path)
}

#[cfg(target_os = "linux")]
pub(super) fn usage(mount: &Mount) -> io::Result<Usage> {
    let info = rustix::fs::statvfs(&mount.path)?;
    let block_size = info.f_frsize.max(1);
    Ok(Usage {
        size: info.f_blocks.saturating_mul(block_size),
        used: info
            .f_blocks
            .saturating_sub(info.f_bfree)
            .saturating_mul(block_size),
        available: info.f_bavail.saturating_mul(block_size),
        inodes: info.f_files,
        inodes_used: info.f_files.saturating_sub(info.f_ffree),
        inodes_free: info.f_ffree,
        blocks: info.f_blocks,
        block_size,
    })
}

#[cfg(not(target_os = "linux"))]
pub(super) fn mounts() -> io::Result<Vec<Mount>> {
    use sysinfo::{DiskRefreshKind, Disks};

    let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage())
        .map_err(io::Error::other)?;
    Ok(disks
        .iter()
        .map(|disk| {
            let path = disk.mount_point().to_path_buf();
            let device = disk.name().to_os_string();
            let filesystem = disk.file_system().to_os_string();
            let class = kind(&filesystem.to_string_lossy(), Path::new(&device), false);
            Mount {
                path,
                device,
                kind: class,
                filesystem,
                options: if disk.is_read_only() { "ro" } else { "rw" }.into(),
                identity: disk.id().map(|id| id.to_string().into()),
                usage: Usage {
                    size: disk.total_space(),
                    used: disk.total_space().saturating_sub(disk.available_space()),
                    available: disk.available_space(),
                    ..Usage::default()
                },
            }
        })
        .collect())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn usage(mount: &Mount) -> io::Result<Usage> {
    if mount.path.as_os_str().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty mount path",
        ));
    }
    Ok(mount.usage)
}

pub(super) fn sync() {
    #[cfg(target_os = "linux")]
    rustix::fs::sync();
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::{parse_mount, Kind};

    #[test]
    #[allow(clippy::unwrap_used)]
    fn parses_linux_mount_fields_and_escaped_names() {
        let mount = parse_mount(
            b"42 24 8:1 /root /with\\040space\\011tab\\134slash rw shared:1 - ext4 /dev/disk rw",
        )
        .unwrap();
        assert_eq!(mount.path.as_os_str(), "/with space\ttab\\slash");
        assert_eq!(mount.identity.as_deref(), Some(std::ffi::OsStr::new("8:1")));
        assert_eq!(mount.device, "/dev/disk");
        assert_eq!(mount.options, "rw");
        assert_eq!(mount.kind, Kind::Binds);
        let literal = parse_mount(b"4 2 0:4 / /literal\\xyz rw - tmpfs tmpfs rw").unwrap();
        assert_eq!(literal.path.as_os_str(), "/literal\\xyz");
        assert!(parse_mount(b"missing separator").is_none());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn classifies_network_fuse_and_loop_devices() {
        for (line, kind) in [
            (
                &b"42 24 0:4 / /data rw - nfs4 host:/data rw"[..],
                Kind::Network,
            ),
            (
                &b"42 24 0:4 / /data rw - fuse.rclone remote rw"[..],
                Kind::Fuse,
            ),
            (
                &b"42 24 0:4 / /data ro - squashfs /dev/loop0 ro"[..],
                Kind::Loops,
            ),
            (
                &b"42 24 0:4 /vol /data rw - btrfs /dev/disk rw"[..],
                Kind::Local,
            ),
        ] {
            assert_eq!(parse_mount(line).unwrap().kind, kind);
        }
    }
}
