use std::io::{self, Read, Write};
use std::num::NonZeroUsize;

use data_encoding::{Encoding, BASE32, BASE64};

use super::Base;

const CHUNK: usize = 60 * 1024;
const A32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const A64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

const fn lookup(alphabet: &[u8]) -> [u8; 256] {
    let mut table = [u8::MAX; 256];
    let mut index = 0u8;
    while (index as usize) < alphabet.len() {
        table[alphabet[index as usize] as usize] = index;
        index += 1;
    }
    table
}

const D32: [u8; 256] = lookup(A32);
const D64: [u8; 256] = lookup(A64);

impl Base {
    const fn raw_block(self) -> usize {
        match self {
            Self::Base32 => 5,
            Self::Base64 => 3,
        }
    }

    const fn encoded_block(self) -> usize {
        match self {
            Self::Base32 => 8,
            Self::Base64 => 4,
        }
    }

    const fn width(self) -> u32 {
        match self {
            Self::Base32 => 5,
            Self::Base64 => 6,
        }
    }

    fn table(self) -> &'static [u8; 256] {
        match self {
            Self::Base32 => &D32,
            Self::Base64 => &D64,
        }
    }

    const fn encoding(self) -> Encoding {
        match self {
            Self::Base32 => BASE32,
            Self::Base64 => BASE64,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Wrap(Option<NonZeroUsize>);

impl Wrap {
    pub(super) const fn new(columns: usize) -> Self {
        Self(NonZeroUsize::new(columns))
    }
}

struct Writer<'a, W> {
    output: &'a mut W,
    width: Option<NonZeroUsize>,
    column: usize,
}

impl<W: Write> Writer<'_, W> {
    fn write(&mut self, mut encoded: &[u8]) -> io::Result<()> {
        let Some(width) = self.width.map(NonZeroUsize::get) else {
            return self.output.write_all(encoded);
        };
        while !encoded.is_empty() {
            let bytes = encoded.len().min(width - self.column);
            self.output.write_all(&encoded[..bytes])?;
            self.column += bytes;
            encoded = &encoded[bytes..];
            if self.column == width {
                self.output.write_all(b"\n")?;
                self.column = 0;
            }
        }
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        if self.column != 0 {
            self.output.write_all(b"\n")?;
        }
        Ok(())
    }
}

pub(super) fn encode(
    base: Base,
    wrap: Wrap,
    input: &mut impl Read,
    output: &mut impl Write,
    advance: &mut impl FnMut(usize),
) -> io::Result<()> {
    let encoding = base.encoding();
    let mut input_block = vec![0; CHUNK];
    let mut scratch = vec![0; encoding.encode_len(CHUNK)];
    let mut writer = Writer {
        output,
        width: wrap.0,
        column: 0,
    };
    let mut pending = 0;
    loop {
        let read = read_some(input, &mut input_block[pending..])?;
        let total = pending + read;
        let whole = if read == 0 {
            total
        } else {
            total - total % base.raw_block()
        };
        if whole != 0 {
            let len = encoding.encode_len(whole);
            encoding.encode_mut(&input_block[..whole], &mut scratch[..len]);
            writer.write(&scratch[..len])?;
        }
        if read == 0 {
            return writer.finish();
        }
        advance(read);
        input_block.copy_within(whole..total, 0);
        pending = total - whole;
    }
}

pub(super) fn decode(
    base: Base,
    ignore_garbage: bool,
    input: &mut impl Read,
    output: &mut impl Write,
    advance: &mut impl FnMut(usize),
) -> io::Result<()> {
    let mut buffer = vec![0; CHUNK];
    let mut scratch = vec![0; CHUNK];
    let mut normalized = Vec::with_capacity(CHUNK);
    let mut decoder = Decoder {
        base,
        block: [0; 8],
        length: 0,
        table: base.table(),
    };
    loop {
        let read = read_some(input, &mut buffer)?;
        if read == 0 {
            return decoder.finish(output);
        }
        let bytes = &buffer[..read];
        normalized.clear();
        let mut start = 0;
        for newline in memchr::memchr_iter(b'\n', bytes) {
            normalized.extend_from_slice(&bytes[start..newline]);
            start = newline + 1;
        }
        let bytes = if start == 0 {
            bytes
        } else {
            normalized.extend_from_slice(&bytes[start..]);
            &normalized
        };
        decoder.feed(bytes, ignore_garbage, &mut scratch, output)?;
        advance(read);
    }
}

struct Decoder {
    base: Base,
    block: [u8; 8],
    length: usize,
    table: &'static [u8; 256],
}

