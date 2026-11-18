use std::path::Path;
use std::process::{Command, Output};

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;

fn reference(directory: &Path, utility: &str, args: &[&str], input: &[u8]) -> Output {
    let mut command = Command::new(directory.join(utility));
    command.args(args).env("LC_ALL", "C");
    super::capture(command, input)
}

fn compare(root: &Path, directory: &Path, utility: &str, args: &[&str], input: &[u8]) {
    let mut actual = vec!["--progress=never", utility];
    actual.extend_from_slice(args);
    let actual = super::convert(root, &actual, input);
    let expected = reference(directory, utility, args, input);
    assert_eq!(
        actual.status.code(),
        expected.status.code(),
        "{utility} {args:?}"
    );
    assert_eq!(actual.stdout, expected.stdout, "{utility} {args:?}");
}

#[test]
#[ignore = "requires GNU Coreutils 9.12 binaries in GNU_COREUTILS_DIR"]
fn generated_cases_match_gnu_encoding_and_decoding() {
    let directory = std::env::var_os("GNU_COREUTILS_DIR").unwrap();
    let directory = Path::new(&directory);
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let mut generator = TestRunner::deterministic();
    let mut comparisons = 0;
    for utility in ["base32", "base64"] {
        let version = reference(directory, utility, &["--version"], b"");
        assert!(version.status.success());
        assert!(String::from_utf8_lossy(&version.stdout)
            .lines()
            .next()
            .unwrap()
            .ends_with(" 9.12"));
        for length in [0, 1, 2, 3, 4, 5, 7, 8, 63, 64, 4095, 61441, 83217] {
            let input = proptest::collection::vec(any::<u8>(), length)
                .new_tree(&mut generator)
                .unwrap()
                .current();
            for width in [0, 1, 17, 76, 128] {
                let wrap = format!("--wrap={width}");
                compare(root, directory, utility, &[&wrap], &input);
                let encoded = reference(directory, utility, &[&wrap], &input);
                compare(root, directory, utility, &["-d"], &encoded.stdout);
                comparisons += 2;
            }
        }
        let malformed: &[&[u8]] = if utility == "base64" {
            &[
                b"Zg=",
                b"Zh==",
                b"Zm9v$YmFy",
                b"Zg==\r\n",
                b"Zg==Yg==",
                b"Zg\n==",
                b"A===",
                b"====",
                b"e88$",
            ]
        } else {
            &[
                b"MY=====",
                b"MZ======",
                b"MY======MI======",
                b"MZXW6!!!",
                b"MY======\r\n",
                b"MY",
                b"my======",
                b"========",
            ]
        };
        for input in malformed {
            for args in [&["--decode"][..], &["--decode", "--ignore-garbage"]] {
                compare(root, directory, utility, args, input);
                comparisons += 1;
            }
        }
    }
    assert_eq!(comparisons, 294);
}
