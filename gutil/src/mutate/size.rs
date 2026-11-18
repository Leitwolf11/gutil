use std::fs::Metadata;
use std::io;

#[derive(Clone, Copy)]
pub(super) enum Adjustment {
    Exact,
    Extend,
    Reduce,
    AtMost,
    AtLeast,
    RoundDown,
    RoundUp,
}

#[derive(Clone, Copy)]
pub(super) struct Size {
    adjustment: Adjustment,
    amount: u64,
}

impl Size {
    pub(super) fn parse(mut value: &[u8]) -> Option<Self> {
        value = value.trim_ascii_start();
        let adjustment = match value.first()? {
            b'+' => Adjustment::Extend,
            b'-' => Adjustment::Reduce,
            b'<' => Adjustment::AtMost,
            b'>' => Adjustment::AtLeast,
            b'/' => Adjustment::RoundDown,
            b'%' => Adjustment::RoundUp,
            _ => Adjustment::Exact,
        };
        if !matches!(adjustment, Adjustment::Exact) {
            value = &value[1..];
            if !matches!(adjustment, Adjustment::Extend | Adjustment::Reduce) {
                value = value.trim_ascii_start();
            }
        }
        let split = value
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let (digits, suffix) = value.split_at(split);
        if digits.is_empty() && matches!(adjustment, Adjustment::Extend | Adjustment::Reduce) {
            return None;
        }
        let amount = if digits.is_empty() && !suffix.is_empty() {
            1u128
        } else {
            std::str::from_utf8(digits).ok()?.parse().ok()?
        };
        let multiplier = match suffix {
            b"" => 1,
            [prefix, ending @ ..] => {
                let power = b"KMGTPEZYRQ"
                    .iter()
                    .position(|unit| *unit == prefix.to_ascii_uppercase())?
                    + 1;
                let radix = match ending {
                    b"" | b"iB" => 1024u128,
                    b"B" => 1000,
                    _ => return None,
                };
                radix.checked_pow(u32::try_from(power).ok()?)?
            }
        };
        let amount = u64::try_from(amount.checked_mul(multiplier)?).ok()?;
        let maximum = if matches!(adjustment, Adjustment::Reduce) {
            i64::MIN.unsigned_abs()
        } else {
            i64::MAX.unsigned_abs()
        };
        if amount > maximum
            || amount == 0 && matches!(adjustment, Adjustment::RoundDown | Adjustment::RoundUp)
        {
            return None;
        }
        Some(Self { adjustment, amount })
    }

    pub(super) const fn relative(self) -> bool {
        !matches!(self.adjustment, Adjustment::Exact)
    }

    pub(super) fn apply(self, current: u64, block_size: u64) -> io::Result<u64> {
        let overflow = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "file size exceeds supported range",
            )
        };
        let amount = self.amount.checked_mul(block_size).ok_or_else(overflow)?;
        let result = match self.adjustment {
            Adjustment::Exact => Some(amount),
            Adjustment::Extend => current.checked_add(amount),
            Adjustment::Reduce => Some(current.saturating_sub(amount)),
            Adjustment::AtMost => Some(current.min(amount)),
            Adjustment::AtLeast => Some(current.max(amount)),
            Adjustment::RoundDown => Some(current / amount * amount),
            Adjustment::RoundUp => current.div_ceil(amount).checked_mul(amount),
        };
        result
            .filter(|size| *size <= i64::MAX.unsigned_abs())
            .ok_or_else(overflow)
    }
}

pub(super) struct Options {
    pub(super) size: Option<Size>,
    pub(super) reference: Option<std::path::PathBuf>,
    pub(super) no_create: bool,
    pub(super) io_blocks: bool,
}

impl Options {
    pub(super) fn prepare(&self) -> io::Result<Option<u64>> {
        self.reference
            .as_ref()
            .map(|path| {
                std::fs::metadata(path)
                    .map(|metadata| metadata.len())
                    .map_err(|error| {
                        io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                    })
            })
            .transpose()
    }

    pub(super) fn truncate(
        &self,
        path: &std::path::Path,
        reference: Option<u64>,
    ) -> io::Result<()> {
        let file = match std::fs::OpenOptions::new()
            .write(true)
            .create(!self.no_create)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if self.no_create && error.kind() == io::ErrorKind::NotFound => {
                return Ok(())
            }
            Err(error) => return Err(error),
        };
        let (current, block) = if self.io_blocks {
            let metadata = file.metadata()?;
            (
                reference.unwrap_or(metadata.len()),
                io_block_size(&metadata)?,
            )
        } else if reference.is_none() && self.size.is_some_and(Size::relative) {
            (file.metadata()?.len(), 1)
        } else {
            (reference.unwrap_or(0), 1)
        };
        let length = self
            .size
            .map_or(Ok(current), |size| size.apply(current, block))?;
        file.set_len(length)
    }
}

#[cfg(unix)]
fn io_block_size(metadata: &Metadata) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let block = metadata.blksize();
    if block == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "filesystem reports zero I/O block size",
        ));
    }
    Ok(block)
}

#[cfg(not(unix))]
fn io_block_size(_metadata: &Metadata) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "I/O block sizing is unavailable on this platform",
    ))
}
