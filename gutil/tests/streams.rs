#![allow(clippy::unwrap_used)]

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn command(path: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(path)
        .env("LC_ALL", "C")
        .args(["--no-config", "--progress=never"])
        .args(args);
    command
}

fn run(path: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = command(path, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    let input = input.to_vec();
    let write = std::thread::spawn(move || {
        if let Err(error) = writer.write_all(&input) {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        }
    });
    let output = child.wait_with_output().unwrap();
    write.join().unwrap();
    output
}

fn verify(
    Output {
        status,
        stdout,
        stderr,
    }: Output,
    expected: &[u8],
) {
    assert!(status.success(), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, expected);
}

#[test]
fn selects_the_same_edges_from_regular_files_and_pipes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let input = b"one\ntwo\nthree";
    fs::write(path.join("input"), input).unwrap();
    for (args, expected) in [
        (&["head", "-n2"][..], b"one\ntwo\n".as_slice()),
        (&["head", "-n", "-2"], b"one\n"),
        (&["head", "--bytes=-3"], b"one\ntwo\nth"),
        (&["head", "-c0"], b""),
        (&["head", "-c-0"], input),
        (&["tail", "--lines=2"], b"two\nthree"),
        (&["tail", "-n+2"], b"two\nthree"),
        (&["tail", "-c", "+5"], b"two\nthree"),
        (&["tail", "-c5"], b"three"),
        (&["tail", "-n0"], b""),
        (&["tail", "-c+0"], input),
        (&["tail", "-n99"], input),
    ] {
        verify(run(path, args, input), expected);
        verify(run(path, &[args, &["input"]].concat(), b""), expected);
    }
    let terminated = b"one\0two\0three\0";
    fs::write(path.join("binary"), terminated).unwrap();
    verify(run(path, &["tail", "-zn2", "binary"], b""), b"two\0three\0");
    verify(run(path, &["head", "-zn-1"], terminated), b"one\0two\0");
}

#[test]
fn handles_boundaries_that_cross_io_blocks() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let long_record = vec![b'x'; 51 * 1024];
    let mut input = b"first\n".to_vec();
    input.extend(&long_record);
    input.extend(b"\nlast\n");
    fs::write(path.join("input"), &input).unwrap();
    for (args, selected) in [
        (&["tail", "-n2"][..], &input[6..]),
        (&["head", "-n-2"][..], &input[..6]),
        (&["head", "-c-1K"][..], &input[..input.len() - 1024]),
        (&["tail", "-c1KiB"][..], &input[input.len() - 1024..]),
    ] {
        verify(run(path, args, &input), selected);
        verify(run(path, &[args, &["input"]].concat(), b""), selected);
    }
}

#[test]
fn controls_headers_and_continues_after_open_errors() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("a"), b"first\nsecond").unwrap();
    fs::write(path.join("-b"), b"third\n").unwrap();
    verify(
        run(path, &["head", "-n1", "a", "--", "-b"], b""),
        b"==> a <==\nfirst\n\n==> -b <==\nthird\n",
    );
    verify(
        run(path, &["tail", "-q", "-n1", "a", "--", "-b"], b""),
        b"secondthird\n",
    );
    verify(
        run(path, &["tail", "-v", "-c", "2"], b"abc"),
        b"==> standard input <==\nbc",
    );
    let missing = run(path, &["head", "-q", "missing", "a"], b"");
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, b"first\nsecond");
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing"));
    for args in [
        &["head", "-n"][..],
        &["tail", "-c", "1KiF"],
        &["tail", "--retry"],
        &["tail", "--follow=invalid"],
        &["tail", "-s", "-1"],
    ] {
        assert_eq!(run(path, args, b"").status.code(), Some(1));
    }
}

struct Following(Child);

