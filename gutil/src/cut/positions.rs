use std::ops::Range;

#[derive(Debug, thiserror::Error)]
#[error("expected one-based, ascending positions separated by commas or blanks")]
pub(super) struct InvalidList;

pub(super) fn parse(list: &[u8], complement: bool) -> Result<Vec<Range<usize>>, InvalidList> {
    let mut ranges = Vec::<Range<usize>>::new();
    for token in list.split(|byte| matches!(byte, b',' | b' ' | b'\t')) {
        let dash = memchr::memchr(b'-', token);
        let (start, end) = if let Some(dash) = dash {
            let left = &token[..dash];
            let right = &token[dash + 1..];
            match (left.is_empty(), right.is_empty()) {
                (true, true) => return Err(InvalidList),
                (true, false) => (1, position(right)?),
                (false, true) => (position(left)?, usize::MAX),
                (false, false) => (position(left)?, position(right)?),
            }
        } else {
            let number = position(token)?;
            (number, number)
        };
        if end < start {
            return Err(InvalidList);
        }
        ranges.push(start - 1..end);
    }
    ranges.sort_unstable_by_key(|range| range.start);
    let mut merged = Vec::<Range<usize>>::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| range.start < previous.end)
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    if !complement {
        return Ok(merged);
    }
    let mut gaps = Vec::new();
    let mut first = 0;
    for range in merged {
        if first < range.start {
            gaps.push(first..range.start);
        }
        first = range.end;
    }
    if first < usize::MAX {
        gaps.push(first..usize::MAX);
    }
    Ok(gaps)
}

fn position(bytes: &[u8]) -> Result<usize, InvalidList> {
    let number = bytes.iter().try_fold(0usize, |value, &byte| {
        if !byte.is_ascii_digit() {
            return None;
        }
        value.checked_mul(10)?.checked_add(usize::from(byte - b'0'))
    });
    number.filter(|&number| number != 0).ok_or(InvalidList)
}
