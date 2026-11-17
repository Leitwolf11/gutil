#![allow(clippy::unwrap_used)]

use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(root: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .args(["--no-config", "--color=never"])
        .args(args);
    command
}

fn convert(root: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = command(root, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            if let Err(error) = stdin.write_all(input) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let result = child.wait_with_output().unwrap();
        writer.join().unwrap();
        result
    })
}

fn check(output: &Output, code: i32, data: &[u8]) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, data);
}

#[test]
fn encodes_and_decodes_rfc4648_examples() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (raw, base32, base64) in [
        ("", "", ""),
        ("f", "MY======", "Zg=="),
        ("fo", "MZXQ====", "Zm8="),
        ("foo", "MZXW6===", "Zm9v"),
        ("foob", "MZXW6YQ=", "Zm9vYg=="),
        ("fooba", "MZXW6YTB", "Zm9vYmE="),
        ("foobar", "MZXW6YTBOI======", "Zm9vYmFy"),
    ] {
        for (utility, coded) in [("base32", base32), ("base64", base64)] {
            check(
                &convert(root, &["--progress=never", utility, "-w0"], raw.as_bytes()),
                0,
                coded.as_bytes(),
            );
            check(
                &convert(root, &["--progress=never", utility, "-d"], coded.as_bytes()),
                0,
                raw.as_bytes(),
            );
        }
    }
}

#[test]
fn wraps_across_chunks_and_round_trips_binary_files_and_pipes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let input: Vec<u8> = (0u8..=255).cycle().take(131_203).collect();
    std::fs::write(root.join("input"), &input).unwrap();
    for utility in ["base32", "base64"] {
        let wrapped = convert(
            root,
            &[
                "--progress=never",
                utility,
                "--threads=1",
                "-w",
                "17",
                "--",
                "input",
            ],
            b"",
        );
        assert!(wrapped.status.success());
        let lines: Vec<_> = wrapped
            .stdout
            .strip_suffix(b"\n")
            .unwrap()
            .split(|&byte| byte == b'\n')
            .collect();
        assert!(lines.len() > 100);
        assert!(lines[..lines.len() - 1].iter().all(|line| line.len() == 17));
        assert!(wrapped.stdout.ends_with(b"\n"));
        check(
            &convert(
                root,
                &["--progress=never", utility, "-dj1", "-"],
                &wrapped.stdout,
            ),
            0,
            &input,
        );
    }
}

#[test]
fn supports_padding_newlines_and_garbage_recovery() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (utility, input, output) in [
        ("base64", b"Zg==Yg==".as_slice(), b"fb".as_slice()),
        ("base64", b"Z\ng", b"f"),
        ("base32", b"MY======MI======", b"fb"),
        ("base32", b"M\nY", b"f"),
    ] {
        check(
            &convert(root, &["--progress=never", utility, "--decode"], input),
            0,
            output,
        );
    }
    for (utility, junk, expected) in [
        ("base64", b"Zg==\r\n$".as_slice(), b"f".as_slice()),
        ("base32", b"MY=====!=", b"f"),
    ] {
        check(
            &convert(root, &["--progress=never", utility, "-di"], junk),
            0,
            expected,
        );
    }
    for (utility, input, prefix) in [
        ("base64", b"Zg=".as_slice(), b"f".as_slice()),
        ("base64", b"Zh==", b"f"),
        ("base64", b"Zm9v$YmFy", b"foo"),
        ("base64", b"====", b""),
        ("base32", b"MY=====", b""),
        ("base32", b"MZ======", b"f"),
        ("base32", b"========", b""),
        ("base32", b"my======", b""),
    ] {
        let result = convert(root, &["--progress=never", utility, "-d"], input);
        check(&result, 1, prefix);
        assert!(String::from_utf8_lossy(&result.stderr).contains("invalid input"));
    }
}

#[test]
fn exposes_options_and_progress_without_mixing_binary_output() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("input"), b"foobar").unwrap();
    let result = convert(
        root,
        &[
            "--progress=always",
            "base64",
            "--wrap=4",
            "--ignore-garbage",
            "input",
        ],
        b"",
    );
    check(&result, 0, b"Zm9v\nYmFy\n");
    assert!(String::from_utf8_lossy(&result.stderr).contains("bytes"));
    for utility in ["base32", "base64"] {
        let output = command(root, &[utility, "--help"]).output().unwrap();
        assert!(output.status.success());
        assert!(output
            .stdout
            .starts_with(format!("Usage: gutil {utility}").as_bytes()));
        let output = command(root, &[utility, "--version"]).output().unwrap();
        check(
            &output,
            0,
            format!("gutil {utility} {}\n", env!("CARGO_PKG_VERSION")).as_bytes(),
        );
    }
    for args in [
        &["--progress=never", "base64", "--wrap"][..],
        &["--progress=never", "base64", "--wrap", "-1"],
        &["--progress=never", "base64", "input", "input"],
        &["--progress=never", "base64", "missing"],
        &["--progress=never", "base64", "-z"],
        &["--progress=never", "base64", "--bad-option"],
    ] {
        check(&convert(root, args, b""), 1, b"");
    }
}
