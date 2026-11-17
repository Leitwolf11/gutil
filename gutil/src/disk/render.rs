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

    fn amount(self, usage: Usage) -> u64 {
        match self {
            Self::Size => usage.size,
            Self::Used => usage.used,
            Self::Available => usage.available,
            Self::Use => percent(usage.used, usage.available).unwrap_or(0),
            Self::Inodes => usage.inodes,
            Self::InodesUsed => usage.inodes_used,
            Self::InodesFree => usage.inodes_free,
            Self::InodesUse => percent(usage.inodes_used, usage.inodes_free).unwrap_or(0),
            _ => 0,
        }
    }

    fn text(self, row: &Row) -> Option<&str> {
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
            return crate::presentation::safe_message(text)
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
                if (self == Self::Use && usage.used.saturating_add(usage.available) == 0)
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

fn percent(used: u64, available: u64) -> Option<u64> {
    let total = u128::from(used) + u128::from(available);
    (total != 0).then(|| u64::try_from((u128::from(used) * 100).div_ceil(total)).unwrap_or(100))
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

fn format_size(bytes: u64, size: Size) -> String {
    let base = match size {
        Size::Blocks(block, suffix) => return format!("{}{suffix}", bytes.div_ceil(block)),
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
    let tenths = (u128::from(bytes) * 10).div_ceil(u128::from(scale));
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
    let mut formatted: Vec<Vec<String>> = Vec::with_capacity(rows.len() + 1);
    formatted.push(
        fields
            .iter()
            .map(|column| column.headings(size, layout.portable, layout.columns.is_some()))
            .collect(),
    );
    formatted.extend(
        rows.iter()
            .map(|row| fields.iter().map(|column| column.cell(row, size)).collect()),
    );
    let mut widths = vec![0; fields.len()];
    for row in &formatted {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    for row in &formatted {
        for (index, cell) in row.iter().enumerate() {
            if index != 0 {
                output.write_all(b" ")?;
            }
            let padding = widths[index].saturating_sub(cell.width());
            if fields[index].numeric() {
                write!(output, "{:padding$}{cell}", "")?;
            } else if index + 1 == fields.len() {
                write!(output, "{cell}")?;
            } else {
                write!(output, "{cell}{:padding$}", "")?;
            }
        }
        output.write_all(b"\n")?;
    }
    Ok(())
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
    device: &'a str,
    device_type: &'a str,
    mount_point: &'a str,
    fs_type: &'a str,
    #[serde(rename = "type")]
    filesystem_type: &'a str,
    opts: &'a str,
    total: u64,
    free: u64,
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
            device: &row.device,
            device_type: row.kind.name(),
            mount_point: &row.target,
            fs_type: &row.filesystem,
            filesystem_type: &row.filesystem,
            opts: &row.options,
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
