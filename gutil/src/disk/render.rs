use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::{self, Write};

use rusty_rich::{AlignMethod, Column as RichColumn, ConsoleOptions, Renderable, Style, Table};
use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use super::{Command, Row, Usage};
use crate::disk::options::{Layout, Size};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Column {
    Source,
    Filesystem,
    Size,
    Used,
    Available,
    Use,
    Inodes,
    InodesUsed,
    InodesFree,
    InodesUse,
    File,
    Target,
}

impl Column {
    pub(super) const DF_ALL: [Self; 12] = [
        Self::Source,
        Self::Filesystem,
        Self::Inodes,
        Self::InodesUsed,
        Self::InodesFree,
        Self::InodesUse,
        Self::Size,
        Self::Used,
        Self::Available,
        Self::Use,
        Self::File,
        Self::Target,
    ];

    pub(super) fn parse(command: Command, name: &str) -> Option<Self> {
        match (command, name) {
            (_, "size") => Some(Self::Size),
            (_, "used") => Some(Self::Used),
            (_, "avail") => Some(Self::Available),
            (Command::Df, "source") | (Command::Duf, "filesystem") => Some(Self::Source),
            (Command::Df, "fstype") | (Command::Duf, "type") => Some(Self::Filesystem),
            (Command::Df, "target") | (Command::Duf, "mountpoint") => Some(Self::Target),
            (Command::Df, "pcent") | (Command::Duf, "usage") => Some(Self::Use),
            (Command::Df, "itotal") | (Command::Duf, "inodes") => Some(Self::Inodes),
            (Command::Df, "iused") | (Command::Duf, "inodes_used") => Some(Self::InodesUsed),
            (Command::Df, "iavail") | (Command::Duf, "inodes_avail") => Some(Self::InodesFree),
            (Command::Df, "ipcent") | (Command::Duf, "inodes_usage") => Some(Self::InodesUse),
            (Command::Df, "file") => Some(Self::File),
            _ => None,
        }
    }

    fn headings(self, size: Size, portable: bool, custom: bool) -> String {
        match self {
            Self::Source => "Filesystem",
            Self::Filesystem => "Type",
            Self::Size => return size_heading(size, portable),
            Self::Used => "Used",
            Self::Available if !custom => "Available",
            Self::Available => "Avail",
            Self::Use if portable => "Capacity",
            Self::Use => "Use%",
            Self::Inodes => "Inodes",
            Self::InodesUsed => "IUsed",
            Self::InodesFree => "IFree",
            Self::InodesUse => "IUse%",
            Self::File => "File",
            Self::Target => "Mounted on",
        }
        .into()
    }

