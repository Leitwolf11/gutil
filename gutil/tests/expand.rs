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
        .args(["--no-config", "--progress=never", "--color=never", "expand"]);
    command
}

fn run(command: &mut Command, flags: &[&str], input: &[u8]) -> Output {
    let mut child = command
        .args(flags)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    std::thread::scope(|scope| {
        let task = scope.spawn(move || {
            if let Err(error) = writer.write_all(input) {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        task.join().unwrap();
        output
    })
}

fn check(output: Output, status: i32, expected: &[u8]) {
    let Output {
        stdout,
        stderr,
        status: actual,
    } = output;
    assert_eq!(actual.code(), Some(status), "{stderr:?}");
    assert_eq!(stdout, expected);
}

#[test]
fn handles_repeating_custom_and_extended_tab_positions() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for (arguments, expected) in [
        (&[][..], b"x       x       x       x       ".as_slice()),
        (&["-4"], b"x   x   x   x   "),
        (&["--tabs=4,7"], b"x   x  x x "),
        (&["-t4", "-t", "7"], b"x   x  x x "),
        (&["--ta", "4, /3"], b"x   x x  x  "),
        (&["-t4,+3"], b"x   x  x  x  "),
        (&["-t", "+3"], b"x  x  x  x  "),
        (&["-t/3"], b"x  x  x  x  "),
    ] {
        check(
            run(&mut command(path, "C"), arguments, b"x\tx\tx\tx\t"),
            0,
            expected,
        );
    }
    check(
        run(&mut command(path, "C"), &["-t8193"], b"\t"),
        0,
        &vec![b' '; 8193],
    );
}

#[test]
fn preserves_cursor_controls_and_switches_conversion_at_newlines() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    check(
        run(&mut command(path, "C"), &[], b"a\tb\rx\tc\n\0\t\x08\t"),
        0,
        b"a       b\rx     c\n\0        \x08 ",
    );
    check(
        run(
            &mut command(path, "C"),
            &["--initial", "-t4"],
            b" \tx\t\n\t \txy\t\n\x08\t",
        ),
        0,
        b"    x\t\n        xy\t\n\x08\t",
    );
}

#[test]
fn counts_unicode_widths_and_preserves_binary_input() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    check(
        run(&mut command(path, "C.UTF-8"), &[], "界é\t🧪\t\n".as_bytes()),
        0,
        "界é     🧪      \n".as_bytes(),
    );
    check(
        run(
            &mut command(path, "C.UTF-8"),
            &["-i"],
            "\u{2003}\t\u{a0}\t\n".as_bytes(),
        ),
        0,
        "\u{2003}       \u{a0}\t\n".as_bytes(),
    );
    check(
        run(&mut command(path, "C.UTF-8"), &[], b"\xff\t\xe2\x82\t"),
        0,
        b"\xff       \xe2\x82      ",
    );
    let mut input = vec![b'a'; 65_537];
    input.extend_from_slice("界\t".as_bytes());
    let mut expected = input[..input.len() - 1].to_vec();
    expected.extend_from_slice(b"     ");
    fs::write(path.join("long"), input).unwrap();
    check(
        run(&mut command(path, "C.UTF-8"), &["long"], b""),
        0,
        &expected,
    );
}

#[test]
fn continues_columns_across_operands_and_reports_file_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("left"), b"foo").unwrap();
    fs::write(path.join("-right"), b"\tx\n").unwrap();
    let output = run(
        &mut command(path, "C"),
        &[
            "--threads=2",
            "-t8",
            "left",
            "missing",
            "-",
            "-",
            "--",
            "-right",
        ],
        b"b",
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing"));
    check(output, 1, b"foob    x\n");
}

#[test]
fn rejects_bad_stops_and_keeps_progress_out_of_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for flags in [
        &["-t"][..],
        &["-t0"],
        &["-t8,4"],
        &["-t/3,+4"],
        &["-t4,+3,7"],
        &["-t5x"],
        &["-t9223372036854775808"],
        &["--initial=yes"],
        &["--unknown"],
        &["-x"],
    ] {
        check(run(&mut command(path, "C"), flags, b"\t"), 1, b"");
    }
    assert!(run(&mut command(path, "C"), &["--help"], b"")
        .stdout
        .starts_with(b"Usage: gutil expand "));
    assert!(run(&mut command(path, "C"), &["--vers"], b"")
        .stdout
        .starts_with(b"gutil expand "));
    let mut progress = Command::new(GUTIL);
    progress.current_dir(path).args([
        "--no-config",
        "--progress=always",
        "--color=never",
        "expand",
    ]);
    let result = run(&mut progress, &["-ij2"], b"\ta\t");
    assert!(String::from_utf8_lossy(&result.stderr).contains("bytes"));
    check(result, 0, b"        a\t");
}

#[test]
fn reports_a_closed_output_pipe_during_large_tab_expansion() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("input"), b"\t").unwrap();
    let mut child = command(path, "C")
        .args(["-t1000000", "input"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.starts_with(b"gutil expand:"));
}

#[test]
fn matches_gnu_on_tabs_binary_data_and_unicode_when_configured() {
    let Some(bin) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let reference = Path::new(&bin).join("expand");
    let directory = tempfile::tempdir().unwrap();
    let data = b" \t\x08\t abc\t\r\t\n\0\xe2\x80\x83\t\xff\t\n  \t\xe7\x95\x8c\t";
    for locale in ["C", "C.UTF-8"] {
        for flags in [
            &[][..],
            &["-i"],
            &["-t2,7"],
            &["-t2,7,+4"],
            &["-t2,7,/4"],
            &["-i", "-t2,7,+4"],
        ] {
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

#[test]
fn preserves_utf8_units_split_at_any_buffer_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for (prefix_len, scalar, spaces) in [(65_538, "é", 5), (65_536, "🧪", 6)] {
        let mut data = vec![b'x'; prefix_len];
        data.extend_from_slice(scalar.as_bytes());
        let mut expected = data.clone();
        expected.extend(std::iter::repeat_n(b' ', spaces));
        data.push(b'\t');
        fs::write(path.join("chunk"), data).unwrap();
        check(
            run(&mut command(path, "C.UTF-8"), &["chunk"], b""),
            0,
            &expected,
        );
    }

    check(
        run(
            &mut command(path, "C.UTF-8"),
            &[],
            "\0é\x08\u{ad}\t".as_bytes(),
        ),
        0,
        "\0é\x08\u{ad}       ".as_bytes(),
    );
    check(
        run(&mut command(path, "C.UTF-8"), &[], b"\x80\x80\x80"),
        0,
        b"\x80\x80\x80",
    );
}

#[cfg(all(unix, not(target_os = "fuchsia")))]
#[test]
fn writes_expanded_text_to_a_terminal() {
    use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
    use std::io::Read;

    let directory = tempfile::tempdir().unwrap();
    let pty = nix::pty::openpty(None, None).unwrap();
    let mut settings = tcgetattr(&pty.slave).unwrap();
    cfmakeraw(&mut settings);
    tcsetattr(&pty.slave, SetArg::TCSANOW, &settings).unwrap();
    fs::write(directory.path().join("input"), b"\tx\n").unwrap();
    let result = command(directory.path(), "C")
        .arg("input")
        .stdout(pty.slave)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let mut terminal = fs::File::from(pty.master);
    let mut bytes = [0; 10];
    terminal.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"        x\n");
}
