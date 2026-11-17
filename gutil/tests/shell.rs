#![allow(clippy::unwrap_used)]

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn command(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .env_remove("OMP_NUM_THREADS")
        .env_remove("OMP_THREAD_LIMIT")
        .args(["--no-config", "--color=never", "--progress=never"])
        .args(args);
    command
}

fn run(args: &[&str]) -> Output {
    command(args).output().unwrap()
}

fn expect(output: &Output, status: i32, stdout: &[u8]) {
    assert_eq!(output.status.code(), Some(status), "{:?}", output.stderr);
    assert_eq!(output.stdout, stdout);
}

#[test]
fn uses_fixed_statuses_and_only_interprets_standalone_information_options() {
    for (utility, status) in [("true", 0), ("false", 1)] {
        for tail in [&[][..], &["one", "--help"], &["--version", "one"]] {
            expect(&run(&[&[utility], tail].concat()), status, b"");
        }
        let help = run(&[utility, "--help"]);
        assert_eq!(help.status.code(), Some(status));
        assert!(help
            .stdout
            .starts_with(format!("Usage: gutil {utility} ").as_bytes()));
        let version = format!("gutil {utility} {}\n", env!("CARGO_PKG_VERSION"));
        expect(&run(&[utility, "--version"]), status, version.as_bytes());
    }
}

#[test]
fn prints_literal_arguments_and_decodes_all_escape_classes() {
    for (arguments, expected) in [
        (&[][..], b"\n".as_slice()),
        (&["one", "", "two"], b"one  two\n"),
        (&["-ne", r"a\tb", r"\x21"], b"a\tb !"),
        (&["-e", "-E", r"a\nb", "-n"], b"a\\nb -n\n"),
        (&["--", "-n"], b"-- -n\n"),
        (&["-foo", "-n"], b"-foo -n\n"),
        (&["--help", "operand"], b"--help operand\n"),
        (
            &["-n", "-e", r"\a\b\e\f\n\r\t\v\\\"],
            b"\x07\x08\x1b\x0c\n\r\t\x0b\\\\",
        ),
        (
            &["-e", r"\0\0123\123\777\xFa\xe\x-\q"],
            b"\0SS\xff\xfa\x0e\\x-\\q\n",
        ),
        (&["-e", "one", r"stop\cignored", "never"], b"one stop"),
    ] {
        expect(&run(&[&["echo"], arguments].concat()), 0, expected);
    }
    let posix = |arguments: &[&str]| {
        command(&[&["echo"], arguments].concat())
            .env("POSIXLY_CORRECT", "1")
            .output()
            .unwrap()
    };
    expect(&posix(&["-ne", r"a\nb"]), 0, b"-ne a\nb\n");
    expect(&posix(&["-n", "-E", r"a\nb"]), 0, b"a\nb");
    expect(&posix(&["--version"]), 0, b"--version\n");
}

#[test]
fn repeats_a_complete_line_and_handles_a_closed_consumer() {
    for (arguments, line) in [
        (&[][..], b"y\n".as_slice()),
        (&["red", "", "blue"], b"red  blue\n"),
        (&["--", "--help"], b"--help\n"),
    ] {
        let mut child = command(&[&["yes"], arguments].concat())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pipe = child.stdout.take().unwrap();
        let expected = line.repeat(1111);
        let mut data = vec![0; expected.len()];
        pipe.read_exact(&mut data).unwrap();
        drop(pipe);
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(data, expected);
        assert!(String::from_utf8_lossy(&output.stderr).contains("gutil yes"));
    }
    let invalid = run(&["yes", "-n"]);
    expect(&invalid, 1, b"");
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("option"));
}

