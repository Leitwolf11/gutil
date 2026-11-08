use std::fs::Metadata;
use std::io::{self, Write};

#[cfg(unix)]
use std::os::unix::fs::{FileTypeExt, MetadataExt};

pub(super) fn write_total<'a>(
    output: &mut impl Write,
    entries: impl Iterator<Item = &'a Metadata>,
) -> io::Result<()> {
    #[cfg(unix)]
    let blocks = entries
        .map(MetadataExt::blocks)
        .fold(0, u64::saturating_add);
    #[cfg(not(unix))]
    let blocks = entries
        .map(Metadata::len)
        .fold(0, u64::saturating_add)
        .div_ceil(512);

    writeln!(output, "total {}", blocks.div_ceil(2))
}

pub(super) fn write_metadata(output: &mut impl Write, metadata: &Metadata) -> io::Result<()> {
    let kind = file_kind(metadata);
    let timestamp = metadata
        .modified()
        .ok()
        .and_then(|time| jiff::Timestamp::try_from(time).ok());
    let timestamp = timestamp.map_or_else(
        || String::from("?"),
        |time| time.strftime("%F %RZ").to_string(),
    );

    #[cfg(unix)]
    {
        let mode = permissions(metadata.mode());
        write!(
            output,
            "{kind}{mode} {} {} {} {} {timestamp} ",
            metadata.nlink(),
            metadata.uid(),
            metadata.gid(),
            metadata.len()
        )
    }
    #[cfg(not(unix))]
    {
        let mode = if metadata.permissions().readonly() {
            "r--------"
        } else {
            "rw-------"
        };
        write!(output, "{kind}{mode} - - - {} {timestamp} ", metadata.len())
    }
}

fn file_kind(metadata: &Metadata) -> char {
    let kind = metadata.file_type();
    if kind.is_dir() {
        return 'd';
    }
    if kind.is_symlink() {
        return 'l';
    }
    #[cfg(unix)]
    {
        if kind.is_block_device() {
            return 'b';
        }
        if kind.is_char_device() {
            return 'c';
        }
        if kind.is_fifo() {
            return 'p';
        }
        if kind.is_socket() {
            return 's';
        }
    }
    '-'
}

#[cfg(unix)]
fn permissions(mode: u32) -> String {
    let mut result = String::with_capacity(9);
    for shift in [6, 3, 0] {
        result.push(if mode & (0o4 << shift) == 0 { '-' } else { 'r' });
        result.push(if mode & (0o2 << shift) == 0 { '-' } else { 'w' });
        let executable = mode & (0o1 << shift) != 0;
        let special = mode & (0o1000 << (shift / 3)) != 0;
        result.push(match (special, executable, shift) {
            (true, true, 0) => 't',
            (true, false, 0) => 'T',
            (true, true, _) => 's',
            (true, false, _) => 'S',
            (false, true, _) => 'x',
            (false, false, _) => '-',
        });
    }
    result
}
