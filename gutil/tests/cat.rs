#![allow(clippy::unwrap_used)]

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command.arg("--no-config").args(args).current_dir(directory);
    command
}

fn run(directory: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut process = command(directory, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    process.stdin.take().unwrap().write_all(input).unwrap();
    process.wait_with_output().unwrap()
}

#[test]
fn copies_files_and_standard_input_byte_for_byte() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("left"), b"a\x00\xff").unwrap();
    fs::write(directory.path().join("--right"), b"c\r\n").unwrap();

    let output = run(
        directory.path(),
        &["cat", "--", "left", "-", "--right", "-"],
        b"b",
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a\x00\xffbc\r\n");
    assert_eq!(run(directory.path(), &["cat"], b"stdin").stdout, b"stdin");
}

#[test]
fn retains_formatting_state_across_operands_and_reads() {
    let directory = tempfile::tempdir().unwrap();
    let mut first = vec![b'x'; 64 * 1024 - 1];
    first.push(b'\r');
    fs::write(directory.path().join("first"), &first).unwrap();
    fs::write(directory.path().join("second"), b"\n\n\n\tz\x80\xff").unwrap();

    let numbered = run(directory.path(), &["cat", "-nbsE", "first", "second"], b"");
    assert!(numbered.status.success());
    let mut expected = b"     1\t".to_vec();
    expected.extend_from_slice(&first[..first.len() - 1]);
    expected.extend_from_slice(b"^M$\n$\n     2\t\tz\x80\xff");
    assert_eq!(numbered.stdout, expected);

    let visible = run(
        directory.path(),
        &["cat", "--threads=2", "-A"],
        b"\x00\t\r\n\x80\xff",
    );
    assert!(visible.status.success());
    assert_eq!(visible.stdout, b"^@^I^M$\nM-^@M-^?");
}

#[test]
fn reports_input_failures_and_continues() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("file"), b"remaining").unwrap();
    let output = run(directory.path(), &["cat", "missing", ".", "file"], b"");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"remaining");
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing"));

    let invalid = run(directory.path(), &["cat", "--unknown"], b"");
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(invalid.stdout, b"");
}

#[test]
fn rejects_the_same_regular_file_as_input_and_output() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    fs::write(&path, b"original").unwrap();
    let output = File::options().write(true).open(&path).unwrap();
    let result = command(directory.path(), &["cat", "file"])
        .stdout(output)
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("input file is output file"));
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[test]
fn exposes_command_help_and_version() {
    let directory = tempfile::tempdir().unwrap();
    let help = run(directory.path(), &["cat", "--help"], b"");
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: gutil cat "));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--show-nonprinting"));

    let version = run(directory.path(), &["cat", "--version"], b"");
    assert!(version.status.success());
    assert_eq!(
        version.stdout,
        format!("gutil cat {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
}

#[test]
fn handles_combined_and_long_formatting_options() {
    let directory = tempfile::tempdir().unwrap();
    let input = b"\t\x00\r\n\n\nend\r";
    let output = run(
        directory.path(),
        &[
            "cat",
            "--show-nonprinting",
            "--show-tabs",
            "--show-ends",
            "--number",
            "--squeeze-blank",
            "-uj2",
        ],
        input,
    );
    assert!(output.status.success());
    assert_eq!(output.stdout, b"     1\t^I^@^M$\n     2\t$\n     3\tend^M");
    assert_eq!(
        run(directory.path(), &["cat", "-e"], b"\x00\n").stdout,
        b"^@$\n"
    );
    assert_eq!(
        run(directory.path(), &["cat", "-t"], b"\t\x00\n").stdout,
        b"^I^@\n"
    );
    assert_eq!(
        run(directory.path(), &["cat", "-E"], b"end\r").stdout,
        b"end\r"
    );
    assert_eq!(
        run(directory.path(), &["cat", "-z"], b"").status.code(),
        Some(1)
    );
}

#[test]
fn reports_a_closed_output_pipe() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("large"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let mut process = command(directory.path(), &["cat", "large"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(process.stdout.take());
    let output = process.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.starts_with(b"gutil cat:"));
}