#[test]
fn prints_only_requested_environment_values_in_operand_order() {
    let general = |args: &[&str]| {
        command(&[&["printenv"], args].concat())
            .env_clear()
            .env("GUTIL_TEST_A", "first")
            .env("GUTIL_TEST_B", "")
            .output()
            .unwrap()
    };
    expect(
        &general(&["-00", "GUTIL_TEST_A", "GUTIL_TEST_B", "GUTIL_TEST_A"]),
        0,
        b"first\0\0first\0",
    );
    expect(
        &general(&["GUTIL_TEST_A", "-0", "MISSING", "GUTIL_TEST_B"]),
        1,
        b"first\n\n",
    );
    expect(&general(&["--", "-0", "GUTIL_TEST_A"]), 1, b"first\n");
    expect(&general(&["-x"]), 2, b"");
    let all = general(&["--null"]);
    assert!(all.status.success());
    assert!(all
        .stdout
        .split(|byte| *byte == 0)
        .any(|part| part == b"GUTIL_TEST_A=first"));
    assert!(all
        .stdout
        .split(|byte| *byte == 0)
        .any(|part| part == b"GUTIL_TEST_B="));
}

#[cfg(unix)]
#[test]
fn preserves_non_utf8_environment_values_and_echo_arguments() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let raw = OsStr::from_bytes(b"a\xffz");
    let output = command(&["echo", "-n"]).arg(raw).output().unwrap();
    expect(&output, 0, b"a\xffz");
    let output = command(&["printenv", "-0", "GUTIL_TEST_NATIVE"])
        .env("GUTIL_TEST_NATIVE", raw)
        .output()
        .unwrap();
    expect(&output, 0, b"a\xffz\0");
}

#[test]
fn applies_openmp_limits_and_saturating_processor_exclusions() {
    let found = run(&["nproc"]);
    assert!(found.status.success());
    let count = String::from_utf8(found.stdout).unwrap();
    assert!(count.trim().parse::<usize>().unwrap() > 0);
    let output = command(&["nproc", "--ignore", "+1"])
        .env("OMP_NUM_THREADS", " 7,2 ")
        .env("OMP_THREAD_LIMIT", "4")
        .output()
        .unwrap();
    expect(&output, 0, b"3\n");
    let output = command(&["nproc", "--ignore=18446744073709551616"])
        .output()
        .unwrap();
    expect(&output, 0, b"1\n");
    let all = run(&["nproc", "--all"]);
    assert!(all.status.success());
    let other = command(&["nproc", "--all"])
        .env("OMP_NUM_THREADS", "1")
        .env("OMP_THREAD_LIMIT", "1")
        .output()
        .unwrap();
    expect(&other, 0, &all.stdout);
    for arguments in [
        &["--ignore=-1"][..],
        &["--ignore"],
        &["--ignore", "no"],
        &["--unknown"],
        &["1"],
        &["--", "--help"],
    ] {
        expect(&run(&[&["nproc"], arguments].concat()), 1, b"");
    }
    let output = command(&["nproc", "--ignore=1"])
        .env("OMP_NUM_THREADS", "0")
        .output()
        .unwrap();
    assert!(output.status.success());
}

#[test]
fn sums_sleep_durations_and_rejects_negative_or_invalid_values() {
    let start = Instant::now();
    expect(&run(&["sleep", "0.01s", "0.01", "0x1p-8"]), 0, b"");
    assert!(start.elapsed() >= Duration::from_millis(20));
    expect(
        &run(&["sleep", "--", "0d", "0m", "0h", "+0", "0x0"]),
        0,
        b"",
    );
    for arguments in [
        &[][..],
        &["-1"],
        &["1n"],
        &["NaN"],
        &["0x"],
        &["0x.p0"],
        &["--", "--help"],
    ] {
        expect(&run(&[&["sleep"], arguments].concat()), 1, b"");
    }
}

#[test]
fn shows_command_specific_help_and_versions() {
    for utility in ["echo", "yes", "printenv", "nproc", "sleep"] {
        let help = run(&[utility, "--help"]);
        assert!(help.status.success());
        assert!(help
            .stdout
            .starts_with(format!("Usage: gutil {utility} ").as_bytes()));
        let version = format!("gutil {utility} {}\n", env!("CARGO_PKG_VERSION"));
        expect(&run(&[utility, "--version"]), 0, version.as_bytes());
    }
}
