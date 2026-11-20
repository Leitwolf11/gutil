use std::env;
use std::io::{self, Read};

use unicode_width::UnicodeWidthChar;

use super::{BYTES, CHARS, LINES, WIDTH, WORDS};

pub(super) const BUFFER_SIZE: usize = 64 * 1024 + 4;

#[derive(Clone, Copy)]
pub(super) struct Locale {
    unicode: bool,
    gnu_space: bool,
}

impl Locale {
    pub(super) fn detect() -> Self {
        let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|key| env::var(key).ok().filter(|value| !value.is_empty()));
        let unicode = locale
            .as_deref()
            .is_some_and(|value| !matches!(value, "C" | "POSIX"));
        Self {
            unicode,
            gnu_space: env::var_os("POSIXLY_CORRECT").is_none(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Counts {
    pub(super) lines: u64,
    pub(super) words: u64,
    pub(super) chars: u64,
    pub(super) bytes: u64,
    pub(super) width: u64,
}

impl Counts {
    pub(super) fn add(&mut self, other: Self) {
        self.lines = self.lines.saturating_add(other.lines);
        self.words = self.words.saturating_add(other.words);
        self.chars = self.chars.saturating_add(other.chars);
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.width = self.width.max(other.width);
    }
}

pub(super) fn read(
    input: &mut impl Read,
    metrics: u8,
    locale: Locale,
    buffer: &mut [u8],
) -> io::Result<Counts> {
    let mut counter = Counter {
        counts: Counts::default(),
        locale,
        metrics,
        word: false,
        column: 0,
    };
    let mut pending = 0;
    let byte_words = !locale.unicode && metrics & (WORDS | CHARS | WIDTH) == WORDS;
    loop {
        let length = match input.read(&mut buffer[pending..]) {
            Ok(0) => break,
            Ok(length) => length,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let end = length + pending;
        counter.counts.bytes += length as u64;
        if metrics & LINES != 0 {
            counter.counts.lines +=
                memchr::memchr_iter(b'\n', &buffer[pending..end]).count() as u64;
        }
        if byte_words {
            counter.counts.words += byte_word_starts(&buffer[..end], &mut counter.word);
        } else if metrics & !(LINES | BYTES) != 0 {
            if locale.unicode {
                pending = counter.utf8(&buffer[..end]);
                buffer.copy_within(end - pending..end, 0);
            } else {
                for &byte in &buffer[..end] {
                    counter.character(
                        char::from(byte),
                        byte.is_ascii()
                            .then(|| usize::from((b' '..=b'~').contains(&byte))),
                    );
                }
            }
        }
    }
    if pending != 0 {
        counter.start_word();
    }
    counter.counts.width = counter.counts.width.max(counter.column);
    Ok(counter.counts)
}

fn byte_word_starts(bytes: &[u8], in_word: &mut bool) -> u64 {
    let Some((&first, rest)) = bytes.split_first() else {
        return 0;
    };
    let space = |byte| matches!(byte, b' ' | b'\t'..=b'\r');
    let initial = u64::from(!*in_word && !space(first));
    // Interior pairs are independent. Only the first byte needs the state
    // from the preceding read.
    let interior = bytes
        .iter()
        .zip(rest)
        .map(|(&left, &right)| u64::from(space(left) & !space(right)))
        .sum::<u64>();
    *in_word = !space(rest.last().copied().unwrap_or(first));
    initial + interior
}

struct Counter {
    counts: Counts,
    locale: Locale,
    metrics: u8,
    word: bool,
    column: u64,
}

impl Counter {
    fn utf8(&mut self, mut bytes: &[u8]) -> usize {
        loop {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    self.text(text);
                    return 0;
                }
                Err(error) => {
                    let (valid, rest) = bytes.split_at(error.valid_up_to());
                    if let Ok(valid) = std::str::from_utf8(valid) {
                        self.text(valid);
                    }
                    let Some(length) = error.error_len() else {
                        return rest.len();
                    };
                    self.start_word();
                    bytes = &rest[length..];
                }
            }
        }
    }

    fn text(&mut self, text: &str) {
        for character in text.chars() {
            let width = if self.metrics & WIDTH == 0 {
                None
            } else {
                character.width()
            };
            self.character(character, width);
        }
    }

    fn character(&mut self, character: char, width: Option<usize>) {
        if self.metrics & CHARS != 0 {
            self.counts.chars += 1;
        }
        if self.metrics & WORDS != 0 {
            let space = matches!(character, ' ' | '\t'..='\r')
                || (self.locale.unicode
                    && match character {
                        '\u{00a0}' | '\u{2007}' | '\u{202f}' | '\u{2060}' => self.locale.gnu_space,
                        '\u{0085}' => false,
                        _ => character.is_whitespace(),
                    });
            if space {
                self.word = false;
            } else {
                self.start_word();
            }
        }
        if self.metrics & WIDTH != 0 {
            match character {
                '\n' | '\r' | '\x0c' => {
                    self.counts.width = self.counts.width.max(self.column);
                    self.column = 0;
                }
                '\t' => self.column = (self.column / 8 + 1) * 8,
                _ => self.column += width.unwrap_or(0) as u64,
            }
        }
    }

    fn start_word(&mut self) {
        if self.metrics & WORDS != 0 && !self.word {
            self.counts.words += 1;
            self.word = true;
        }
    }
}
