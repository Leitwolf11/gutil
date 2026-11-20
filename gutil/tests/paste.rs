#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(path: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(path)
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--progress=never", "--color=never", "paste"]);
    command
}

fn run(command: &mut Command, args: &[&str], bytes: &[u8]) -> Output {
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            if let Err(error) = stdin.write_all(bytes) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    })
}

fn verify(output: Output, expected_status: i32, bytes: &[u8]) {
    let Output {
        status,
        stdout,
        stderr,
    } = output;
    assert_eq!(status.code(), Some(expected_status), "{stderr:?}");
    assert_eq!(stdout, bytes);
}

#[test]
fn adds_empty_columns_and_cycles_delimiters_per_row() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("a"), b"a\n\nc").unwrap();
    fs::write(path.join("b"), b"").unwrap();
    fs::write(path.join("c"), b"x\ny\n").unwrap();
    verify(
        run(&mut command(path), &["--delimiters=:;", "a", "b", "c"], b""),
        0,
        b"a:;x\n:;y\nc:;\n",
    );
    verify(run(&mut command(path), &["b", "b"], b""), 0, b"");
    verify(
        run(&mut command(path), &["a", "c", "b"], b""),
        0,
        b"a\tx\t\n\ty\t\nc\t\t\n",
    );
}

#[test]
fn serial_and_parallel_dash_operands_share_one_input() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    let input = b"one\ntwo\nthree";
    verify(run(&mut command(path), &[], input), 0, b"one\ntwo\nthree\n");
    verify(
        run(&mut command(path), &["-", "-"], input),
        0,
        b"one\ttwo\nthree\t\n",
    );
    verify(
        run(&mut command(path), &["-sd|", "-", "-"], input),
        0,
        b"one|two|three\n\n",
    );
}

#[test]
fn preserves_binary_records_and_understands_delimiter_escapes() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("a"), b"x\xff\0\ny\0").unwrap();
    fs::write(path.join("b"), b"\0z").unwrap();
    verify(
        run(&mut command(path), &["-z", "-d", r"\0", "a", "b"], b""),
        0,
        b"x\xff\0\nyz\0",
    );
    verify(
        run(
            &mut command(path),
            &["--zero-terminated", "-s", "--delimiters", r"\n", "a"],
            b"",
        ),
        0,
        b"x\xff\n\ny\0",
    );
    verify(
        run(
            &mut command(path),
            &["-sd", r"\b\f\n\r\t\v\x\\", "-"],
            b"0\n1\n2\n3\n4\n5\n6\n7\n8",
        ),
        0,
        b"0\x081\x0c2\n3\r4\t5\x0b6x7\\8\n",
    );
    verify(
        run(&mut command(path), &["--delim=", "-", "-"], b"a\nb"),
        0,
        b"ab\n",
    );
}

#[test]
fn handles_unicode_delimiters_and_large_records() {
    let fixture = tempfile::tempdir().unwrap();
    let record = "x".repeat(100_000);
    let input = format!("{record}\nb\nc\nd");
    let output = run(
        command(fixture.path()).env("LC_ALL", "C.UTF-8"),
        &["--delimiters=é界", "-", "-", "-", "-"],
        input.as_bytes(),
    );
    verify(output, 0, format!("{record}éb界céd\n").as_bytes());
}

#[test]
fn continues_after_open_errors_only_in_serial_mode() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("data"), b"a\nb").unwrap();
    let parallel = run(&mut command(path), &["data", "missing", "data"], b"");
    assert!(String::from_utf8_lossy(&parallel.stderr).contains("missing"));
    verify(parallel, 1, b"");
    let serial = run(
        &mut command(path),
        &["--ser", "data", "missing", "data"],
        b"",
    );
    assert!(String::from_utf8_lossy(&serial.stderr).contains("missing"));
    verify(serial, 1, b"a\tb\na\tb\n");
}

#[test]
fn reports_errors_and_honors_option_boundaries() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    for args in [
        &["-d"][..],
        &["-x"],
        &["--unknown"],
        &["-d", "abc\\"],
        &["--serial=yes"],
    ] {
        let output = run(&mut command(path), args, b"");
        assert!(!output.stderr.is_empty(), "{output:?}");
        verify(output, 1, b"");
    }
    fs::write(path.join("-s"), b"a\nb").unwrap();
    verify(run(&mut command(path), &["--", "-s"], b""), 0, b"a\nb\n");
    verify(
        run(
            command(path).env("POSIXLY_CORRECT", "1"),
            &["-s", "--", "-s"],
            b"",
        ),
        0,
        b"a\tb\n",
    );
    let help = run(&mut command(path), &["--help"], b"");
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: gutil paste "));
    let version = format!("gutil paste {}\n", env!("CARGO_PKG_VERSION"));
    verify(
        run(&mut command(path), &["--version"], b""),
        0,
        version.as_bytes(),
    );
}

#[test]
fn matches_gnu_for_ragged_columns_and_shared_input() {
    let Ok(directory) = std::env::var("GNU_COREUTILS_DIR") else {
        return;
    };
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("a"), b"a\nb\n\nc\0").unwrap();
    fs::write(path.join("b"), b"\nx\n").unwrap();
    fs::write(path.join("empty"), b"").unwrap();
    let reference = Path::new(&directory).join("paste");
    for args in [
        &[][..],
        &["-j2", "-"],
        &["-", "b", "-"],
        &["a", "b", "empty"],
        &["-sd", r"\0\rf", "a", "b", "empty"],
        &["-z", "a", "b"],
        &["--delimiters=", "empty", "-"],
        &["-d", "é界", "a", "b", "a"],
        &["-s", "-", "-"],
        &["a", "missing"],
        &["-s", "a", "missing", "b"],
    ] {
        let mut gnu = Command::new(&reference);
        gnu.current_dir(path)
            .env("LC_ALL", "C")
            .env_remove("POSIXLY_CORRECT");
        let gnu_args = if args.first() == Some(&"-j2") {
            &args[1..]
        } else {
            args
        };
        let expected = run(&mut gnu, gnu_args, b"1\n2\n3\n");
        let actual = run(&mut command(path), args, b"1\n2\n3\n");
        assert_eq!(actual.status.code(), expected.status.code(), "{args:?}");
        assert_eq!(actual.stdout, expected.stdout, "{args:?}");
    }
}

#[test]
fn validates_only_the_effective_delimiter_list() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    verify(
        run(
            &mut command(path),
            &["-d", "x\\", "--delimiters=:", "-", "-"],
            b"a\nb",
        ),
        0,
        b"a:b\n",
    );
    let help = run(&mut command(path), &["-d", "x\\", "--help"], b"");
    assert!(help.status.success(), "{help:?}");
    assert!(help.stdout.starts_with(b"Usage: gutil paste "));
}

#[test]
fn writes_progress_to_stderr_and_reports_closed_output() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("data"), b"a\nb\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(path)
        .args([
            "--no-config",
            "--color=never",
            "--progress=always",
            "paste",
            "data",
        ])
        .output()
        .unwrap();
    assert!(!output.stderr.is_empty(), "{output:?}");
    verify(output, 0, b"a\nb\n");

    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = command(path).arg("data").stdout(writer).output().unwrap();
    assert!(!output.stderr.is_empty(), "{output:?}");
    verify(output, 1, b"");
}

#[cfg(unix)]
#[test]
fn retains_other_columns_after_a_read_error() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("data"), b"a\nb\n").unwrap();
    let output = run(&mut command(path), &["data", "."], b"");
    assert!(!output.stderr.is_empty(), "{output:?}");
    verify(output, 1, b"a\t\nb\t\n");
}