    fn amount(self, usage: Usage) -> i128 {
        match self {
            Self::Size => i128::from(usage.size),
            Self::Used => i128::from(usage.used),
            Self::Available => usage.available,
            Self::Use => percent(usage.used, usage.available).unwrap_or(0),
            Self::Inodes => i128::from(usage.inodes),
            Self::InodesUsed => i128::from(usage.inodes_used),
            Self::InodesFree => i128::from(usage.inodes_free),
            Self::InodesUse => {
                percent(usage.inodes_used, i128::from(usage.inodes_free)).unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn text(self, row: &Row) -> Option<&OsStr> {
        match self {
            Self::Source => Some(&row.device),
            Self::Filesystem => Some(&row.filesystem),
            Self::File => Some(&row.file),
            Self::Target => Some(&row.target),
            _ => None,
        }
    }

    fn numeric(self) -> bool {
        !matches!(
            self,
            Self::Source | Self::Filesystem | Self::File | Self::Target
        )
    }

    fn cell(self, row: &Row, size: Size) -> String {
        if let Some(text) = self.text(row) {
            return crate::presentation::safe_message(&text.to_string_lossy())
                .replace(['\n', '\r'], "\u{fffd}")
                .replace('\t', " ");
        }
        let Some(usage) = row.usage else {
            return "-".into();
        };
        let amount = self.amount(usage);
        match self {
            Self::Size | Self::Used | Self::Available => format_size(amount, size),
            Self::Use | Self::InodesUse => {
                if (self == Self::Use
                    && i128::from(usage.used).saturating_add(usage.available) <= 0)
                    || (self == Self::InodesUse && usage.inodes == 0)
                {
                    "-".into()
                } else {
                    format!("{amount}%")
                }
            }
            _ if usage.inodes == 0 => "-".into(),
            _ => amount.to_string(),
        }
    }
}

fn percent(used: u64, available: i128) -> Option<i128> {
    let total = i128::from(used).saturating_add(available);
    if total <= 0 {
        return None;
    }
    let scaled = i128::from(used) * 100;
    Some(scaled / total + i128::from(scaled % total != 0))
}

fn size_heading(size: Size, portable: bool) -> String {
    let Size::Blocks(bytes, _) = size else {
        return "Size".into();
    };
    if portable {
        return format!("{bytes}-blocks");
    }
    for (base, units) in [
        (1024, ["K", "M", "G", "T", "P", "E"]),
        (1000, ["kB", "MB", "GB", "TB", "PB", "EB"]),
    ] {
        let mut scale = 1;
        let mut prefix = None;
        for unit in units {
            scale *= base;
            if bytes % scale != 0 {
                break;
            }
            prefix = Some((scale, unit));
        }
        if let Some((scale, suffix)) = prefix {
            return format!("{}{suffix}-blocks", bytes / scale);
        }
    }
    format!("{bytes}B-blocks")
}

fn format_size(amount: i128, size: Size) -> String {
    let rendered = format_magnitude(amount.unsigned_abs(), size);
    if amount < 0 {
        format!("-{rendered}")
    } else {
        rendered
    }
}

fn format_magnitude(bytes: u128, size: Size) -> String {
    let base = match size {
        Size::Blocks(block, suffix) => {
            return format!("{}{suffix}", bytes.div_ceil(u128::from(block)))
        }
        Size::Binary => 1024,
        Size::Si => 1000,
    };
    if bytes < base {
        return bytes.to_string();
    }
    let mut scale = 1;
    let mut suffix = 'B';
    for unit in ['K', 'M', 'G', 'T', 'P', 'E'] {
        if bytes / scale < base {
            break;
        }
        scale *= base;
        suffix = unit;
    }
    let tenths = (bytes / scale) * 10 + ((bytes % scale) * 10).div_ceil(scale);
    if tenths < 100 {
        format!("{}.{}{suffix}", tenths / 10, tenths % 10)
    } else {
        format!("{}{suffix}", bytes.div_ceil(scale))
    }
}

fn columns(command: Command, layout: &Layout) -> Vec<Column> {
    if let Some(columns) = &layout.columns {
        return columns.clone();
    }
    let mut columns = vec![Column::Source];
    if layout.show_type || command == Command::Duf {
        columns.push(Column::Filesystem);
    }
    if layout.inodes {
        columns.extend([
            Column::Inodes,
            Column::InodesUsed,
            Column::InodesFree,
            Column::InodesUse,
        ]);
    } else {
        columns.extend([Column::Size, Column::Used, Column::Available, Column::Use]);
    }
    columns.push(Column::Target);
    columns
}

pub(super) fn df(
    rows: &[Row],
    layout: &Layout,
    size: Size,
    output: &mut impl Write,
) -> io::Result<()> {
    let fields = columns(Command::Df, layout);
    let mut formatted: Vec<Vec<Cow<'_, [u8]>>> = Vec::with_capacity(rows.len() + 1);
    formatted.push(
        fields
            .iter()
            .map(|column| {
                Cow::Owned(
                    column
                        .headings(size, layout.portable, layout.columns.is_some())
                        .into_bytes(),
                )
            })
            .collect(),
    );
    formatted.extend(rows.iter().map(|row| {
        fields
            .iter()
            .map(|column| {
                if let Some(name) = column.text(row) {
                    native_cell(name)
                } else {
                    Cow::Owned(column.cell(row, size).into_bytes())
                }
            })
            .collect()
    }));
    let cell_width =
        |cell: &[u8]| std::str::from_utf8(cell).map_or(cell.len(), UnicodeWidthStr::width);
    let mut widths = vec![0; fields.len()];
    for row in &formatted {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell_width(cell));
        }
    }
    for row in &formatted {
        for (index, cell) in row.iter().enumerate() {
            if index != 0 {
                output.write_all(b" ")?;
            }
            let padding = widths[index].saturating_sub(cell_width(cell));
            if fields[index].numeric() {
                write!(output, "{:padding$}", "")?;
            }
            output.write_all(cell)?;
            if !fields[index].numeric() && index + 1 < fields.len() {
                write!(output, "{:padding$}", "")?;
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
}

fn native_cell(path: &OsStr) -> Cow<'_, [u8]> {
    let bytes = path.as_encoded_bytes();
    if bytes.iter().any(u8::is_ascii_control) {
        Cow::Owned(
            bytes
                .iter()
                .map(|&byte| if byte.is_ascii_control() { b'?' } else { byte })
                .collect(),
        )
    } else {
        Cow::Borrowed(bytes)
    }
}

pub(super) fn duf(
    rows: &mut [Row],
    layout: &Layout,
    color: bool,
    output: &mut impl Write,
) -> io::Result<()> {
    let sort = layout.sort.unwrap_or(Column::Target);
    rows.sort_by(|left, right| {
        if sort.numeric() {
            sort.amount(right.usage.unwrap_or_default())
                .cmp(&sort.amount(left.usage.unwrap_or_default()))
        } else {
            sort.text(left).cmp(&sort.text(right))
        }
        .then_with(|| left.target.cmp(&right.target))
    });
    if layout.duf.json {
        serde_json::to_writer(
            &mut *output,
            &rows.iter().map(JsonRow::from).collect::<Vec<_>>(),
        )?;
        return output.write_all(b"\n");
    }
    let fields = columns(Command::Duf, layout);
    let width = layout
        .width
        .or_else(|| terminal_size::terminal_size().map(|(width, _)| usize::from(width.0)))
        .unwrap_or(100);
    let mut table = Table::new();
    if layout.duf.ascii {
        table = table.box_style(rusty_rich::box_drawing::BOX_ASCII.clone());
    }
    for field in &fields {
        let mut column = RichColumn::new(field.headings(Size::Binary, false, true));
        if field.numeric() {
            column = column.justify(AlignMethod::Right);
        }
        if color {
            column = column.header_style(Style::from_str("bold cyan"));
        }
        table.add_column(column);
    }
    for row in rows {
        table.add_row_str(
            fields
                .iter()
                .map(|field| field.cell(row, Size::Binary))
                .collect(),
        );
    }
    let mut options = ConsoleOptions::default().update_width(width);
    options.ascii_only = layout.duf.ascii;
    options.markup = false;
    writeln!(output, "{}", table.render(&options).to_ansi().trim_end())
}

#[derive(Serialize)]
struct JsonRow<'a> {
    device: Cow<'a, str>,
    device_type: &'a str,
    mount_point: Cow<'a, str>,
    fs_type: Cow<'a, str>,
    #[serde(rename = "type")]
    filesystem_type: Cow<'a, str>,
    opts: Cow<'a, str>,
    total: u64,
    free: i128,
    used: u64,
    inodes: u64,
    inodes_free: u64,
    inodes_used: u64,
    blocks: u64,
    block_size: u64,
}

impl<'a> From<&'a Row> for JsonRow<'a> {
    fn from(row: &'a Row) -> Self {
        let usage = row.usage.unwrap_or_default();
        Self {
            device: row.device.to_string_lossy(),
            device_type: row.kind.name(),
            mount_point: row.target.to_string_lossy(),
            fs_type: row.filesystem.to_string_lossy(),
            filesystem_type: row.filesystem.to_string_lossy(),
            opts: row.options.to_string_lossy(),
            total: usage.size,
            free: usage.available,
            used: usage.used,
            inodes: usage.inodes,
            inodes_free: usage.inodes_free,
            inodes_used: usage.inodes_used,
            blocks: usage.blocks,
            block_size: usage.block_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Column, Row, Size, Usage};
    use crate::disk::Kind;

    #[test]
    fn preserves_negative_availability_in_the_value_and_percentage() {
        let row = Row {
            device: "full".into(),
            target: "/test".into(),
            filesystem: "ext4".into(),
            file: "-".into(),
            kind: Kind::Local,
            options: "rw".into(),
            usage: Some(Usage {
                size: 500,
                used: 300,
                available: -25,
                ..Usage::default()
            }),
        };
        assert_eq!(Column::Available.cell(&row, Size::Blocks(1, "")), "-25");
        assert_eq!(Column::Use.cell(&row, Size::Blocks(1, "")), "110%");
        let json = serde_json::to_value(super::JsonRow::from(&row));
        assert_eq!(json.ok().and_then(|row| row["free"].as_i64()), Some(-25));
    }
}