impl Drop for Following {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn finish_after_stdout_closes(child: &mut Following, path: &Path) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(b"closed\n")
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "tail kept running after its output closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn follows_appended_bytes_and_a_replaced_file_by_name() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for (follow, name) in [("-f", "descriptor.log"), ("-F", "name.log")] {
        let target = path.join(name);
        fs::write(&target, b"previous\n").unwrap();
        let mut child = Following(
            command(path, &["tail", "-n1", follow, "-s0.01", name])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let reader = BufReader::new(child.0.stdout.take().unwrap());
        let (sender, receiver) = mpsc::channel();
        let lines = usize::from(follow == "-F") + 2;
        let copy = std::thread::spawn(move || {
            for line in reader.lines().take(lines) {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let next = || receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(next(), "previous");
        OpenOptions::new()
            .append(true)
            .open(&target)
            .unwrap()
            .write_all(b"appended\n")
            .unwrap();
        assert_eq!(next(), "appended");
        if follow == "-F" {
            fs::rename(&target, path.join("old.log")).unwrap();
            fs::write(&target, b"replacement\n").unwrap();
            assert_eq!(next(), "replacement");
        }
        copy.join().unwrap();
        finish_after_stdout_closes(&mut child, &target);
    }
}

#[test]
fn counts_and_formats_totals_for_files_and_stdin() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let a = b"one\ttwo\nthree";
    fs::write(path.join("a"), a).unwrap();
    fs::write(path.join("b"), b"x\n").unwrap();
    verify(run(path, &["wc"], b"a b\n"), b"      1       2       4\n");
    verify(
        run(path, &["wc", "-lwcL", "a", "b"], b""),
        b" 1  3 13 11 a\n 1  1  2  1 b\n 2  4 15 11 total\n",
    );
    verify(
        run(path, &["wc", "-l", "--total=only", "a", "b"], b""),
        b"2\n",
    );
    verify(
        run(path, &["wc", "--bytes", "--total=always", "a"], b""),
        b"13 a\n13 total\n",
    );
    fs::write(path.join("manifest"), b"a\0b\0").unwrap();
    verify(
        run(
            path,
            &["wc", "-l", "--files0-from=manifest", "--total=never"],
            b"",
        ),
        b" 1 a\n 1 b\n",
    );
    verify(
        run(
            path,
            &["wc", "-l", "--files0-from", "-", "--total=only"],
            b"b\0",
        ),
        b"1\n",
    );
    assert_eq!(
        run(path, &["wc", "--files0-from=-", "a"], b"")
            .status
            .code(),
        Some(1)
    );
    let failed = run(path, &["wc", "-l", "--files0-from=-"], b"-\0a\0");
    assert_eq!(failed.status.code(), Some(1));
    assert_eq!(failed.stdout, b" 1 a\n 1 total\n");
}

#[test]
fn decodes_utf8_across_read_blocks_and_counts_display_columns() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let mut bytes = vec![b'x'; 65539];
    bytes.extend("界\n\té\n".as_bytes());
    fs::write(path.join("input"), &bytes).unwrap();
    let output = command(path, &["wc", "-mlwLc", "--total=only", "input"])
        .env("LC_ALL", "C.UTF-8")
        .output()
        .unwrap();
    verify(
        output,
        format!("2 2 {} {} {}\n", 65544, bytes.len(), 65541).as_bytes(),
    );
}

#[test]
fn reports_follow_failure_after_a_named_file_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("first"), b"first\n").unwrap();
    fs::write(path.join("second"), b"second\n").unwrap();
    let mut child = Following(
        command(
            path,
            &[
                "tail",
                "--follow=name",
                "--sleep-interval=0.01",
                "first",
                "second",
            ],
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap(),
    );
    let output = BufReader::new(child.0.stdout.take().unwrap());
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in output.lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    for expected in ["==> first <==", "first", "", "==> second <==", "second"] {
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
            expected
        );
    }
    fs::write(path.join("first"), b"x\n").unwrap();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), "");
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        "==> first <=="
    );
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), "x");
    fs::rename(path.join("first"), path.join("old-first")).unwrap();
    OpenOptions::new()
        .append(true)
        .open(path.join("second"))
        .unwrap()
        .write_all(b"still following\n")
        .unwrap();
    assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), "");
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        "==> second <=="
    );
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        "still following"
    );
    fs::rename(path.join("second"), path.join("old-second")).unwrap();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)),
        Err(mpsc::RecvTimeoutError::Disconnected)
    );
    assert_eq!(child.0.wait().unwrap().code(), Some(1));
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.0.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(stderr.contains("first"));
    assert!(stderr.contains("second"));
    reader.join().unwrap();
}

