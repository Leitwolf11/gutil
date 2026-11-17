use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use super::command;

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
