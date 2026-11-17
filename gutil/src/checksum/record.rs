use std::borrow::Cow;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::PathBuf;

use super::{Algorithm, InputMode, Options};

pub(super) fn write(
    output: &mut dyn Write,
    algorithm: Algorithm,
    digest: &[u8],
    name: &[u8],
    options: &Options,
) -> io::Result<()> {
    let name = if options.zero {
        Cow::Borrowed(name)
    } else {
        escape(name)
    };
    if matches!(name, Cow::Owned(_)) {
        output.write_all(b"\\")?;
    }
    if options.tagged {
        write!(output, "{} (", algorithm.tag())?;
        output.write_all(&name)?;
        output.write_all(b") = ")?;
        output.write_all(digest)?;
    } else {
        output.write_all(digest)?;
        output.write_all(if options.mode == InputMode::Binary {
            b" *"
        } else {
            b"  "
        })?;
        output.write_all(&name)?;
    }
    output.write_all(if options.zero { b"\0" } else { b"\n" })
}

pub(super) fn escape(name: &[u8]) -> Cow<'_, [u8]> {
    if !name
        .iter()
        .any(|byte| matches!(byte, b'\\' | b'\n' | b'\r'))
    {
        return Cow::Borrowed(name);
    }
    let mut escaped = Vec::with_capacity(name.len());
    for &byte in name {
        match byte {
            b'\\' => escaped.extend_from_slice(b"\\\\"),
            b'\n' => escaped.extend_from_slice(b"\\n"),
            b'\r' => escaped.extend_from_slice(b"\\r"),
            _ => escaped.push(byte),
        }
    }
    Cow::Owned(escaped)
}

pub(super) struct Entry<'a> {
    pub(super) name: Cow<'a, [u8]>,
    pub(super) digest: &'a [u8],
}

pub(super) fn parse(algorithm: Algorithm, line: &[u8]) -> Option<Entry<'_>> {
    let (line, escaped) = line
        .strip_prefix(b"\\")
        .map_or((line, false), |line| (line, true));
    let (name, digest) = if let Some(tagged) = line.strip_prefix(algorithm.tag().as_bytes()) {
        let tagged = tagged.strip_prefix(b" (")?;
        let split = tagged.iter().rposition(|&byte| byte == b')')?;
        let (name, tail) = tagged.split_at(split);
        (name, tail.strip_prefix(b") = ")?)
    } else {
        let (digest, rest) = line.split_at_checked(hex_len(algorithm))?;
        let rest = rest.strip_prefix(b" ")?;
        (
            rest.strip_prefix(b" ")
                .or_else(|| rest.strip_prefix(b"*"))?,
            digest,
        )
    };
    if digest.len() != hex_len(algorithm)
        || !digest.iter().all(u8::is_ascii_hexdigit)
        || name.is_empty()
        || name.contains(&0)
    {
        return None;
    }
    let name = if escaped {
        Cow::Owned(unescape(name)?)
    } else {
        Cow::Borrowed(name)
    };
    Some(Entry { name, digest })
}

const fn hex_len(algorithm: Algorithm) -> usize {
    match algorithm {
        Algorithm::Md5 => 32,
        Algorithm::Sha1 => 40,
        Algorithm::Sha224 => 56,
        Algorithm::Sha256 => 64,
        Algorithm::Sha384 => 96,
        Algorithm::Sha512 => 128,
    }
}

fn unescape(name: &[u8]) -> Option<Vec<u8>> {
    let mut name = name.iter().copied();
    let mut decoded = Vec::with_capacity(name.len());
    while let Some(byte) = name.next() {
        decoded.push(match byte {
            b'\\' => match name.next()? {
                b'\\' => b'\\',
                b'n' => b'\n',
                b'r' => b'\r',
                _ => return None,
            },
            byte => byte,
        });
    }
    Some(decoded)
}

#[cfg_attr(unix, allow(clippy::unnecessary_wraps))]
pub(super) fn path(name: &[u8]) -> io::Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(PathBuf::from(std::ffi::OsStr::from_bytes(name)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(name)
            .map(PathBuf::from)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

#[cfg_attr(unix, allow(clippy::unnecessary_wraps))]
pub(super) fn filename(path: &OsStr) -> io::Result<&[u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Ok(path.as_bytes())
    }
    #[cfg(not(unix))]
    {
        path.to_str().map(str::as_bytes).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "file name is not valid UTF-8")
        })
    }
}
