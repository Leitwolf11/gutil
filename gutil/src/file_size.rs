use std::fs::{File, Metadata};

pub(crate) fn verified_length(file: &File, metadata: &Metadata) -> Option<u64> {
    let length = metadata.len();
    if !metadata.is_file() || length == 0 {
        return None;
    }
    if confirms_boundary(file, length) {
        return Some(length);
    }
    None
}

#[cfg(unix)]
fn confirms_boundary(file: &File, length: u64) -> bool {
    use std::os::unix::fs::FileExt;

    // Regular-looking virtual files may report a synthetic size. These reads
    // leave the cursor untouched so callers can fall back to streaming.
    let mut byte = [0];
    matches!(file.read_at(&mut byte, length - 1), Ok(1))
        && matches!(file.read_at(&mut byte, length), Ok(0))
}

#[cfg(not(unix))]
const fn confirms_boundary(_file: &File, _length: u64) -> bool {
    true
}
