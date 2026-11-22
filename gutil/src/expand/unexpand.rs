use std::io::{self, Write};

use super::options::Options;
use super::stops::{advance, Stops};
use super::stream::{display_width, horizontal_blank};

const TABS: [u8; 1024] = [b'\t'; 1024];

pub(super) struct Unexpander<'a> {
    options: &'a Options,
    column: u64,
    convert: bool,
    previous_blank: bool,
    pending: Vec<u8>,
    single_stop_blank: usize,
}

impl<'a> Unexpander<'a> {
    pub(super) fn new(options: &'a Options) -> Self {
        Self {
            options,
            column: 0,
            convert: true,
            previous_blank: true,
            pending: Vec::new(),
            single_stop_blank: 0,
        }
    }

    pub(super) fn process(
        &mut self,
        bytes: &[u8],
        utf8: bool,
        output: &mut impl Write,
    ) -> io::Result<()> {
        if !utf8 || bytes.is_ascii() {
            return self.single_byte(bytes, output);
        }
        for chunk in bytes.utf8_chunks() {
            let valid = chunk.valid();
            for (offset, character) in valid.char_indices() {
                let unit = &valid.as_bytes()[offset..offset + character.len_utf8()];
                match character {
                    '\n' => self.new_line(output)?,
                    _ if !self.convert => output.write_all(unit)?,
                    '\t' => self.blank(unit, 0, output)?,
                    _ if horizontal_blank(character) => {
                        self.blank(unit, display_width(character), output)?;
                    }
                    _ => {
                        self.text(unit, display_width(character), character == '\x08', output)?;
                    }
                }
            }
            let invalid = chunk.invalid();
            if !invalid.is_empty() {
                self.text(invalid, invalid.len() as u64, false, output)?;
            }
        }
        Ok(())
    }

    fn single_byte(&mut self, mut bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        while let Some(&byte) = bytes.first() {
            if !self.convert {
                let end = memchr::memchr(b'\n', bytes).map_or(bytes.len(), |index| index + 1);
                output.write_all(&bytes[..end])?;
                if bytes[end - 1] == b'\n' {
                    self.reset();
                }
                bytes = &bytes[end..];
                continue;
            }

            let unit = &bytes[..1];
            let length = match byte {
                b'\n' => {
                    self.new_line(output)?;
                    1
                }
                b' ' => {
                    let length = bytes
                        .iter()
                        .position(|&byte| byte != b' ')
                        .unwrap_or(bytes.len());
                    self.spaces(&bytes[..length], output)?;
                    length
                }
                b'\t' => {
                    self.blank(unit, 0, output)?;
                    1
                }
                b'\x08' | 0 => {
                    self.text(unit, 0, byte == b'\x08', output)?;
                    1
                }
                _ if self.options.initial => {
                    self.text(unit, 0, false, output)?;
                    1
                }
                _ => {
                    let length = bytes
                        .iter()
                        .position(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\x08' | 0))
                        .unwrap_or(bytes.len());
                    self.text(&bytes[..length], length as u64, false, output)?;
                    length
                }
            };
            bytes = &bytes[length..];
        }
        Ok(())
    }

    fn spaces(&mut self, mut spaces: &[u8], output: &mut impl Write) -> io::Result<()> {
        while !spaces.is_empty() {
            let Some(stop) = self.options.stops.next_explicit(self.column)? else {
                self.convert = false;
                self.finish(output)?;
                return output.write_all(spaces);
            };
            let distance = stop - self.column;
            let count =
                usize::try_from(distance.min(spaces.len() as u64)).map_err(io::Error::other)?;
            self.column += count as u64;
            if self.column == stop && (self.previous_blank || count > 1) {
                self.write_tab(output)?;
            } else {
                if self.column == stop {
                    self.single_stop_blank = 1;
                }
                self.pending.extend_from_slice(&spaces[..count]);
            }
            self.previous_blank = true;
            spaces = &spaces[count..];
            if self.column == stop && self.pending.is_empty() {
                if let Stops::Every(period) = self.options.stops {
                    let repeats = spaces.len() as u64 / period;
                    if repeats != 0 {
                        let columns = repeats * period.get();
                        self.column = advance(self.column, columns)?;
                        let repeats = usize::try_from(repeats).map_err(io::Error::other)?;
                        for block in (0..repeats).step_by(TABS.len()) {
                            output.write_all(&TABS[..(repeats - block).min(TABS.len())])?;
                        }
                        let columns = usize::try_from(columns).map_err(io::Error::other)?;
                        spaces = &spaces[columns..];
                    }
                }
            }
        }
        Ok(())
    }

    fn write_tab(&mut self, output: &mut impl Write) -> io::Result<()> {
        // A single blank waiting at the previous stop gains a companion
        // when the current run reaches another stop or contains a tab.
        let count = 1 + usize::from(self.single_stop_blank != 0);
        output.write_all(&TABS[..count])?;
        self.pending.clear();
        self.single_stop_blank = 0;
        Ok(())
    }

    fn blank(&mut self, unit: &[u8], width: u64, output: &mut impl Write) -> io::Result<()> {
        let Some(stop) = self.options.stops.next_explicit(self.column)? else {
            self.convert = false;
            self.finish(output)?;
            return output.write_all(unit);
        };
        let tab = unit == b"\t";
        self.column = if tab {
            stop
        } else {
            advance(self.column, width)?
        };
        if self.column > stop {
            // A tab alone would drop the columns beyond this stop.
            self.finish(output)?;
            output.write_all(unit)?;
        } else if tab || (self.previous_blank && self.column == stop) {
            self.write_tab(output)?;
        } else {
            if self.column == stop {
                self.single_stop_blank = unit.len();
            }
            self.pending.extend_from_slice(unit);
        }
        self.previous_blank = true;
        Ok(())
    }

    fn text(
        &mut self,
        bytes: &[u8],
        width: u64,
        backspace: bool,
        output: &mut impl Write,
    ) -> io::Result<()> {
        if self.convert {
            self.column = if backspace {
                self.column.saturating_sub(1)
            } else {
                advance(self.column, width)?
            };
            self.finish(output)?;
            self.previous_blank = false;
            self.convert = !self.options.initial;
        }
        output.write_all(bytes)
    }

    fn new_line(&mut self, output: &mut impl Write) -> io::Result<()> {
        self.finish(output)?;
        self.reset();
        output.write_all(b"\n")
    }

    fn reset(&mut self) {
        self.column = 0;
        self.convert = true;
        self.previous_blank = true;
    }

    pub(super) fn finish(&mut self, output: &mut impl Write) -> io::Result<()> {
        if self.single_stop_blank != 0 && self.pending.len() > self.single_stop_blank {
            output.write_all(b"\t")?;
            output.write_all(&self.pending[self.single_stop_blank..])?;
        } else if !self.pending.is_empty() {
            output.write_all(&self.pending)?;
        }
        self.pending.clear();
        self.single_stop_blank = 0;
        Ok(())
    }
}
