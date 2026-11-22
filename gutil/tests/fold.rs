#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const GUTIL: &str = env!("CARGO_BIN_EXE_gutil");

fn command(path: &Path, locale: &str) -> Command {
    let mut command = Command::new(GUTIL);
    command
        .current_dir(path)
        .env("LC_ALL", locale)
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--progress=never", "--color=never", "fold"]);
    command
}

fn run(command: &mut Command, flags: &[&str], data: &[u8]) -> Output {
    let mut child = command
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            if let Err(error) = input.write_all(data) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    })
}

fn expect(output: Output, status: i32, bytes: &[u8]) {
    let Output {
        status: actual,
        stdout,
        stderr,
    } = output;
    assert_eq!(actual.code(), Some(status), "{stderr:?}");
    assert_eq!(stdout, bytes);
}

#[test]
fn selects_counting_modes_and_preserves_cursor_controls() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let data = b"a\tb\x08c\rd\0e\x0bf\r\nz";
    for (flags, expected) in [
        (
            &["--width", "3"][..],
            b"a\n\t\nb\x08c\rd\0e\x0b\nf\r\nz".as_slice(),
        ),
        (&["-b3"], b"a\tb\n\x08c\r\nd\0e\n\x0bf\r\nz"),
        (&["--characters", "-w3"], b"a\n\t\nb\x08c\rd\0e\n\x0bf\r\nz"),
    ] {
        expect(run(&mut command(path, "C"), flags, data), 0, expected);
    }
    let text = "é界🧪a\n".as_bytes();
    for (flags, expected) in [
        (&["-w2"][..], "é\n界\n🧪\na\n"),
        (&["-w2", "-c"], "é界\n🧪a\n"),
        (&["-w1", "--bytes"], "é\n界\n🧪\na\n"),
        (&["-b", "-c", "-w2"], "é界\n🧪a\n"),
    ] {
        expect(
            run(&mut command(path, "C.UTF-8"), flags, text),
            0,
            expected.as_bytes(),
        );
    }
}

#[test]
fn prefers_horizontal_blanks_and_preserves_line_endings() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    expect(
        run(&mut command(path, "C"), &["-s3"], b"aaa bb cc\n\nword"),
        0,
        b"aaa\n \nbb \ncc\n\nwor\nd",
    );
    expect(
        run(
            &mut command(path, "C.UTF-8"),
            &["-bs5"],
            "a\u{2003}bcd".as_bytes(),
        ),
        0,
        "a\u{2003}\nbcd".as_bytes(),
    );
    expect(
        run(&mut command(path, "C"), &["-bs3"], b"a\rbcd"),
        0,
        b"a\rb\ncd",
    );
}

#[test]
fn keeps_utf8_scalars_intact_across_reads_and_copies_invalid_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let mut data = vec![b'a'; 65_537];
    data.extend_from_slice("界é".as_bytes());
    data.extend_from_slice(b"\xff\xe2\x82");
    fs::write(path.join("input"), &data).unwrap();
    expect(
        run(&mut command(path, "C.UTF-8"), &["-w65537", "input"], b""),
        0,
        [&data[..65_537], b"\n", &data[65_537..]]
            .concat()
            .as_slice(),
    );
    let mut prefix = vec![0; 140_000];
    prefix.extend_from_slice(b"a\n");
    expect(run(&mut command(path, "C"), &["-s1"], &prefix), 0, &prefix);
}

#[test]
fn resets_at_operands_and_continues_after_open_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("left"), b"a").unwrap();
    fs::write(path.join("-right"), b"bcd").unwrap();
    let result = run(
        &mut command(path, "C"),
        &[
            "--threads=2",
            "-2",
            "left",
            "missing",
            "-",
            "-",
            "--",
            "-right",
        ],
        b"xy",
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
    expect(result, 1, b"axybc\nd");
}

#[test]
fn validates_options_and_keeps_progress_on_stderr() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for flags in [
        &["-w"][..],
        &["-w0"],
        &["--width=5k"],
        &["--bytes=yes"],
        &["-z"],
    ] {
        expect(run(&mut command(path, "C"), flags, b"input"), 1, b"");
    }
    assert!(run(&mut command(path, "C"), &["--help"], b"")
        .stdout
        .starts_with(b"Usage: gutil fold"));
    assert!(run(&mut command(path, "C"), &["--vers"], b"")
        .stdout
        .starts_with(b"gutil fold "));
    let mut progress = Command::new(GUTIL);
    progress
        .current_dir(path)
        .args(["--no-config", "--progress=always", "--color=never", "fold"]);
    let result = run(&mut progress, &["-sj2", "-w2"], b"abcd");
    assert!(String::from_utf8_lossy(&result.stderr).contains("bytes"));
    expect(result, 0, b"ab\ncd");
}

#[test]
fn matches_gnu_for_mixed_and_binary_input_when_configured() {
    let Some(base) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let reference = Path::new(&base).join("fold");
    for locale in ["C", "C.UTF-8"] {
        for flags in [
            &["-w3"][..],
            &["-bw3"],
            &["-cw3"],
            &["-sw4"],
            &["-bsw7"],
            &["-csw2"],
        ] {
            let data = b"hi there\r\t\x08\n\0\xff \xe7\x95\x8c\xe2\x80\x83z";
            let mut gnu = Command::new(&reference);
            gnu.env("LC_ALL", locale).env_remove("POSIXLY_CORRECT");
            let desired = run(&mut gnu, flags, data);
            let actual = run(&mut command(directory.path(), locale), flags, data);
            assert_eq!(
                (actual.status.code(), actual.stdout),
                (desired.status.code(), desired.stdout),
                "{locale} {flags:?}"
            );
        }
    }
}
