use std::io::{self, Read};

use aws_lc_rs::digest::{self, Context};
use md5::{Digest, Md5};
use sha1::Sha1;

use super::Algorithm;

pub(super) fn digest(
    algorithm: Algorithm,
    input: &mut dyn Read,
    buffer: &mut [u8],
    advance: impl FnMut(u64),
) -> io::Result<String> {
    match algorithm {
        Algorithm::Md5 => read_digest::<Md5>(input, buffer, advance),
        Algorithm::Sha1 => read_digest::<Sha1>(input, buffer, advance),
        Algorithm::Sha224 => read_native(&digest::SHA224, input, buffer, advance),
        Algorithm::Sha256 => read_native(&digest::SHA256, input, buffer, advance),
        Algorithm::Sha384 => read_native(&digest::SHA384, input, buffer, advance),
        Algorithm::Sha512 => read_native(&digest::SHA512, input, buffer, advance),
    }
}

fn read_digest<D: Digest>(
    input: &mut dyn Read,
    buffer: &mut [u8],
    advance: impl FnMut(u64),
) -> io::Result<String> {
    let mut digest = D::new();
    read_chunks(input, buffer, advance, |bytes| digest.update(bytes))?;
    Ok(hex(&digest.finalize()))
}

fn read_native(
    algorithm: &'static digest::Algorithm,
    input: &mut dyn Read,
    buffer: &mut [u8],
    advance: impl FnMut(u64),
) -> io::Result<String> {
    let mut context = Context::new(algorithm);
    read_chunks(input, buffer, advance, |bytes| context.update(bytes))?;
    Ok(hex(context.finish().as_ref()))
}

fn read_chunks(
    input: &mut dyn Read,
    buffer: &mut [u8],
    mut advance: impl FnMut(u64),
    mut update: impl FnMut(&[u8]),
) -> io::Result<()> {
    loop {
        match input.read(buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => {
                update(&buffer[..read]);
                advance(read as u64);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 15)]));
    }
    hex
}

#[cfg(test)]
mod tests;
