#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

const GUTIL: &str = env!("CARGO_BIN_EXE_gutil");

fn command(path: &Path, locale: &str) -> Command {
    tab_command(path, locale, "expand")
}

fn tab_command(path: &Path, locale: &str, name: &str) -> Command {
    let mut command = Command::new(GUTIL);
    command
        .current_dir(path)
        .env("LC_ALL", locale)
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--progress=never", "--color=never", name]);
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

#[test]
fn unexpand_applies_the_selected_conversion_policy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let unexpand = || tab_command(path, "C", "unexpand");
    let data = b"        abc        z\n   \txyz\n      ";
    for flags in [&[][..], &["--first-only", "-a", "-t8"], &["-8"]] {
        check(
            run(&mut unexpand(), flags, data),
            0,
            b"\tabc        z\n\txyz\n      ",
        );
    }
    let data = b"abc x\nabc  x\nabc \tx\n  x  \ty\n";
    for flags in [&["-t4"][..], &["-4a"], &["--all", "-4"]] {
        check(
            run(&mut unexpand(), flags, data),
            0,
            b"abc x\nabc\t x\nabc\t\tx\n  x\t\ty\n",
        );
    }
    check(run(&mut unexpand(), &["-2", "-3"], &[b' '; 23]), 0, b"\t");
    check(run(&mut unexpand(), &["-1,4"], b"    abc"), 0, b"\t\tabc");
    check(
        run(&mut unexpand(), &["-aj2", "-t4"], b"a\0\x08    x\n"),
        0,
        b"a\0\x08\tx\n",
    );
    assert!(run(&mut unexpand(), &["--help"], b"")
        .stdout
        .starts_with(b"Usage: gutil unexpand "));
    assert!(run(&mut unexpand(), &["--version"], b"")
        .stdout
        .starts_with(b"gutil unexpand "));
    check(run(&mut unexpand(), &["--initial"], b""), 1, b"");
}

#[test]
fn unexpand_keeps_valid_unicode_and_pending_blanks_between_operands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let unexpand = || tab_command(path, "C.UTF-8", "unexpand");
    check(
        run(
            &mut unexpand(),
            &["-t4"],
            "abc\u{2002}  z\nabc\u{2002}z\n\u{3000}\u{3000}é\n".as_bytes(),
        ),
        0,
        "abc\t  z\nabc\u{2002}z\n\té\n".as_bytes(),
    );
    check(
        run(&mut unexpand(), &["-t4"], b"\xff  \tx\n"),
        0,
        b"\xff\tx\n",
    );
    fs::write(path.join("first"), b"abc ").unwrap();
    fs::write(path.join("second"), b" x\n").unwrap();
    let result = run(&mut unexpand(), &["-t4", "first", "missing", "second"], b"");
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
    check(result, 1, b"abc\t x\n");

    let mut data = vec![b'a'; 65_538];
    data.extend_from_slice("界    x\n".as_bytes());
    fs::write(path.join("boundary"), &data).unwrap();
    data.truncate(65_538 + "界".len());
    data.extend_from_slice(b"\tx\n");
    check(run(&mut unexpand(), &["-t8", "boundary"], b""), 0, &data);
}

#[test]
fn unexpand_matches_gnu_on_ascii_and_utf8_tab_stops_when_configured() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let cases = [
        (
            &[][..],
            b"        xyz       abc\n   \tx\n  ".as_slice(),
            b"\txyz       abc\n\tx\n  ".as_slice(),
        ),
        (
            &["-a"][..],
            b"abcdefg x\nabcdefg  x\nabcdefg \tx\n",
            b"abcdefg x\nabcdefg\t x\nabcdefg\t\tx\n",
        ),
        (
            &["-t3,5"][..],
            b"abc  X     Y\n          Z\n",
            b"abc\tX     Y\n\t\t     Z\n",
        ),
        (
            &["-t3,5,+4"][..],
            b"abc      X\n         Z\n",
            b"abc\t\tX\n\t\t\tZ\n",
        ),
        (
            &["-t3,5,/4"][..],
            b"abc      X\n         Z\n",
            b"abc\t\t X\n\t\t\t Z\n",
        ),
        (
            &["-t4"][..],
            b"a\0\x08    x\n\xff  \tx\n",
            b"a\0\x08\tx\n\xff\tx\n",
        ),
        (
            &["-t4"][..],
            "界  x\n\u{3000}\u{3000}é\n".as_bytes(),
            "界\tx\n\té\n".as_bytes(),
        ),
        (
            &["-t3,6"][..],
            "界\t   \t é\n".as_bytes(),
            "界\t\t\t é\n".as_bytes(),
        ),
    ];
    let gnu = std::env::var_os("GNU_COREUTILS_DIR").map(|root| Path::new(&root).join("unexpand"));
    for (flags, input, expected) in cases {
        if let Some(gnu) = &gnu {
            let mut reference = Command::new(gnu);
            reference
                .env("LC_ALL", "C.UTF-8")
                .env_remove("POSIXLY_CORRECT");
            check(run(&mut reference, flags, input), 0, expected);
        }
        check(
            run(&mut tab_command(path, "C.UTF-8", "unexpand"), flags, input),
            0,
            expected,
        );
    }
}

#[test]
fn unexpand_batches_long_space_runs_across_reads() {
    let directory = tempfile::tempdir().unwrap();
    let input = vec![b' '; 3 * 65_536 + 7];
    let mut expected = vec![b'\t'; 3 * 65_536 / 8];
    expected.extend_from_slice(b"       ");
    check(
        run(
            &mut tab_command(directory.path(), "C", "unexpand"),
            &[],
            &input,
        ),
        0,
        &expected,
    );
}