#[test]
fn retries_a_missing_tail_operand_and_reads_truncated_files_from_the_start() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let mut child = Following(
        command(path, &["tail", "--retry", "-f", "-s", "0.01", "file"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let output = BufReader::new(child.0.stdout.take().unwrap());
    let mut diagnostics = BufReader::new(child.0.stderr.take().unwrap());
    let (sender, receiver) = mpsc::channel();
    let started = sender.clone();
    let diagnostic = std::thread::spawn(move || {
        let mut line = String::new();
        diagnostics.read_line(&mut line).unwrap();
        let _ = started.send(line);
    });
    assert!(receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .contains("file"));
    let reader = std::thread::spawn(move || {
        for line in output.lines().take(2) {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    fs::write(path.join("file"), b"contents before truncation\n").unwrap();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        "contents before truncation"
    );
    fs::write(path.join("file"), b"short\n").unwrap();
    assert_eq!(
        receiver.recv_timeout(Duration::from_secs(5)).unwrap(),
        "short"
    );
    diagnostic.join().unwrap();
    reader.join().unwrap();
    finish_after_stdout_closes(&mut child, &path.join("file"));
}

#[cfg(target_os = "linux")]
#[test]
fn counts_and_selects_bytes_from_virtual_files_with_synthetic_sizes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for virtual_file in ["/sys/devices/system/cpu/online", "/proc/self/status"] {
        let Ok(input) = fs::read(virtual_file) else {
            continue;
        };
        if virtual_file.starts_with("/proc") {
            // Process status is process-dependent. Only compare within one child.
            let output = run(path, &["wc", "-cm", virtual_file], b"");
            assert!(output.status.success());
            let rendered = String::from_utf8(output.stdout).unwrap();
            let counts: Vec<_> = rendered.split_whitespace().take(2).collect();
            assert_eq!(counts[0], counts[1]);
            assert_ne!(counts[0], "0");
            continue;
        }
        let length = input.len();
        assert!(length > 1);
        verify(
            run(path, &["wc", "-c", virtual_file], b""),
            format!("{length} {virtual_file}\n").as_bytes(),
        );
        verify(run(path, &["tail", "-n1", virtual_file], b""), &input);
        verify(
            run(path, &["tail", "-c1", virtual_file], b""),
            &input[length - 1..],
        );
        verify(
            run(path, &["head", "-c-1", virtual_file], b""),
            &input[..length - 1],
        );
        verify(run(path, &["head", "-n-1", virtual_file], b""), b"");
    }
}

#[test]
fn counts_words_in_utf8_and_defaults_to_byte_characters_without_a_locale() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(
        path.join("input"),
        "a\u{a0}b\u{2007}c\u{202f}d\u{2060}e\u{85}f",
    )
    .unwrap();
    for (posix, expected) in [(false, b"5 11\n".as_slice()), (true, b"1 11\n")] {
        let mut process = command(path, &["wc", "-wm", "--total=only", "input"]);
        process
            .env("LC_ALL", "C.UTF-8")
            .env_remove("POSIXLY_CORRECT");
        if posix {
            process.env("POSIXLY_CORRECT", "1");
        }
        verify(process.output().unwrap(), expected);
    }
    fs::write(path.join("input"), b"a \xff b \xe2\x82").unwrap();
    verify(
        command(path, &["wc", "-wm", "--total=only", "input"])
            .env("LC_ALL", "C.UTF-8")
            .output()
            .unwrap(),
        b"4 5\n",
    );
    verify(
        command(path, &["wc", "-m", "--total=only", "input"])
            .env_remove("LC_ALL")
            .env_remove("LC_CTYPE")
            .env_remove("LANG")
            .output()
            .unwrap(),
        b"8\n",
    );
}

#[test]
fn displays_each_command_help_and_version_and_checks_options() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    for utility in ["head", "tail", "wc"] {
        let cli = format!("gutil {utility}");
        let help = run(path, &[utility, "--help"], b"");
        assert!(help.status.success());
        assert!(String::from_utf8(help.stdout)
            .unwrap()
            .starts_with(&format!("Usage: {cli}")));
        verify(
            run(path, &[utility, "--version"], b""),
            format!("{cli} {}\n", env!("CARGO_PKG_VERSION")).as_bytes(),
        );
        for flag in ["--unsupported", "-j0"] {
            assert_eq!(run(path, &[utility, flag], b"").status.code(), Some(1));
        }
    }
}

#[test]
fn recognizes_long_and_legacy_edge_options_and_keeps_pipe_boundaries() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    verify(
        run(path, &["head", "--jobs=1", "-2", "--silent"], b"a\nb\nc\n"),
        b"a\nb\n",
    );
    verify(
        run(
            path,
            &["head", "--zero-terminated", "--verbose", "-n1"],
            b"a\0b",
        ),
        b"==> standard input <==\na\0",
    );
    let bytes = vec![b'x'; 1200];
    verify(run(path, &["head", "-c1b"], &bytes), &bytes[..512]);
    verify(run(path, &["tail", "-c1KB"], &bytes), &bytes[..1000]);
    verify(
        run(path, &["tail", "--follow=descriptor", "-n0"], b"a\nb"),
        b"",
    );
    for utility in ["head", "tail"] {
        assert_eq!(run(path, &[utility, "-p"], b"").status.code(), Some(1));
    }

    let mut records = b"first record\n".repeat(10000);
    records.extend(b"last record");
    let expected = records.len() - b"first record\nlast record".len();
    verify(run(path, &["head", "-n-2"], &records), &records[..expected]);
    fs::write(path.join("empty"), []).unwrap();
    verify(run(path, &["head", "-n-1", "empty"], b""), b"");
}

