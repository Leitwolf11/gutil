use std::io::{self, Write};
use std::ops::Range;

use crate::text::horizontal_blank;

use super::options::{Mode, Options, Splitter};

pub(super) struct Records<'a> {
    options: &'a Options,
    byte_offset: usize,
    joined_fields: usize,
    wrote_field: bool,
    opened_record: bool,
}

impl<'a> Records<'a> {
    pub(super) fn new(options: &'a Options) -> Self {
        Self {
            options,
            byte_offset: 0,
            joined_fields: 0,
            wrote_field: false,
            opened_record: false,
        }
    }

    pub(super) fn terminator(&self) -> u8 {
        self.options.terminator
    }

    pub(super) fn streams_bytes(&self) -> bool {
        matches!(self.options.mode, Mode::Bytes)
    }

    pub(super) fn record(
        &mut self,
        record: &[u8],
        terminated: bool,
        output: &mut impl Write,
    ) -> io::Result<()> {
        let rendered = match &self.options.mode {
            Mode::Bytes => return self.byte_fragment(record, terminated, output),
            Mode::Characters | Mode::WholeUnits if record.is_ascii() => {
                self.byte_ranges(record, output)?;
                true
            }
            Mode::Characters => {
                self.character_ranges(record, output)?;
                true
            }
            Mode::WholeUnits => {
                self.whole_units(record, output)?;
                true
            }
            Mode::Fields {
                splitter: Splitter::Byte(byte),
                suppress,
            } if *byte == self.terminator() => {
                return self.joined(record, terminated, *suppress, output);
            }
            Mode::Fields { splitter, suppress } => {
                self.fields(record, splitter, *suppress, terminated, output)?
            }
        };
        if rendered {
            output.write_all(&[self.terminator()])?;
        }
        Ok(())
    }

    pub(super) fn finish(&self, output: &mut impl Write) -> io::Result<()> {
        if self.opened_record || self.byte_offset != 0 {
            output.write_all(&[self.terminator()])?;
        }
        Ok(())
    }

    fn byte_fragment(
        &mut self,
        bytes: &[u8],
        terminated: bool,
        output: &mut impl Write,
    ) -> io::Result<()> {
        let offset = self.byte_offset;
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("record is too long"))?;
        let first = self
            .options
            .ranges
            .partition_point(|range| range.end <= offset);
        for (index, range) in self.options.ranges.iter().enumerate().skip(first) {
            if range.start >= end {
                break;
            }
            self.write_separator(index != 0 && range.start >= offset, output)?;
            output.write_all(
                &bytes[range.start.saturating_sub(offset)..range.end.min(end) - offset],
            )?;
        }
        self.byte_offset = if terminated { 0 } else { end };
        if terminated {
            output.write_all(&[self.terminator()])?;
        }
        Ok(())
    }

    fn byte_ranges(&self, record: &[u8], output: &mut impl Write) -> io::Result<()> {
        for (index, range) in self.options.ranges.iter().enumerate() {
            if range.start >= record.len() {
                break;
            }
            self.write_separator(index != 0, output)?;
            output.write_all(&record[range.start..range.end.min(record.len())])?;
        }
        Ok(())
    }

    fn character_ranges(&self, record: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut units = units(record).enumerate().peekable();
        let mut wrote = false;
        for selection in &self.options.ranges {
            let mut first = None;
            let mut end = 0;
            while let Some((index, unit)) = units.peek() {
                let position = *index;
                if position >= selection.end {
                    break;
                }
                if position >= selection.start {
                    first.get_or_insert(unit.start);
                    end = unit.end;
                }
                units.next();
            }
            if let Some(first) = first {
                self.write_separator(wrote, output)?;
                output.write_all(&record[first..end])?;
                wrote = true;
            }
        }
        Ok(())
    }

    fn whole_units(&self, record: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut ranges = self.options.ranges.iter().peekable();
        let mut wrote = false;
        for unit in units(record) {
            let mut first = None;
            let mut suffix = true;
            for position in unit.clone() {
                while ranges.peek().is_some_and(|range| position >= range.end) {
                    ranges.next();
                }
                let Some(range) = ranges.peek() else {
                    return Ok(());
                };
                if range.contains(&position) {
                    first.get_or_insert(position == range.start);
                } else if first.is_some() {
                    suffix = false;
                }
            }
            if let Some(at_start) = first.filter(|_| suffix) {
                self.write_separator(wrote && at_start, output)?;
                output.write_all(&record[unit])?;
                wrote = true;
            }
        }
        Ok(())
    }

    fn fields(
        &self,
        mut record: &[u8],
        splitter: &Splitter,
        suppress: bool,
        terminated: bool,
        output: &mut impl Write,
    ) -> io::Result<bool> {
        let utf8 = self.options.utf8
            && (!matches!(splitter, Splitter::Blanks { .. }) || !record.is_ascii());
        if matches!(splitter, Splitter::Blanks { trim: true }) {
            record = trim_blanks(record, utf8);
            if record.is_empty() && !terminated {
                return Ok(false);
            }
        }
        let mut separator = splitter.next(record, utf8);
        if separator.is_none() {
            if !suppress {
                output.write_all(record)?;
            }
            return Ok(!suppress);
        }
        let mut ranges = self.options.ranges.iter().peekable();
        let mut wrote = false;
        let mut position = 0;
        loop {
            while ranges.peek().is_some_and(|range| position >= range.end) {
                ranges.next();
            }
            let Some(range) = ranges.peek() else {
                break;
            };
            if range.contains(&position) {
                let end = separator
                    .as_ref()
                    .map_or(record.len(), |separator| separator.start);
                self.write_separator(wrote, output)?;
                output.write_all(&record[..end])?;
                wrote = true;
            }
            let Some(consumed) = separator else {
                break;
            };
            record = &record[consumed.end..];
            position += 1;
            separator = splitter.next(record, utf8);
        }
        Ok(true)
    }

    fn joined(
        &mut self,
        field: &[u8],
        terminated: bool,
        suppress: bool,
        output: &mut impl Write,
    ) -> io::Result<()> {
        let next = self.joined_fields;
        let first_undelimited = next == 0 && !terminated;
        if first_undelimited && suppress {
            return Ok(());
        }
        self.joined_fields += 1;
        let index = self
            .options
            .ranges
            .partition_point(|range| next >= range.end);
        let selected = first_undelimited
            || self
                .options
                .ranges
                .get(index)
                .is_some_and(|range| range.contains(&next));
        self.opened_record |= !suppress || next > 0 || selected;
        if selected {
            self.write_separator(self.wrote_field, output)?;
            output.write_all(field)?;
            self.wrote_field = true;
        }
        Ok(())
    }

    fn write_separator(&self, wrote: bool, output: &mut impl Write) -> io::Result<()> {
        if wrote {
            if let Some(separator) = &self.options.separator {
                output.write_all(separator)?;
            }
        }
        Ok(())
    }
}

