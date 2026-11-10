use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::Metadata;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rusty_rich::console::{ConsoleOptions, RenderResult, Renderable};
use rusty_rich::segment::Segment;
use rusty_rich::{Columns, Style};

use crate::config::{Output, Quote};
use crate::presentation::color_enabled;

pub(super) struct Names {
    terminal: bool,
    quote: Quote,
    cwd: Option<PathBuf>,
    styles: Option<(Style, Style)>,
}

impl Names {
    pub(super) fn new(output: Output, terminal: bool, quote: Quote) -> Self {
        Self {
            terminal,
            quote,
            styles: color_enabled(output.color, terminal)
                .then(|| (Style::from_str("bold blue"), Style::from_str("cyan"))),
            cwd: output
                .hyperlinks
                .enabled(terminal)
                .then(std::env::current_dir)
                .and_then(Result::ok),
        }
    }

    pub(super) fn color(&self) -> bool {
        self.styles.is_some()
    }

    pub(super) fn links(&self) -> bool {
        self.cwd.is_some()
    }

    pub(super) fn write(
        &self,
        output: &mut impl Write,
        name: &OsStr,
        path: &Path,
        metadata: Option<&Metadata>,
    ) -> io::Result<()> {
        if self.styles.is_none()
            && self.cwd.is_none()
            && (self.quote == Quote::Literal || (self.quote == Quote::Auto && !self.terminal))
        {
            #[cfg(unix)]
            {
                return output.write_all(name.as_encoded_bytes());
            }
            #[cfg(not(unix))]
            {
                return output.write_all(name.to_string_lossy().as_bytes());
            }
        }

        output.write_all(self.cell(name, path, metadata).ansi().as_bytes())
    }

    pub(super) fn cell(&self, name: &OsStr, path: &Path, metadata: Option<&Metadata>) -> Cell {
        let text = match self.quote {
            Quote::Shell => shell_quote(name),
            _ if (self.quote == Quote::Auto && self.terminal) || self.color() || self.links() => {
                name.to_string_lossy()
                    .chars()
                    .map(|character| {
                        if character.is_control() {
                            '?'
                        } else {
                            character
                        }
                    })
                    .collect()
            }
            _ => name.to_string_lossy().into_owned(),
        };
        let style = match (&self.styles, metadata) {
            (Some((directory, _)), Some(metadata)) if metadata.is_dir() => directory.clone(),
            (Some((_, symlink)), Some(metadata)) if metadata.is_symlink() => symlink.clone(),
            _ => Style::null(),
        };
        let uri = self.cwd.as_ref().and_then(|cwd| file_uri(&cwd.join(path)));
        let segment = if style.is_null() {
            Segment::new(text)
        } else {
            Segment::styled(text, style)
        };
        Cell { segment, uri }
    }
}

fn shell_quote(name: &OsStr) -> String {
    let raw = name.as_encoded_bytes();
    if !raw.is_empty()
        && raw
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._/-".contains(byte))
    {
        return name.to_string_lossy().into_owned();
    }
    if let Some(text) = name
        .to_str()
        .filter(|text| !text.chars().any(char::is_control))
    {
        return format!("'{}'", text.replace('\'', "'\\''"));
    }
    let mut quoted = String::from("$'");
    for &byte in raw {
        let _ = write!(quoted, "\\x{byte:02x}");
    }
    quoted.push('\'');
    quoted
}

fn file_uri(path: &Path) -> Option<String> {
    #[cfg(unix)]
    let (raw, prefix) = (path.as_os_str().as_encoded_bytes(), "file://");

    #[cfg(windows)]
    let normalized = path.to_str()?.replace('\\', "/");
    #[cfg(windows)]
    let (raw, prefix) = if let Some(network) = normalized.strip_prefix("//?/UNC/").or_else(|| {
        normalized
            .strip_prefix("//")
            .filter(|path| !path.starts_with("?/"))
    }) {
        (network.as_bytes(), "file://")
    } else {
        (
            normalized
                .strip_prefix("//?/")
                .unwrap_or(&normalized)
                .as_bytes(),
            "file:///",
        )
    };

    if raw.is_empty() {
        return None;
    }
    let mut uri = String::from(prefix);
    for &byte in raw {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    Some(uri)
}

pub(super) struct Cell {
    segment: Segment,
    uri: Option<String>,
}

impl Cell {
    fn ansi(&self) -> String {
        let text = self.segment.to_ansi();
        if let Some(uri) = &self.uri {
            format!("\x1b]8;;{uri}\x1b\\{text}\x1b]8;;\x1b\\")
        } else {
            text
        }
    }
}

impl Renderable for Cell {
    fn render(&self, _: &ConsoleOptions) -> RenderResult {
        RenderResult::from_segments(vec![Segment::new(self.ansi())])
    }
}

pub(super) fn write_columns(output: &mut impl Write, cells: Vec<Cell>) -> io::Result<()> {
    let width = terminal_size::terminal_size().map_or(80, |(width, _)| usize::from(width.0));
    let cell_width = cells
        .iter()
        .map(|cell| cell.segment.cell_length())
        .max()
        .unwrap_or(1);
    let per_row = ((width + 2) / (cell_width + 2)).max(1);
    let options = ConsoleOptions::default().update_width(width);
    let mut cells = cells.into_iter();
    while cells.len() > 0 {
        let row_len = cells.len().min(per_row);
        let mut columns = Columns::new().padding(2);
        for (index, mut cell) in cells.by_ref().take(row_len).enumerate() {
            if index + 1 < row_len {
                let padding = cell_width.saturating_sub(cell.segment.cell_length());
                cell.segment.text.extend(std::iter::repeat_n(' ', padding));
            }
            columns.add(cell);
        }
        writeln!(output, "{}", columns.render(&options).to_ansi())?;
    }
    Ok(())
}