#[test]
fn validates_count_file_lists_and_preserves_explicit_progress() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("-input"), b"two words\n").unwrap();
    verify(
        run(
            path,
            &[
                "wc",
                "--threads=1",
                "-lj1",
                "--chars",
                "--words",
                "--max-line-length",
                "--total",
                "auto",
                "--",
                "-input",
            ],
            b"",
        ),
        b" 1  2 10  9 -input\n",
    );
    let progress = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(path)
        .env("LC_ALL", "C")
        .args([
            "--no-config",
            "--color=never",
            "--progress=always",
            "wc",
            "-c",
            "--",
            "-input",
        ])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&progress.stderr).contains("bytes"));
    verify(progress, b"10 -input\n");
    for args in [
        &["wc", "-x"][..],
        &["wc", "--total=bogus"],
        &["wc", "--files0-from=nonexistent"],
    ] {
        assert_eq!(run(path, args, b"").status.code(), Some(1));
    }
    let manifest = run(path, &["wc", "--files0-from=-", "-l"], b"\0-input\0");
    assert_eq!(manifest.status.code(), Some(1));
    assert_eq!(manifest.stdout, b" 1 -input\n 1 total\n");
    assert!(String::from_utf8_lossy(&manifest.stderr).contains("empty file name"));
    let directory_name = run(path, &["wc", "-c", "."], b"");
    assert_eq!(directory_name.status.code(), Some(1));
    assert_eq!(directory_name.stdout, b"");
}

#[cfg(unix)]
#[test]
fn retains_native_bytes_in_a_count_file_list() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    // Passing an invalidly encoded operand should fail as an option, without
    // requiring support for invalid bytes in on-disk names.
    let unknown = command(path, &["wc"])
        .arg(OsStr::from_bytes(b"--\xff"))
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(1));
    if cfg!(target_vendor = "apple") {
        return;
    }
    fs::write(path.join(OsStr::from_bytes(b"name\xff")), b"x").unwrap();
    verify(
        run(path, &["wc", "-c", "--files0-from=-"], b"name\xff\0"),
        b"1 name\xff\n",
    );
}
