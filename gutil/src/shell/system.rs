use std::env;
use std::ffi::{OsStr, OsString};
use std::future;
use std::io::Write;
use std::time::Duration;

use super::{Command, Error, Info};

pub(super) fn nproc(
    mut args: impl Iterator<Item = OsString>,
    output: &mut impl Write,
) -> Result<(), Error> {
    let mut installed = false;
    let mut ignored = 0;
    let mut flags = true;
    while let Some(argument) = args.next() {
        if !flags {
            return Err(Error::Operand(argument));
        }
        if let Some(info) = Info::parse(&argument) {
            return info.write(Command::Nproc, output).map_err(Error::from);
        }
        match argument.as_encoded_bytes() {
            b"--" => flags = false,
            b"--all" => installed = true,
            b"--ignore" => {
                let value = args.next().ok_or(Error::MissingValue("--ignore"))?;
                ignored = parse_ignored(&value)?;
            }
            [b'-', b'-', b'i', b'g', b'n', b'o', b'r', b'e', b'=', value @ ..] => {
                ignored = std::str::from_utf8(value)
                    .ok()
                    .and_then(ignored_decimal)
                    .ok_or(Error::Count(argument))?;
            }
            [b'-', ..] => return Err(Error::Option(argument)),
            _ => return Err(Error::Operand(argument)),
        }
    }
    let count = if installed {
        installed_count().unwrap_or_else(available_count)
    } else {
        let count = omp("OMP_NUM_THREADS").unwrap_or_else(available_count);
        count.min(omp("OMP_THREAD_LIMIT").unwrap_or(u64::MAX))
    };
    writeln!(output, "{}", count.saturating_sub(ignored).max(1))?;
    Ok(())
}

fn parse_ignored(value: &OsStr) -> Result<u64, Error> {
    value
        .to_str()
        .and_then(ignored_decimal)
        .ok_or_else(|| Error::Count(value.into()))
}

fn ignored_decimal(value: &str) -> Option<u64> {
    let value = value.trim_start();
    decimal(value.strip_prefix('+').unwrap_or(value))
}

fn decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(text.parse().unwrap_or(u64::MAX))
}

fn omp(key: &str) -> Option<u64> {
    let value = env::var(key).ok()?;
    decimal(value.split(',').next()?.trim()).filter(|&value| value != 0)
}

fn available_count() -> u64 {
    std::thread::available_parallelism().map_or(1, |available| available.get() as u64)
}

#[cfg(target_os = "linux")]
fn installed_count() -> Option<u64> {
    cpu_list_count(&std::fs::read_to_string("/sys/devices/system/cpu/present").ok()?)
}

#[cfg(target_os = "linux")]
fn cpu_list_count(list: &str) -> Option<u64> {
    list.trim().split(',').try_fold(0u64, |total, range| {
        let count = if let Some((first, last)) = range.split_once('-') {
            decimal(last)?
                .checked_sub(decimal(first)?)?
                .checked_add(1)?
        } else {
            decimal(range)?;
            1
        };
        total.checked_add(count)
    })
}

#[cfg(not(target_os = "linux"))]
const fn installed_count() -> Option<u64> {
    None
}

pub(super) fn sleep(
    args: impl Iterator<Item = OsString>,
    output: &mut impl Write,
) -> Result<(), Error> {
    let mut flags = true;
    let mut total = None;
    for argument in args {
        if flags {
            if let Some(info) = Info::parse(&argument) {
                return info.write(Command::Sleep, output).map_err(Error::from);
            }
            if argument == "--" {
                flags = false;
                continue;
            }
        }
        let seconds = argument
            .to_str()
            .and_then(parse_duration)
            .ok_or(Error::Duration(argument))?;
        total = Some(total.unwrap_or(0.0) + seconds);
    }
    let seconds = total.ok_or(Error::MissingDuration)?;
    if seconds == 0.0 {
        return Ok(());
    }
    let clock = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    clock.block_on(wait_for_duration(seconds));
    Ok(())
}

async fn wait_for_duration(seconds: f64) {
    // Use bounded timer slots independent of the platform's Instant range
    // and the Tokio timer wheel's maximum interval.
    const MAX_WAIT: Duration = Duration::from_hours(24);
    let Ok(mut remaining) = Duration::try_from_secs_f64(seconds) else {
        future::pending::<()>().await;
        return;
    };
    while !remaining.is_zero() {
        let next = remaining.min(MAX_WAIT);
        tokio::time::sleep(next).await;
        remaining -= next;
    }
}

fn parse_duration(argument: &str) -> Option<f64> {
    let (number, scale) = match argument.as_bytes().last()? {
        b's' => (&argument[..argument.len() - 1], 1.0),
        b'm' => (&argument[..argument.len() - 1], 60.0),
        b'h' => (&argument[..argument.len() - 1], 3600.0),
        b'd' => (&argument[..argument.len() - 1], 86_400.0),
        _ => (argument, 1.0),
    };
    let value = number
        .trim_start()
        .parse::<f64>()
        .ok()
        .or_else(|| parse_hex(number.trim_start()))?;
    (value >= 0.0).then_some(value * scale)
}

fn parse_hex(text: &str) -> Option<f64> {
    let text = text.strip_prefix('+').unwrap_or(text);
    let mantissa = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))?;
    let (digits, exponent) = match mantissa.split_once(['p', 'P']) {
        Some((digits, exponent)) => (digits, exponent.parse::<i32>().ok()?),
        None => (mantissa, 0),
    };
    let mut whole = 0.0_f64;
    let mut fraction = 1.0;
    let mut point = false;
    let mut seen_digit = false;
    for symbol in digits.chars() {
        if symbol == '.' && !point {
            point = true;
            continue;
        }
        let digit = f64::from(symbol.to_digit(16)?);
        if point {
            fraction /= 16.0;
            whole += digit * fraction;
        } else {
            whole = whole.mul_add(16.0, digit);
        }
        seen_digit = true;
    }
    seen_digit.then(|| {
        if whole == 0.0 {
            0.0
        } else {
            whole * 2.0_f64.powi(exponent)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{wait_for_duration, Duration};

    #[test]
    #[allow(clippy::unwrap_used)]
    fn very_large_durations_remain_pending_without_panicking() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        for seconds in [f64::INFINITY, 1e12, 1e100] {
            assert!(runtime
                .block_on(async {
                    tokio::time::timeout(Duration::from_millis(1), wait_for_duration(seconds)).await
                })
                .is_err());
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn counts_cpu_lists_with_ranges_and_individual_entries() {
        assert_eq!(super::cpu_list_count("0-3,8,12-13\n"), Some(7));
        assert_eq!(super::cpu_list_count("4-2"), None);
    }
}
