#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};

const GUTIL: &str = env!("CARGO_BIN_EXE_gutil");

fn command(locale: &str) -> Command {
    let mut command = Command::new(GUTIL);
    command
        .env("LC_ALL", locale)
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--color=never", "--progress=never", "cut"]);
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
    let mut stdin = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let writer = scope.spawn(move || {
            if let Err(error) = stdin.write_all(data) {
                assert_eq!(error.kind(), ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    })
}

fn check(output: Output, status: i32, expected: &[u8]) {
    let Output {
        status: actual,
        stdout,
        stderr,
    } = output;
    assert_eq!(actual.code(), Some(status), "{stderr:?}");
    assert_eq!(stdout, expected);
}

#[test]
fn sorts_ranges_keeps_adjacent_boundaries_and_complements_selection() {
    let data = b"abcdefg\nxyz\n\n123456";
    for (flags, expected) in [
        (
            &["-b6-,2-3,3-4", "-O|"][..],
            b"bcd|fg\nyz\n\n234|6\n".as_slice(),
        ),
        (&["-b-2,3,4-5", "-O|"], b"ab|c|de\nxy|z\n\n12|3|45\n"),
        (
            &["--bytes=2-4 6", "--complement", "-O|"],
            b"a|e|g\nx\n\n1|5\n",
        ),
        (&["-b4-", "-z"], b"defg\nxyz\n\n123456\0"),
    ] {
        check(run(&mut command("C"), flags, data), 0, expected);
    }
}

#[test]
fn handles_empty_selections_and_unterminated_blank_records() {
    check(
        run(&mut command("C"), &["-b1-", "--complement"], b"ab"),
        0,
        b"\n",
    );
    check(run(&mut command("C.UTF-8"), &["-c9"], b"ab\n"), 0, b"\n");
    check(
        run(&mut command("C.UTF-8"), &["-c1,3,8"], "🧪aé\n".as_bytes()),
        0,
        "🧪é\n".as_bytes(),
    );
    check(
        run(
            &mut command("C.UTF-8"),
            &["-f2", "--whitespace-delimited=trimmed"],
            "\u{2003} \n\u{3000}".as_bytes(),
        ),
        0,
        b"\n",
    );
}

#[test]
fn reports_a_closed_output_pipe() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("data");
    fs::write(&path, vec![b'x'; 1024 * 1024]).unwrap();
    let mut child = command("C")
        .arg("-b1-")
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let result = child.wait_with_output().unwrap();
    assert!(result.stderr.starts_with(b"gutil cut:"));
    check(result, 1, b"");
}

#[test]
fn selects_utf8_characters_and_whole_units_from_byte_suffixes() {
    let data = "aé界x\n".as_bytes();
    for (flags, expected) in [
        (&["-c2,4", "-O|"][..], "é|x\n".as_bytes()),
        (&["-b1,3-6", "-n", "-O|"], "a|é界\n".as_bytes()),
        (&["-b2", "-n"], b"\n"),
        (&["-b3", "-n"], "é\n".as_bytes()),
    ] {
        check(run(&mut command("C.UTF-8"), flags, data), 0, expected);
    }
    check(
        run(&mut command("C.UTF-8"), &["-c2-3"], b"a\xe2\x80b\xff\n"),
        0,
        b"\xe2\x80\n",
    );
    check(
        run(
            &mut command("C.UTF-8"),
            &["-b1,3,5-6", "-n", "-O|"],
            "a界é\n".as_bytes(),
        ),
        0,
        "a|é\n".as_bytes(),
    );
    check(
        run(&mut command("C"), &["-c2-3"], data),
        0,
        "é\n".as_bytes(),
    );
}

#[test]
fn handles_custom_and_blank_separators() {
    let input = b":a::d\nnone\nx:y\n";
    check(
        run(&mut command("C"), &["-f1,3-4", "-d:", "-O|"], input),
        0,
        b"||d\nnone\nx\n",
    );
    check(
        run(&mut command("C"), &["-sf2", "-d:", "--complement"], input),
        0,
        b"::d\nx\n",
    );
    let blanks = "  a\t\u{2003}b  \nword\n  \n".as_bytes();
    check(
        run(&mut command("C.UTF-8"), &["-w", "-f2"], blanks),
        0,
        b"a\nword\n\n",
    );
    check(
        run(
            &mut command("C.UTF-8"),
            &["-F1-", "--whitespace-delimited=trim", "-s"],
            blanks,
        ),
        0,
        b"a b\n",
    );
    check(
        run(
            &mut command("C.UTF-8"),
            &["-f2", "-d界"],
            "a界b界c\nword".as_bytes(),
        ),
        0,
        b"b\nword\n",
    );
    check(
        run(&mut command("C"), &["-f1,2", "-O", ""], b"a\tb\n"),
        0,
        b"a\0b\n",
    );
}

#[test]
fn handles_nul_and_matching_field_and_record_terminators() {
    check(
        run(&mut command("C"), &["-f2", "-d", ""], b"a\0b\nword\n"),
        0,
        b"b\nword\n",
    );
    check(
        run(
            &mut command("C"),
            &["-zf2,3", "-d:", "-O|"],
            b"a:b:c\0x:y\n\0",
        ),
        0,
        b"b|c\0y\n\0",
    );
    check(
        run(&mut command("C"), &["-f2,3", "-d", "\n", "-O|"], b"a\nb\nc"),
        0,
        b"b|c\n",
    );
    check(run(&mut command("C"), &["-sf2", "-d", "\n"], b"a"), 0, b"");
    check(
        run(&mut command("C"), &["-f2", "-d", "\n"], b"a\n"),
        0,
        b"\n",
    );
}

#[test]
fn processes_long_records_and_continues_after_a_missing_operand() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let mut long = vec![b'x'; 65_535];
    long.extend_from_slice("界selected\n".as_bytes());
    fs::write(path.join("long"), &long).unwrap();
    fs::write(path.join("-last"), b"a\tb").unwrap();
    let mut selected = vec![b'x'; 65_534];
    selected.extend_from_slice("界|s\n".as_bytes());
    check(
        run(
            command("C").current_dir(path),
            &["-b2-65538,65539", "-O|", "long"],
            b"",
        ),
        0,
        &selected,
    );
    let result = run(
        command("C.UTF-8").current_dir(path),
        &[
            "--threads=2",
            "-f2",
            "-d界",
            "long",
            "missing",
            "-",
            "--",
            "-last",
        ],
        "a界stdin\n".as_bytes(),
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
    check(result, 1, b"selected\nstdin\na\tb\n");
}

#[test]
fn rejects_invalid_combinations_and_keeps_progress_on_stderr() {
    for flags in [
        &[][..],
        &["-b"],
        &["-b0"],
        &["-b1x"],
        &["--complement=yes"],
        &["-b1, 2"],
        &["-b-"],
        &["-b3-2"],
        &["-f2", "-b1"],
        &["-b1", "-s"],
        &["-f1", "-w", "-d:"],
        &["-f1", "-dxy"],
        &["-f1", "--whitespace-delimited=other"],
        &["--o"],
        &["-Q"],
    ] {
        check(run(&mut command("C"), flags, b"input"), 1, b"");
    }
    assert!(run(&mut command("C"), &["--help"], b"")
        .stdout
        .starts_with(b"Usage: gutil cut "));
    assert!(run(&mut command("C"), &["--vers"], b"")
        .stdout
        .starts_with(b"gutil cut "));
    let output = run(
        Command::new(GUTIL).args(["--no-config", "--color=never", "--progress=always", "cut"]),
        &["-nzj2", "-b1"],
        b"ab\0",
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("bytes"));
    check(output, 0, b"a\0");
}

#[test]
fn matches_gnu_in_c_and_utf8_locales_when_configured() {
    let Some(paths) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let gnu = Path::new(&paths).join("cut");
    for locale in ["C", "C.UTF-8"] {
        for flags in [
            &["-b1,3-6", "-n", "-O|"][..],
            &["-c2,4-", "--complement"],
            &["-f2", "-d:"],
            &["-sf1,3", "-w"],
            &["-F1-", "--whitespace-delimited=trimmed"],
            &["-zf1,2", "-d:"],
            &["-f2,3", "-d", "\n", "-O|"],
        ] {
            let data = b"  a :\xc3\xa9\te\x00\n\xe2\x80\x83x:y\xff z\n";
            let expected = run(
                Command::new(&gnu)
                    .env("LC_ALL", locale)
                    .env_remove("POSIXLY_CORRECT"),
                flags,
                data,
            );
            let actual = run(&mut command(locale), flags, data);
            assert_eq!(
                (actual.status.code(), actual.stdout),
                (expected.status.code(), expected.stdout),
                "{locale} {flags:?}"
            );
        }
    }
}
