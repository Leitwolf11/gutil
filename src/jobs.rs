use std::ffi::OsString;
use std::io;
use std::num::NonZeroUsize;
use std::thread;

#[derive(Debug, thiserror::Error)]
#[error("job count must be a positive integer")]
pub(crate) struct InvalidCount;

pub(crate) fn default_count() -> usize {
    thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .min(8)
}

pub(crate) fn option(
    argument: &[u8],
    remaining: &mut impl Iterator<Item = OsString>,
) -> Option<Result<usize, InvalidCount>> {
    match argument {
        b"-j" | b"--jobs" | b"--threads" => Some(read_count(None, remaining)),
        _ => [b"-j".as_slice(), b"--jobs=", b"--threads="]
            .into_iter()
            .find_map(|prefix| argument.strip_prefix(prefix))
            .map(|value| read_count(Some(value), remaining)),
    }
}

pub(crate) fn short_value(
    value: &[u8],
    remaining: &mut impl Iterator<Item = OsString>,
) -> Result<usize, InvalidCount> {
    read_count((!value.is_empty()).then_some(value), remaining)
}

fn read_count(
    attached: Option<&[u8]>,
    remaining: &mut impl Iterator<Item = OsString>,
) -> Result<usize, InvalidCount> {
    let separate;
    let value = if let Some(value) = attached {
        value
    } else {
        separate = remaining.next().ok_or(InvalidCount)?;
        separate.as_encoded_bytes()
    };
    let value = std::str::from_utf8(value).map_err(|_| InvalidCount)?;
    value
        .parse::<NonZeroUsize>()
        .map(NonZeroUsize::get)
        .map_err(|_| InvalidCount)
}

pub(crate) async fn map<T, U, F>(inputs: Vec<T>, count: usize, work: F) -> io::Result<Vec<U>>
where
    T: Send + 'static,
    U: Send + 'static,
    F: Fn(T) -> U + Clone + Send + 'static,
{
    if inputs.is_empty() {
        return Ok(Vec::new());
    }

    // Batch cheap filesystem calls and retain the original input order.
    let chunk_size = inputs.len().div_ceil(count).max(16);
    let mut output = Vec::with_capacity(inputs.len());
    let mut inputs = inputs.into_iter();
    let mut pending = Vec::new();
    while inputs.len() > 0 {
        let batch: Vec<_> = inputs.by_ref().take(chunk_size).collect();
        let work = work.clone();
        pending.push(tokio::task::spawn_blocking(move || {
            batch.into_iter().map(work).collect::<Vec<_>>()
        }));
    }
    for task in pending {
        output.extend(task.await.map_err(io::Error::other)?);
    }
    Ok(output)
}
