use std::io::{self, Read, Write};

use unicode_width::UnicodeWidthChar;

use super::options::{Count, Options};
use crate::presentation::ProgressReport;

const BUFFER_SIZE: usize = 64 * 1024;

pub(super) struct Folder<'a> {
    options: &'a Options,
    utf8: bool,
    cursor: Cursor,
    emitted: bool,
    pending: Vec<u8>,
    blank_end: Option<usize>,
}

#[derive(Default)]
struct Cursor {
    column: usize,
    last_width: usize,
}

impl Cursor {
    fn advance(&mut self, count: Count, unit: Unit) {
        if count == Count::Bytes {
            self.column = self.column.saturating_add(unit.bytes);
        } else {
            match unit.character {
                Some('\x08') if self.column != 0 => {
                    self.column = self.column.wrapping_sub(self.last_width);
                }
                Some('\x08') => {}
                Some('\r') => self.column = 0,
                Some('\t') => self.column = self.column.saturating_add(8 - self.column % 8),
                character => {
                    let width = if count == Count::Characters {
                        1
                    } else {
                        match character {
                            Some('\0') => 0,
                            Some('\u{ad}') => 1,
                            _ => character.and_then(UnicodeWidthChar::width).unwrap_or(1),
                        }
                    };
                    self.last_width = width;
                    self.column = self.column.saturating_add(width);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Unit {
    character: Option<char>,
    bytes: usize,
}

impl Unit {
    fn read(input: &[u8], utf8: bool) -> Self {
        let first = input[0];
        if first.is_ascii() {
            return Self {
                character: Some(char::from(first)),
                bytes: 1,
            };
        }
        let scalar = utf8
            .then(|| input.get(..scalar_length(first)))
            .flatten()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| text.chars().next());
        Self {
            bytes: scalar.map_or(1, char::len_utf8),
            character: scalar,
        }
    }

    fn is_blank(self) -> bool {
        matches!(
            self.character,
            Some(
                ' '
                | '\t'
                | '\u{1680}'
                | '\u{2000}'..='\u{2006}'
                | '\u{2008}'..='\u{200a}'
                | '\u{205f}'
                | '\u{3000}',
            )
        )
    }
}

const fn scalar_length(first: u8) -> usize {
    match first {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    }
}

fn partial_scalar(input: &[u8]) -> usize {
    let tail = &input[input.len().saturating_sub(3)..];
    tail.iter()
        .rposition(|byte| byte & 0xc0 != 0x80)
        .map_or(0, |index| {
            let bytes = tail.len() - index;
            if bytes < scalar_length(tail[index]) {
                bytes
            } else {
                0
            }
        })
}

fn utf8_locale() -> bool {
    ["LC_ALL", "LC_CTYPE", "LANG"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .find(|value| !value.is_empty())
        .is_some_and(|value| {
            let value = value.to_ascii_lowercase();
            value.contains("utf-8") || value.contains("utf8")
        })
}

impl<'a> Folder<'a> {
    pub(super) fn new(options: &'a Options) -> Self {
        Self {
            options,
            utf8: utf8_locale(),
            cursor: Cursor::default(),
            emitted: false,
            pending: Vec::new(),
            blank_end: None,
        }
    }

    pub(super) fn copy(
        &mut self,
        input: &mut impl Read,
        output: &mut impl Write,
        progress: &mut Option<ProgressReport>,
        errors: &mut impl Write,
    ) -> io::Result<Option<io::Error>> {
        let mut buffer = vec![0; BUFFER_SIZE + 3];
        let mut carry = 0;
        let error = loop {
            let length = match input.read(&mut buffer[carry..]) {
                Ok(0) => break None,
                Ok(length) => length,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => break Some(error),
            };
            let end = carry + length;
            carry = if self.utf8 {
                partial_scalar(&buffer[..end])
            } else {
                0
            };
            self.block(&buffer[..end - carry], output)?;
            buffer.copy_within(end - carry..end, 0);
            if let Some(progress) = progress {
                progress.advance(length as u64, errors);
            }
        };
        self.block(&buffer[..carry], output)?;
        output.write_all(&self.pending)?;
        self.reset();
        Ok(error)
    }

    fn reset(&mut self) {
        self.cursor.column = 0;
        self.pending.clear();
        self.blank_end = None;
        self.emitted = false;
    }

    fn block(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut start = 0;
        for end in memchr::memchr_iter(b'\n', bytes) {
            self.segment(&bytes[start..end], output)?;
            output.write_all(&self.pending)?;
            output.write_all(b"\n")?;
            self.reset();
            start = end + 1;
        }
        self.segment(&bytes[start..], output)
    }

    fn segment(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if self.options.spaces {
            self.prefer_blanks(bytes, output)
        } else if self.fixed_units(bytes) {
            self.fixed_segment(bytes, output)
        } else {
            self.variable_segment(bytes, output)
        }
    }

    fn fixed_units(&self, bytes: &[u8]) -> bool {
        if self.utf8 && !bytes.is_ascii() {
            return false;
        }
        self.options.count == Count::Bytes
            || (memchr::memchr3(b'\t', b'\r', b'\x08', bytes).is_none()
                && (self.options.count != Count::Columns || !bytes.contains(&0)))
    }

    fn fixed_segment(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let width = self.options.width;
        let first = bytes.len().min(width.saturating_sub(self.cursor.column));
        output.write_all(&bytes[..first])?;
        self.cursor.column += first;
        for chunk in bytes[first..].chunks(width) {
            output.write_all(b"\n")?;
            output.write_all(chunk)?;
            self.cursor.column = chunk.len();
        }
        self.emitted = true;
        self.cursor.last_width = 1;
        Ok(())
    }

    fn variable_segment(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut start = 0;
        let mut offset = 0;
        while offset < bytes.len() {
            let unit = Unit::read(&bytes[offset..], self.utf8);
            self.cursor.advance(self.options.count, unit);
            if self.cursor.column > self.options.width && self.emitted {
                output.write_all(&bytes[start..offset])?;
                output.write_all(b"\n")?;
                self.cursor.column = 0;
                self.cursor.advance(self.options.count, unit);
                start = offset;
            }
            self.emitted = true;
            offset += unit.bytes;
        }
        output.write_all(&bytes[start..])
    }

    fn prefer_blanks(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut offset = 0;
        while offset < bytes.len() {
            let unit = Unit::read(&bytes[offset..], self.utf8);
            self.cursor.advance(self.options.count, unit);
            while self.cursor.column > self.options.width
                && (self.emitted || !self.pending.is_empty())
            {
                let split = self.blank_end.take().unwrap_or(self.pending.len());
                self.emit_prefix(output, split)?;
                output.write_all(b"\n")?;
                self.emitted = false;
                self.cursor.column = 0;
                self.rescan_pending();
                self.cursor.advance(self.options.count, unit);
            }
            let end = offset + unit.bytes;
            self.pending.extend_from_slice(&bytes[offset..end]);
            if unit.is_blank() {
                self.blank_end = Some(self.pending.len());
            }
            if self.pending.len() >= BUFFER_SIZE {
                let flush = self.blank_end.unwrap_or(self.pending.len());
                if flush != 0 {
                    self.emit_prefix(output, flush)?;
                    if self.blank_end.is_some() {
                        self.blank_end = Some(0);
                    }
                }
            }
            offset = end;
        }
        Ok(())
    }

    fn emit_prefix(&mut self, output: &mut impl Write, count: usize) -> io::Result<()> {
        output.write_all(&self.pending[..count])?;
        self.pending.drain(..count);
        self.emitted = true;
        Ok(())
    }

    fn rescan_pending(&mut self) {
        let mut offset = 0;
        while offset < self.pending.len() {
            let mut unit = Unit::read(&self.pending[offset..], self.utf8);
            // The GNU cursor rescan treats malformed UTF-8 as a zero-width
            // placeholder. The first pass still counts the original byte.
            if self.utf8 && unit.character.is_none() {
                unit.character = Some('\0');
            }
            self.cursor.advance(self.options.count, unit);
            offset += unit.bytes;
        }
    }
}
