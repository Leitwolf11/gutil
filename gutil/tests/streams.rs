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
        let copy = std::thread::spawn(move || {
            for line in reader.lines() {
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
        drop(child);
        copy.join().unwrap();
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