impl Splitter {
    fn next(&self, record: &[u8], utf8: bool) -> Option<Range<usize>> {
        match self {
            Self::Byte(byte) if utf8 && (0x80..=0xf4).contains(byte) => {
                units(record).find(|unit| record[unit.clone()] == [*byte])
            }
            Self::Byte(byte) => memchr::memchr(*byte, record).map(|index| index..index + 1),
            Self::Text(finder) => finder
                .find(record)
                .map(|index| index..index + finder.needle().len()),
            Self::Blanks { .. } if !utf8 => {
                let start = memchr::memchr2(b' ', b'\t', record)?;
                let end = record[start..]
                    .iter()
                    .position(|byte| !matches!(byte, b' ' | b'\t'))
                    .map_or(record.len(), |offset| start + offset);
                Some(start..end)
            }
            Self::Blanks { .. } => {
                let mut units = units(record);
                let first = units.find(|unit| blank(&record[unit.clone()]))?;
                let end = units
                    .find(|unit| !blank(&record[unit.clone()]))
                    .map_or(record.len(), |unit| unit.start);
                Some(first.start..end)
            }
        }
    }
}

fn trim_blanks(record: &[u8], utf8: bool) -> &[u8] {
    if !utf8 {
        let nonblank = |&byte: &u8| !matches!(byte, b' ' | b'\t');
        let Some(start) = record.iter().position(nonblank) else {
            return &[];
        };
        let end = record
            .iter()
            .rposition(nonblank)
            .map_or(start, |index| index + 1);
        return &record[start..end];
    }
    let mut nonblank = units(record).filter(|unit| !blank(&record[unit.clone()]));
    let Some(first) = nonblank.next() else {
        return &[];
    };
    let end = nonblank.last().map_or(first.end, |unit| unit.end);
    &record[first.start..end]
}

fn blank(unit: &[u8]) -> bool {
    if unit.len() == 1 {
        return matches!(unit[0], b' ' | b'\t');
    }
    std::str::from_utf8(unit)
        .ok()
        .and_then(|text| text.chars().next())
        .is_some_and(horizontal_blank)
}

fn units(record: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        let first = *record.get(start)?;
        let mut width = match first {
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => 1,
        };
        if width > 1
            && record
                .get(start..start + width)
                .is_none_or(|unit| std::str::from_utf8(unit).is_err())
        {
            width = 1;
        }
        let range = start..start + width;
        start += width;
        Some(range)
    })
}
