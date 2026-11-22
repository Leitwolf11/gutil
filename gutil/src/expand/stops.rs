use std::io;
use std::num::NonZeroU64;

const MAX_COLUMN: u64 = i64::MAX as u64;

pub(super) enum Stops {
    Every(NonZeroU64),
    Custom {
        columns: Vec<u64>,
        remainder: Remainder,
    },
}

pub(super) enum Remainder {
    SingleSpace,
    Aligned { origin: u64, period: NonZeroU64 },
}

#[derive(Default)]
pub(super) struct Builder {
    columns: Vec<u64>,
    absolute_period: Option<NonZeroU64>,
    relative_period: Option<NonZeroU64>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Invalid {
    #[error("expected a tab size or a list of positive tab positions")]
    Number,
    #[error("tab sizes must be positive and strictly ascending")]
    Order,
    #[error("the '/' and '+' tab intervals are mutually exclusive")]
    Both,
    #[error("a repeating tab size may appear only once at the end of a list")]
    RepeatedTail,
}

#[derive(Clone, Copy)]
enum Prefix {
    Position,
    Absolute,
    Relative,
}

impl Builder {
    pub(super) fn add(&mut self, list: &[u8]) -> Result<(), Invalid> {
        let mut prefix = Prefix::Position;
        for token in list.split(|byte| matches!(byte, b',' | b' ' | b'\t')) {
            let digits = token
                .iter()
                .position(|byte| !matches!(byte, b'+' | b'/'))
                .unwrap_or(token.len());
            if let Some(&marker) = token[..digits].last() {
                prefix = if marker == b'/' {
                    Prefix::Absolute
                } else {
                    Prefix::Relative
                };
            }
            let digits = &token[digits..];
            if digits.is_empty() {
                continue;
            }
            let number = digits.iter().try_fold(0u64, |value, &byte| {
                if !byte.is_ascii_digit() {
                    return Err(Invalid::Number);
                }
                value
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(u64::from(byte - b'0')))
                    .filter(|&value| value <= MAX_COLUMN)
                    .ok_or(Invalid::Number)
            })?;
            let target = match prefix {
                Prefix::Position => {
                    self.columns.push(number);
                    continue;
                }
                Prefix::Absolute => &mut self.absolute_period,
                Prefix::Relative => &mut self.relative_period,
            };
            if target.is_some() {
                return Err(Invalid::RepeatedTail);
            }
            *target = NonZeroU64::new(number);
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Result<Stops, Invalid> {
        if self.columns.first() == Some(&0)
            || self.columns.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(Invalid::Order);
        }
        if self.absolute_period.is_some() && self.relative_period.is_some() {
            return Err(Invalid::Both);
        }
        let origin = self.columns.last().copied().unwrap_or(0);
        let remainder = if let Some(period) = self.absolute_period {
            Remainder::Aligned { origin: 0, period }
        } else if let Some(period) = self.relative_period {
            Remainder::Aligned { origin, period }
        } else {
            Remainder::SingleSpace
        };
        match (self.columns.as_slice(), &remainder) {
            ([], Remainder::Aligned { period, .. }) => Ok(Stops::Every(*period)),
            ([], Remainder::SingleSpace) => {
                const DEFAULT: NonZeroU64 = NonZeroU64::new(8).unwrap();
                Ok(Stops::Every(DEFAULT))
            }
            ([single], Remainder::SingleSpace) => Ok(Stops::Every(
                NonZeroU64::new(*single).ok_or(Invalid::Order)?,
            )),
            _ => Ok(Stops::Custom {
                columns: self.columns,
                remainder,
            }),
        }
    }
}

impl Stops {
    pub(super) fn next(&self, column: u64) -> io::Result<u64> {
        let distance = match self {
            Self::Every(period) => period.get() - column % period.get(),
            Self::Custom { columns, remainder } => {
                let index = columns.partition_point(|&stop| stop <= column);
                if let Some(&stop) = columns.get(index) {
                    return Ok(stop);
                }
                match remainder {
                    Remainder::SingleSpace => 1,
                    Remainder::Aligned { origin, period } => {
                        period.get() - (column - origin) % period.get()
                    }
                }
            }
        };
        advance(column, distance)
    }
}

pub(super) fn advance(column: u64, width: u64) -> io::Result<u64> {
    column
        .checked_add(width)
        .filter(|&column| column <= MAX_COLUMN)
        .ok_or_else(|| io::Error::other("input line is too long"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
#[test]
fn refuses_a_tab_stop_past_the_maximum_column() {
    let mut builder = Builder::default();
    builder.add(b"8").unwrap();
    assert_eq!(
        builder
            .finish()
            .unwrap()
            .next(MAX_COLUMN)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Other
    );
}
