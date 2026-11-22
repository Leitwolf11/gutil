#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const GUTIL: &str = env!("CARGO_BIN_EXE_gutil");

fn command() -> Command {
    let mut command = Command::new(GUTIL);
    command
        .args(["--no-config", "--color=never", "--progress=never", "tsort"])
        .env_remove("POSIXLY_CORRECT");
    command
}

fn execute(command: &mut Command, input: &[u8]) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            if let Err(error) = stdin.write_all(input) {
                assert_eq!(error.kind(), ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    })
}

fn check(output: Output, status: i32, bytes: &[u8]) {
    assert_eq!(output.status.code(), Some(status), "{:?}", output.stderr);
    assert_eq!(output.stdout, bytes);
}

#[test]
fn sorts_pairs_and_preserves_tie_order_and_name_bytes() {
    let cases = [
        (
            b"b c b a b d\nx x\tb d\n".as_slice(),
            b"b\nx\nd\na\nc\n".as_slice(),
        ),
        (b"a b a c d d e e", b"a\nd\ne\nc\nb\n"),
        (b"a\0x b a\0y c \xff\r \xff\r", b"a\n\xff\r\nc\nb\n"),
        (b"   \n\t ", b""),
        (b"\0x b", b"\nb\n"),
    ];
    let gnu = std::env::var_os("GNU_COREUTILS_DIR").map(|path| Path::new(&path).join("tsort"));
    for (input, expected) in cases {
        check(execute(&mut command(), input), 0, expected);
        if let Some(gnu) = &gnu {
            check(
                execute(Command::new(gnu).env("LC_ALL", "C"), input),
                0,
                expected,
            );
        }
    }
}

#[test]
fn reports_cycles_and_emits_the_remaining_order() {
    let result = execute(&mut command(), b"a b b a c d d c e f b f b f");
    let errors = String::from_utf8_lossy(&result.stderr);
    assert_eq!(errors.matches("input contains a loop").count(), 2);
    assert!(errors.contains("gutil tsort: a\n"));
    assert!(errors.contains("gutil tsort: d\n"));
    check(result, 1, b"e\na\nc\nb\nd\nf\n");
    let result = execute(&mut command(), b"a b b a b a");
    assert_eq!(
        String::from_utf8_lossy(&result.stderr)
            .matches("input contains a loop")
            .count(),
        2
    );
    check(result, 1, b"a\nb\n");
}

#[test]
fn accepts_literal_dash_paths_and_reports_file_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("-pairs"), b"a b\n").unwrap();
    check(
        execute(
            command().current_dir(path).args(["-wj1", "--", "-pairs"]),
            b"",
        ),
        0,
        b"a\nb\n",
    );
    let result = execute(command().current_dir(path).arg("missing"), b"");
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
    check(result, 1, b"");
    #[cfg(unix)]
    check(execute(command().arg(path), b""), 1, b"");
}

#[test]
fn handles_chunk_boundaries_and_unterminated_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pairs");
    let long = vec![b'x'; 150_000];
    let data = [long.as_slice(), b" a\n", b"b b"].concat();
    fs::write(&path, &data).unwrap();
    let expected = [b"b\n", long.as_slice(), b"\na\n"].concat();
    check(execute(command().arg(path), b""), 0, &expected);
    check(
        execute(command().args(["--threads=2", "-"]), b"z y"),
        0,
        b"z\ny\n",
    );
    let result = execute(&mut command(), b"a b c");
    assert!(String::from_utf8_lossy(&result.stderr).contains("odd number of tokens"));
    check(result, 1, b"");
}

#[test]
fn validates_options_and_reports_progress_separately() {
    for flags in [
        &["--unknown"][..],
        &["-x"],
        &["a", "b"],
        &["--jobs=0"],
        &["-wj"],
    ] {
        check(execute(command().args(flags), b""), 1, b"");
    }
    assert!(execute(command().arg("--help"), b"")
        .stdout
        .starts_with(b"Usage: gutil tsort "));
    assert!(execute(command().arg("--vers"), b"")
        .stdout
        .starts_with(b"gutil tsort "));
    let result = execute(
        Command::new(GUTIL).args(["--no-config", "--color=never", "--progress=always", "tsort"]),
        b"a b\n",
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("bytes"));
    check(result, 0, b"a\nb\n");
}