impl Decoder {
    fn feed(
        &mut self,
        mut bytes: &[u8],
        ignore_garbage: bool,
        scratch: &mut [u8],
        output: &mut impl Write,
    ) -> io::Result<()> {
        let quantum = self.base.encoded_block();
        while self.length != 0 {
            let Some((&byte, rest)) = bytes.split_first() else {
                return Ok(());
            };
            self.push(byte, ignore_garbage, output)?;
            bytes = rest;
        }
        let whole = bytes.len() / quantum * quantum;
        if self.length == 0 && whole != 0 {
            let encoding = self.base.encoding();
            if let Ok(capacity) = encoding.decode_len(whole) {
                if let Ok(written) = encoding.decode_mut(&bytes[..whole], &mut scratch[..capacity])
                {
                    output.write_all(&scratch[..written])?;
                    bytes = &bytes[whole..];
                }
            }
        }
        for &byte in bytes {
            self.push(byte, ignore_garbage, output)?;
        }
        Ok(())
    }

    fn push(&mut self, byte: u8, ignore_garbage: bool, output: &mut impl Write) -> io::Result<()> {
        if self.table[usize::from(byte)] == u8::MAX && byte != b'=' {
            if ignore_garbage {
                return Ok(());
            }
            self.flush_block(output, false, false)?;
            return Err(invalid_input());
        }
        self.block[self.length] = byte;
        self.length += 1;
        if self.length == self.base.encoded_block() {
            self.flush_block(output, true, true)?;
            self.length = 0;
        }
        Ok(())
    }

    fn finish(&self, output: &mut impl Write) -> io::Result<()> {
        self.flush_block(output, false, true)
    }

    fn flush_block(
        &self,
        output: &mut impl Write,
        complete: bool,
        validate: bool,
    ) -> io::Result<()> {
        let block = &self.block[..self.length];
        let data_length = memchr::memchr(b'=', block).unwrap_or(block.len());
        let padded = data_length < block.len();
        let width = self.base.width();
        let output_length = data_length * width as usize / 8;
        let data_is_valid = (!padded || data_length != 0)
            && (output_length * 8).div_ceil(width as usize) == data_length;
        let padding_is_valid =
            !padded || (complete && block[data_length..].iter().all(|&byte| byte == b'='));
        let mut bits = 0;
        let mut buffer = 0u16;
        let mut decoded = [0; 5];
        let mut count = 0;
        for &symbol in &block[..data_length] {
            buffer = (buffer << width) | u16::from(self.table[usize::from(symbol)]);
            bits += width;
            if bits >= 8 {
                bits -= 8;
                decoded[count] = (buffer >> bits).to_le_bytes()[0];
                count += 1;
            }
        }
        if self.base == Base::Base64 || !padded || padding_is_valid && data_is_valid {
            output.write_all(&decoded[..count])?;
        }
        let trailing_bits = buffer & ((1 << bits) - 1);
        if validate && !(data_is_valid && padding_is_valid && trailing_bits == 0) {
            return Err(invalid_input());
        }
        Ok(())
    }
}

fn read_some(input: &mut impl Read, bytes: &mut [u8]) -> io::Result<usize> {
    loop {
        match input.read(bytes) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

fn invalid_input() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid input")
}

#[cfg(test)]
mod tests {
    use super::{decode, encode, Base, Wrap};
    use std::io::{self, Read};

    struct Fragmented<'a> {
        bytes: &'a [u8],
        interrupted: bool,
    }

    impl Read for Fragmented<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.bytes.read(&mut output[..1])
        }
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn retries_interrupted_reads_and_carries_single_byte_fragments() {
        for (base, expected) in [
            (Base::Base32, b"MZXW6YTBOI======".as_slice()),
            (Base::Base64, b"Zm9vYmFy".as_slice()),
        ] {
            let mut input = Fragmented {
                bytes: b"foobar",
                interrupted: false,
            };
            let mut encoded = Vec::new();
            encode(base, Wrap::new(0), &mut input, &mut encoded, &mut |_| {}).unwrap();
            assert_eq!(encoded, expected);
            let mut input = Fragmented {
                bytes: &encoded,
                interrupted: false,
            };
            let mut decoded = Vec::new();
            decode(base, false, &mut input, &mut decoded, &mut |_| {}).unwrap();
            assert_eq!(decoded, b"foobar");
        }
    }

    #[test]
    fn alphabet_tables_reject_every_non_alphabet_byte() {
        for alphabet in [super::A32.as_slice(), super::A64.as_slice()] {
            let table = super::lookup(alphabet);
            for byte in 0u8..=255 {
                let index = table[usize::from(byte)];
                if index == u8::MAX {
                    assert!(!alphabet.contains(&byte));
                } else {
                    assert_eq!(alphabet[usize::from(index)], byte);
                }
            }
        }
    }
}
