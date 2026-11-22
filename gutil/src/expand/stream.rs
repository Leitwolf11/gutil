use std::io::{self, Read, Write};

use unicode_width::UnicodeWidthChar;

use super::options::Options;
use super::stops::advance;
use crate::presentation::ProgressReport;

pub(super) const BUFFER_SIZE: usize = 64 * 1024;
static SPACES: [u8; 8192] = [b' '; 8192];

pub(super) struct Expander<'a> {
    options: &'a Options,
    utf8: bool,
    column: u64,
    convert: bool,
}

impl<'a> Expander<'a> {
    pub(super) fn new(options: &'a Options) -> Self {
        let utf8 = ["LC_ALL", "LC_CTYPE", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .is_some_and(|value| {
                let value = value.to_ascii_lowercase();
                value.contains("utf-8") || value.contains("utf8")
            });
        Self {
            options,
            utf8,
            column: 0,
            convert: true,
        }
    }

    pub(super) fn copy(
        &mut self,
        input: &mut impl Read,
        output: &mut impl Write,
        buffer: &mut [u8],
        progress: &mut Option<ProgressReport>,
        errors: &mut impl Write,
    ) -> io::Result<Option<io::Error>> {
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
            self.process(&buffer[..end - carry], output)?;
            buffer.copy_within(end - carry..end, 0);
            if let Some(progress) = progress {
                progress.advance(length as u64, errors);
            }
        };
        self.process(&buffer[..carry], output)?;
        Ok(error)
    }

    fn process(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        if !self.utf8 || bytes.is_ascii() {
            self.single_byte(bytes, output)
        } else {
            self.unicode(bytes, output)
        }
    }

    fn single_byte(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut written = 0;
        let mut start = 0;
        let has_nul = memchr::memchr(0, bytes).is_some();
        for position in memchr::memchr3_iter(b'\n', b'\t', b'\x08', bytes) {
            self.advance_run(&bytes[start..position], has_nul)?;
            match bytes[position] {
                b'\n' => self.new_line(),
                b'\t' if self.convert => {
                    output.write_all(&bytes[written..position])?;
                    self.write_tab(output)?;
                    written = position + 1;
                }
                b'\x08' => {
                    self.column = self.column.saturating_sub(1);
                    self.convert &= !self.options.initial;
                }
                _ => {}
            }
            start = position + 1;
        }
        self.advance_run(&bytes[start..], has_nul)?;
        output.write_all(&bytes[written..])
    }

    fn advance_run(&mut self, bytes: &[u8], has_nul: bool) -> io::Result<()> {
        if self.convert && !bytes.is_empty() {
            if self.options.initial && bytes.iter().any(|&byte| byte != b' ') {
                self.convert = false;
            } else {
                let nuls = if has_nul {
                    memchr::memchr_iter(0, bytes).count()
                } else {
                    0
                };
                self.column = advance(self.column, (bytes.len() - nuls) as u64)?;
            }
        }
        Ok(())
    }

    fn unicode(&mut self, bytes: &[u8], output: &mut impl Write) -> io::Result<()> {
        let mut base = 0;
        let mut written = 0;
        for chunk in bytes.utf8_chunks() {
            for (offset, character) in chunk.valid().char_indices() {
                match character {
                    '\n' => self.new_line(),
                    _ if !self.convert => {}
                    '\t' => {
                        output.write_all(&bytes[written..base + offset])?;
                        self.write_tab(output)?;
                        written = base + offset + 1;
                    }
                    '\x08' => {
                        self.convert &= !self.options.initial;
                        self.column = self.column.saturating_sub(1);
                    }
                    _ => {
                        if self.options.initial && !horizontal_blank(character) {
                            self.convert = false;
                        } else {
                            let width = match character {
                                '\0' => 0,
                                '\u{ad}' => 1,
                                _ => character.width().unwrap_or(1),
                            };
                            self.column = advance(self.column, width as u64)?;
                        }
                    }
                }
            }
            if !chunk.invalid().is_empty() && self.convert {
                self.convert &= !self.options.initial;
                self.column = advance(self.column, chunk.invalid().len() as u64)?;
            }
            base += chunk.valid().len() + chunk.invalid().len();
        }
        output.write_all(&bytes[written..])
    }

    fn new_line(&mut self) {
        self.column = 0;
        self.convert = true;
    }

    fn write_tab(&mut self, output: &mut impl Write) -> io::Result<()> {
        let next = self.options.stops.next(self.column)?;
        let mut remaining = next - self.column;
        self.column = next;
        while remaining >= SPACES.len() as u64 {
            output.write_all(&SPACES)?;
            remaining -= SPACES.len() as u64;
        }
        let remaining = usize::try_from(remaining).map_err(io::Error::other)?;
        output.write_all(&SPACES[..remaining])
    }
}

fn horizontal_blank(character: char) -> bool {
    matches!(character, ' ' | '\t' | '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' | '\u{205f}' | '\u{3000}')
}

fn partial_scalar(bytes: &[u8]) -> usize {
    let tail = &bytes[bytes.len().saturating_sub(3)..];
    let Some(start) = tail.iter().rposition(|byte| byte & 0xc0 != 0x80) else {
        return 0;
    };
    let expected = match tail[start] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    };
    let remaining = tail.len() - start;
    if remaining < expected {
        remaining
    } else {
        0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[test]
fn retries_an_interrupted_read_before_reporting_the_next_error() {
    use super::options::Request;
    use crate::jobs::Count;

    struct FailingReader(bool);

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            if self.0 {
                Err(io::ErrorKind::InvalidData.into())
            } else {
                self.0 = true;
                Err(io::ErrorKind::Interrupted.into())
            }
        }
    }

    let request = Request::parse(std::iter::empty(), Count::DEFAULT_AUTO_LIMIT).unwrap();
    let Request::Expand(options) = request else {
        panic!("default options must request expansion");
    };
    let mut output = Vec::new();
    let error = Expander::new(&options)
        .copy(
            &mut FailingReader(false),
            &mut output,
            &mut [0; 8],
            &mut None,
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(output, b"");
}
