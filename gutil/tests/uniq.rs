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
        .args(["--no-config", "--progress=never", "--color=never", "uniq"]);
    command
}

fn run(command: &mut Command, args: &[&str], input: &[u8]) -> Output {
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let thread = scope.spawn(move || {
            if let Err(error) = writer.write_all(input) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        thread.join().unwrap();
        output
    })
}

fn expect(output: &Output, status: i32, expected: &[u8]) {
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert_eq!(output.stdout, expected);
}

#[test]
fn filters_only_adjacent_groups_and_terminates_partial_records() {
    let fixture = tempfile::tempdir().unwrap();
    let input = b"a\na\nb\na\nc\nc\nd";
    for (args, expected) in [
        (&[][..], b"a\nb\na\nc\nd\n".as_slice()),
        (
            &["--count"],
            b"      2 a\n      1 b\n      1 a\n      2 c\n      1 d\n",
        ),
        (&["--repeated"], b"a\nc\n"),
        (&["--unique"], b"b\na\nd\n"),
        (&["-d", "-u"], b""),
        (&["-Du"], b"a\nc\n"),
        (&["--all-repeated=separate"], b"a\na\n\nc\nc\n"),
        (&["--all-repeated=prepend"], b"\na\na\n\nc\nc\n"),
        (&["--all-repeated=none"], b"a\na\nc\nc\n"),
    ] {
        expect(&run(&mut command(fixture.path()), args, input), 0, expected);
    }
}

#[test]
fn groups_all_records_with_each_separator_mode() {
    let fixture = tempfile::tempdir().unwrap();
    for (option, expected) in [
        ("--group", b"a\na\n\nb\n".as_slice()),
        ("--group=append", b"a\na\n\nb\n\n"),
        ("--group=prepend", b"\na\na\n\nb\n"),
        ("--group=both", b"\na\na\n\nb\n\n"),
    ] {
        expect(
            &run(&mut command(fixture.path()), &[option], b"a\na\nb"),
            0,
            expected,
        );
        expect(&run(&mut command(fixture.path()), &[option], b""), 0, b"");
    }
}

#[test]
fn selects_key_bytes_and_utf8_characters_and_preserves_records() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    expect(
        &run(
            &mut command(path),
            &["-if1", "-s1", "--check-chars=2"],
            b"1 ABx\n2 aby\n3 Cxz",
        ),
        0,
        b"1 ABx\n3 Cxz\n",
    );
    expect(
        &run(&mut command(path), &["-1"], b"1 a\n2 a\n3 b"),
        0,
        b"1 a\n3 b\n",
    );
    expect(
        &run(&mut command(path), &["-cz"], b"a\xff\0a\xff\0b"),
        0,
        b"      2 a\xff\0      1 b\0",
    );
    let input = "éA\n界a\n界b\n".as_bytes();
    let result = run(
        command(path).env("LC_ALL", "C.UTF-8"),
        &["-is1", "--wid=1"],
        input,
    );
    expect(&result, 1, b"");
    let result = run(
        command(path).env("LC_ALL", "C.UTF-8"),
        &["-is1", "-w1"],
        input,
    );
    expect(&result, 0, "éA\n界b\n".as_bytes());
}

#[test]
fn writes_regular_output_and_rejects_aliases_without_truncating() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    fs::write(path.join("input"), b"a\na\nb").unwrap();
    fs::write(path.join("out"), b"some older content").unwrap();
    expect(&run(&mut command(path), &["input", "out"], b""), 0, b"");
    assert_eq!(fs::read(path.join("out")).unwrap(), b"a\nb\n");
    fs::hard_link(path.join("input"), path.join("alias")).unwrap();
    for output in ["input", "alias"] {
        let result = run(&mut command(path), &["input", output], b"");
        expect(&result, 1, b"");
        assert!(String::from_utf8_lossy(&result.stderr).contains("same file"));
        assert_eq!(fs::read(path.join("input")).unwrap(), b"a\na\nb");
    }
    let missing = run(&mut command(path), &["missing", "out"], b"");
    expect(&missing, 1, b"");
    assert_eq!(fs::read(path.join("out")).unwrap(), b"a\nb\n");
}

#[test]
fn rejects_regular_aliases_through_standard_streams() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    let file = path.join("data");
    fs::write(&file, b"a\na\nb\n").unwrap();
    let input = fs::File::open(&file).unwrap();
    let result = command(path)
        .args(["-", "data"])
        .stdin(input)
        .output()
        .unwrap();
    expect(&result, 1, b"");
    assert!(String::from_utf8_lossy(&result.stderr).contains("same file"));

    let output = fs::OpenOptions::new().append(true).open(&file).unwrap();
    let result = command(path).arg("data").stdout(output).output().unwrap();
    expect(&result, 1, b"");
    assert!(String::from_utf8_lossy(&result.stderr).contains("same file"));
    assert_eq!(fs::read(&file).unwrap(), b"a\na\nb\n");
}

#[test]
fn rejects_incompatible_and_malformed_options() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path();
    for args in [
        &["-cD"][..],
        &["--group", "-u"],
        &["--group=unknown"],
        &["--all-repeated=both"],
        &["-f"],
        &["-f-1"],
        &["--skip-chars=foo"],
        &["-q"],
        &["--c"],
        &["a", "b", "c"],
    ] {
        let output = run(&mut command(path), args, b"");
        expect(&output, 1, b"");
        assert!(!output.stderr.is_empty(), "{output:?}");
    }
    let help = run(&mut command(path), &["--help"], b"");
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: gutil uniq "));
    let version = format!("gutil uniq {}\n", env!("CARGO_PKG_VERSION"));
    expect(
        &run(&mut command(path), &["--version"], b""),
        0,
        version.as_bytes(),
    );
}

#[test]
fn matches_gnu_modes_on_binary_and_utf8_inputs() {
    let Ok(directory) = std::env::var("GNU_COREUTILS_DIR") else {
        return;
    };
    let fixture = tempfile::tempdir().unwrap();
    let reference = Path::new(&directory).join("uniq");
    for (locale, input) in [
        (
            "C",
            b"1 Ax\n2 ax\n3 bx\n\xff\n\xff\nhello\0world".as_slice(),
        ),
        ("C.UTF-8", "1 éa\n2 界a\n3 界a\n3 界b\né\né\n".as_bytes()),
    ] {
        for args in [
            &[][..],
            &["-c"],
            &["-d"],
            &["-D"],
            &["-u"],
            &["-Di"],
            &["-Du"],
            &["-f1", "-s1", "-w1"],
            &["-i", "-w0"],
            &["-z"],
            &["--group=both"],
            &["--all-repeated=prepend"],
            &["--all-repeated=separate"],
        ] {
            let mut gnu = Command::new(&reference);
            gnu.env("LC_ALL", locale).env_remove("POSIXLY_CORRECT");
            let desired = run(&mut gnu, args, input);
            let actual = run(command(fixture.path()).env("LC_ALL", locale), args, input);
            assert_eq!(
                (actual.status.code(), actual.stdout),
                (desired.status.code(), desired.stdout),
                "{locale} {args:?}"
            );
        }
    }
}
