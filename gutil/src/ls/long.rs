use std::fs::Metadata;
use std::io::{self, Write};

use crate::config::{Long, OwnerFormat, SizeStyle, TimeStyle, Timezone};

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

pub(super) fn write_metadata(
    output: &mut impl Write,
    metadata: &Metadata,
    options: Long,
) -> io::Result<()> {
    let kind = file_kind(metadata);
    let timestamp = metadata
        .modified()
        .ok()
        .and_then(|time| jiff::Timestamp::try_from(time).ok());
    let timestamp = timestamp.map_or_else(|| String::from("?"), |time| format_time(time, options));
    let size = match options.size_style {
        SizeStyle::Bytes => metadata.len().to_string(),
        SizeStyle::Human => {
            rusty_rich::filesize::format_size(metadata.len(), None).replace(' ', "")
        }
        SizeStyle::Si => rusty_rich::filesize::decimal(metadata.len(), None, Some("")),
    };

    #[cfg(unix)]
    {
        let mode = permissions(metadata.mode());
        write!(output, "{kind}{mode} {} ", metadata.nlink())?;
        if options.owner_format == OwnerFormat::Numeric {
            write!(output, "{} {} ", metadata.uid(), metadata.gid())?;
        }
        write!(output, "{size} {timestamp} ")
    }
    #[cfg(not(unix))]
    {
        let mode = if metadata.permissions().readonly() {
            "r--------"
        } else {
            "rw-------"
        };
        write!(output, "{kind}{mode} - ")?;
        if options.owner_format == OwnerFormat::Numeric {
            output.write_all(b"- - ")?;
        }
        write!(output, "{size} {timestamp} ")
    }
}

fn format_time(time: jiff::Timestamp, options: Long) -> String {
    match (options.time_style, options.timezone) {
        (TimeStyle::Unix, _) => time.as_second().to_string(),
        (TimeStyle::Iso, Timezone::Utc) => time.strftime("%F %RZ").to_string(),
        (TimeStyle::Rfc3339, Timezone::Utc) => time.strftime("%FT%TZ").to_string(),
        (style, Timezone::Local) => {
            let zoned = time.to_zoned(jiff::tz::TimeZone::system());
            let format = if style == TimeStyle::Iso {
                "%F %R%:z"
            } else {
                "%FT%T%:z"
            };
            zoned.strftime(format).to_string()
        }
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
