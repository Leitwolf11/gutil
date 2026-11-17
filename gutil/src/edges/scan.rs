use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};

use memchr::{memchr_iter, memrchr_iter};

use super::{Edge, Options, Origin, Unit};

const BLOCK: usize = 16 * 1024;

pub(super) fn seekable(
    edge: Edge,
    options: &Options,
    file: &mut File,
    length: u64,
    output: &mut impl Write,
) -> io::Result<()> {
    let limit = options.limit;
    let start = match limit.unit {
        Unit::Bytes => length.saturating_sub(limit.count),
        Unit::Records => suffix_offset(file, length, limit.count, options.delimiter)?,
    };
    if edge == Edge::Head {
        file.seek(SeekFrom::Start(0))?;
        io::copy(&mut file.take(start), output)?;
    } else {
        file.seek(SeekFrom::Start(start))?;
        io::copy(file, output)?;
    }
    Ok(())
}

fn suffix_offset(file: &mut File, mut end: u64, mut count: u64, delimiter: u8) -> io::Result<u64> {
    if count == 0 {
        return Ok(end);
    }
    let mut buffer = [0u8; BLOCK];
    let mut last_block = true;
    while end != 0 {
        let start = end.saturating_sub(BLOCK as u64);
        let length = usize::try_from(end - start).map_err(io::Error::other)?;
        let block = &mut buffer[..length];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(block)?;
        let scan = if last_block && block.last() == Some(&delimiter) {
            &block[..length - 1]
        } else {
            block
        };
        last_block = false;
        for position in memrchr_iter(delimiter, scan) {
            count -= 1;
            if count == 0 {
                return Ok(start + position as u64 + 1);
            }
        }
        end = start;
    }
    Ok(0)
}

pub(super) fn stream(
    edge: Edge,
    options: &Options,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<()> {
    let limit = options.limit;
    match (limit.unit, limit.origin, edge) {
        (Unit::Bytes, Origin::Start, Edge::Head) => {
            io::copy(&mut input.take(limit.count), output)?;
        }
        (Unit::Bytes, Origin::Start, Edge::Tail) => {
            io::copy(&mut input.take(limit.count), &mut io::sink())?;
            io::copy(input, output)?;
        }
        (Unit::Records, Origin::Start, Edge::Head) => {
            copy_prefix(input, output, limit.count, options.delimiter)?;
        }
        (Unit::Records, Origin::Start, Edge::Tail) => {
            copy_prefix(input, &mut io::sink(), limit.count, options.delimiter)?;
            io::copy(input, output)?;
        }
        (_, Origin::End, Edge::Head) if limit.count == 0 => {
            io::copy(input, output)?;
        }
        (_, Origin::End, Edge::Tail) if limit.count == 0 => {
            if options.follow.mode.is_some() {
                io::copy(input, &mut io::sink())?;
            }
        }
        (Unit::Bytes, Origin::End, _) => suffix_bytes(edge, input, output, limit.count)?,
        (Unit::Records, Origin::End, _) => {
            suffix_records(edge, input, output, limit.count, options.delimiter)?;
        }
    }
    Ok(())
}

fn copy_prefix(
    input: &mut impl BufRead,
    output: &mut impl Write,
    mut count: u64,
    delimiter: u8,
) -> io::Result<()> {
    while count != 0 {
        let block = input.fill_buf()?;
        if block.is_empty() {
            break;
        }
        let mut length = block.len();
        for offset in memchr_iter(delimiter, block) {
            count -= 1;
            if count == 0 {
                length = offset + 1;
                break;
            }
        }
        output.write_all(&block[..length])?;
        input.consume(length);
    }
    Ok(())
}

fn suffix_records(
    edge: Edge,
    input: &mut impl BufRead,
    output: &mut impl Write,
    count: u64,
    delimiter: u8,
) -> io::Result<()> {
    let mut blocks = VecDeque::<(Vec<u8>, u64)>::new();
    let mut buffer = Vec::new();
    let mut delimiters = 0;
    loop {
        buffer.resize(BLOCK, 0);
        let read = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        buffer.truncate(read);
        let found = memchr_iter(delimiter, &buffer).count() as u64;
        blocks.push_back((std::mem::take(&mut buffer), found));
        delimiters += found;

        // Keep a separator before the selected suffix, even if the final byte
        // turns out to be a delimiter terminating the last record.
        while blocks
            .front()
            .is_some_and(|(_, found)| delimiters - found > count)
        {
            let Some((block, found)) = blocks.pop_front() else {
                break;
            };
            delimiters -= found;
            if edge == Edge::Head {
                output.write_all(&block)?;
            }
            buffer = block;
        }
    }

    let ends_with_delimiter = blocks
        .back()
        .is_some_and(|(block, _)| block.last() == Some(&delimiter));
    let mut remaining = count;
    for (index, (block, _)) in blocks.iter().enumerate().rev() {
        let scan = if ends_with_delimiter && index + 1 == blocks.len() {
            &block[..block.len() - 1]
        } else {
            block
        };
        for position in memrchr_iter(delimiter, scan) {
            remaining -= 1;
            if remaining == 0 {
                let split = position + 1;
                match edge {
                    Edge::Head => {
                        for (prefix, _) in blocks.iter().take(index) {
                            output.write_all(prefix)?;
                        }
                        output.write_all(&block[..split])?;
                    }
                    Edge::Tail => {
                        output.write_all(&block[split..])?;
                        for (suffix, _) in blocks.iter().skip(index + 1) {
                            output.write_all(suffix)?;
                        }
                    }
                }
                return Ok(());
            }
        }
    }
    if edge == Edge::Tail {
        for (block, _) in blocks {
            output.write_all(&block)?;
        }
    }
    Ok(())
}

fn suffix_bytes(
    edge: Edge,
    input: &mut impl BufRead,
    output: &mut impl Write,
    count: u64,
) -> io::Result<()> {
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    let mut bytes = VecDeque::new();
    loop {
        let incoming = input.fill_buf()?;
        if incoming.is_empty() {
            break;
        }
        let excess = bytes
            .len()
            .saturating_add(incoming.len())
            .saturating_sub(count);
        let retained = excess.min(bytes.len());
        if edge == Edge::Head {
            let (left, right) = bytes.as_slices();
            let split = retained.min(left.len());
            output.write_all(&left[..split])?;
            output.write_all(&right[..retained - split])?;
            output.write_all(&incoming[..excess - retained])?;
        }
        bytes.drain(..retained);
        bytes.extend(&incoming[excess - retained..]);
        let length = incoming.len();
        input.consume(length);
    }
    if edge == Edge::Tail {
        let (left, right) = bytes.as_slices();
        output.write_all(left)?;
        output.write_all(right)?;
    }
    Ok(())
}
