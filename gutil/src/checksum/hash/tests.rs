#![allow(clippy::unwrap_used)]

use std::fmt::Write;
use std::io::{self, Cursor, Read};

use proptest::prelude::*;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

use super::{digest, Algorithm};

const SHA2: [Algorithm; 4] = [
    Algorithm::Sha224,
    Algorithm::Sha256,
    Algorithm::Sha384,
    Algorithm::Sha512,
];

fn reference(algorithm: Algorithm, bytes: &[u8]) -> String {
    let hash = match algorithm {
        Algorithm::Sha224 => Sha224::digest(bytes).to_vec(),
        Algorithm::Sha256 => Sha256::digest(bytes).to_vec(),
        Algorithm::Sha384 => Sha384::digest(bytes).to_vec(),
        Algorithm::Sha512 => Sha512::digest(bytes).to_vec(),
        _ => unreachable!("SHA-2 oracle"),
    };
    hash.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}

struct InterruptedChunks {
    bytes: Cursor<Vec<u8>>,
    chunk: usize,
    interrupt: bool,
}

impl Read for InterruptedChunks {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.interrupt = !self.interrupt;
        if self.interrupt {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let size = self.chunk.min(buffer.len());
        self.bytes.read(&mut buffer[..size])
    }
}

fn check(bytes: &[u8], chunk: usize) {
    for algorithm in SHA2 {
        let mut input = InterruptedChunks {
            bytes: Cursor::new(bytes.to_vec()),
            chunk,
            interrupt: false,
        };
        let mut total = 0;
        let actual = digest(algorithm, &mut input, &mut [0; 8192], |read| total += read).unwrap();
        assert_eq!(actual, reference(algorithm, bytes));
        assert_eq!(total, bytes.len() as u64);
    }
}

#[test]
fn matches_rustcrypto_at_padding_block_and_read_boundaries() {
    for length in [
        0, 1, 55, 56, 63, 64, 65, 111, 112, 127, 128, 129, 8191, 8192, 8193, 65_537,
    ] {
        let bytes: Vec<_> = (0..=255).cycle().take(length).collect();
        check(&bytes, 37);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn matches_rustcrypto_for_random_input_and_short_reads(
        bytes in prop::collection::vec(any::<u8>(), 0..32_769),
        chunk in 1_usize..8193,
    ) {
        check(&bytes, chunk);
    }
}

#[test]
fn propagates_read_errors_without_returning_a_partial_digest() {
    struct Failed;
    impl Read for Failed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }
    }

    for algorithm in [Algorithm::Md5, Algorithm::Sha1].into_iter().chain(SHA2) {
        let mut progress = 0;
        let error = digest(algorithm, &mut Failed, &mut [0; 8192], |read| {
            progress += read;
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(progress, 0);
    }
}
