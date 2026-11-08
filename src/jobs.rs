use std::ffi::OsString;
use std::io;
use std::num::NonZeroUsize;
use std::thread;

const MAX_WORKERS: usize = 512;

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
        .map(|value| value.get().min(MAX_WORKERS))
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

    // Start at most one batch per worker, including for small input sets.
    let batch_count = count.min(inputs.len());
    let batch_size = inputs.len() / batch_count;
    let larger_batches = inputs.len() % batch_count;
    let mut output = Vec::with_capacity(inputs.len());
    let mut inputs = inputs.into_iter();
    let mut pending = Vec::with_capacity(batch_count);
    for index in 0..batch_count {
        let size = batch_size + usize::from(index < larger_batches);
        let batch: Vec<_> = inputs.by_ref().take(size).collect();
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use parking_lot::{Condvar, Mutex};

    use super::map;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn small_batch_uses_available_workers_in_input_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(4)
            .build()
            .unwrap();
        let rendezvous = Arc::new((Mutex::new(0), Condvar::new()));

        let result = runtime.block_on(map(vec![0, 1, 2, 3], 4, move |value| {
            let (started, ready) = &*rendezvous;
            let mut started = started.lock();
            *started += 1;
            ready.notify_all();
            ready.wait_while_for(&mut started, |count| *count < 4, Duration::from_secs(5));
            (value, *started)
        }));

        assert_eq!(result.unwrap(), [(0, 4), (1, 4), (2, 4), (3, 4)]);
    }
}
