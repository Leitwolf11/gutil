#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn command(root: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .args(["--no-config", "--progress=never", "--color=never", "tee"])
        .args(arguments);
    command
}

fn capture(mut command: Command, input: &[u8]) -> Output {
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
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
            }
        });
        let output = child.wait_with_output().unwrap();
        writer.join().unwrap();
        output
    })
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn duplicates_binary_streams_appends_and_preserves_literal_file_names() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let bytes: Vec<_> = (0..=255).cycle().take(400_003).collect();
    let output = capture(command(root, &["first", "-", "--", "-a"]), &bytes);
    success(&output);
    assert_eq!(output.stdout, bytes);
    for name in ["first", "-", "-a"] {
        assert_eq!(fs::read(root.join(name)).unwrap(), bytes);
    }
    success(&capture(command(root, &["-aij1", "first"]), b"end"));
    assert_eq!(
        fs::read(root.join("first")).unwrap(),
        [bytes, b"end".to_vec()].concat()
    );
    let before = fs::read(root.join("first")).unwrap();
    success(&capture(
        command(
            root,
            &["--append", "--ignore-interrupts", "--threads=2", "first"],
        ),
        b"long options",
    ));
    assert_eq!(
        fs::read(root.join("first")).unwrap(),
        [before, b"long options".to_vec()].concat()
    );
    success(&capture(command(root, &["first"]), b"replacement"));
    assert_eq!(fs::read(root.join("first")).unwrap(), b"replacement");
}

#[test]
fn opening_errors_follow_the_selected_policy() {
    for (mode, stops) in [
        ("warn", false),
        ("warn-nopipe", false),
        ("exit", true),
        ("exit-nopipe", true),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let output = capture(
            command(
                root,
                &[&format!("--output-error={mode}"), "missing/file", "good"],
            ),
            b"data",
        );
        assert_eq!(output.status.code(), Some(1));
        assert_ne!(output.stderr, [] as [u8; 0]);
        assert_eq!(output.stdout, if stops { b"".as_slice() } else { b"data" });
        assert_eq!(root.join("good").exists(), !stops);
        if !stops {
            assert_eq!(fs::read(root.join("good")).unwrap(), b"data");
        }
    }
}

#[test]
fn validates_options_before_creating_files_and_keeps_optional_modes_attached() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for args in [
        vec!["file", "--output-error=bad"],
        vec!["file", "-z"],
        vec!["file", "--unknown"],
        vec!["file", "-j0"],
        vec!["file", "--threads"],
    ] {
        let output = capture(command(root, &args), b"");
        assert_eq!(output.status.code(), Some(2));
        assert!(!root.join("file").exists());
    }
    success(&capture(
        command(root, &["--output-error", "exit"]),
        b"literal",
    ));
    assert_eq!(fs::read(root.join("exit")).unwrap(), b"literal");
    for args in [["--jobs=1", "--help"], ["--threads=2", "--version"]] {
        success(&capture(command(root, &args), b""));
    }
}

fn wait(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("tee did not terminate: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn closed_pipes_continue_or_exit_without_waiting_for_more_input() {
    for (mode, code, copies) in [
        ("warn", 1, true),
        ("warn-nopipe", 0, true),
        ("exit", 1, false),
        ("exit-nopipe", 0, true),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let data = vec![42; 300_001];
        fs::write(root.join("input"), &data).unwrap();
        let mut child = command(root, &[&format!("--output-error={mode}"), "good"])
            .stdin(fs::File::open(root.join("input")).unwrap())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let output = wait(child);
        assert_eq!(output.status.code(), Some(code), "{mode}: {output:?}");
        assert_eq!(
            fs::read(root.join("good")).unwrap(),
            if copies { data } else { vec![] }
        );

        let mut child = command(root, &[&format!("--output-error={mode}")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"data").unwrap();
        // Keep stdin open: with no live outputs, tee must not await EOF.
        assert_eq!(wait(child).status.code(), Some(code));
        drop(stdin);
    }
}

#[test]
fn default_broken_pipe_terminates_the_command() {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    let fixture = tempfile::tempdir().unwrap();
    let mut child = command(fixture.path(), &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"data").unwrap();
    let output = wait(child);
    #[cfg(unix)]
    assert_eq!(output.status.signal(), Some(13));
    #[cfg(windows)]
    assert_eq!(output.status.code(), Some(1));
}

#[cfg(unix)]
#[test]
fn interrupt_handling_is_installed_before_output_files_are_created() {
    use std::os::unix::process::ExitStatusExt;
    for ignore in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        let args = if ignore {
            vec!["-i", "ready"]
        } else {
            vec!["ready"]
        };
        let mut child = command(root, &args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !root.join("ready").exists() {
            if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
                let _ = child.kill();
                panic!(
                    "tee did not open its output: {:?}",
                    child.wait_with_output()
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        for _ in 0..2 {
            assert!(Command::new("kill")
                .args(["-s", "INT", &child.id().to_string()])
                .status()
                .unwrap()
                .success());
            if !ignore {
                break;
            }
        }
        if ignore {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(b"after interrupt").unwrap();
            drop(stdin);
            let output = wait(child);
            success(&output);
            assert_eq!(output.stdout, b"after interrupt");
            assert_eq!(fs::read(root.join("ready")).unwrap(), b"after interrupt");
        } else {
            assert_eq!(wait(child).status.signal(), Some(2));
        }
    }
}

#[cfg(unix)]
#[test]
fn reports_input_and_non_pipe_output_errors() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = command(root, &[])
        .stdin(fs::File::open(root).unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("standard input"));
    if Path::new("/dev/full").exists() {
        let output = capture(command(root, &["/dev/full", "good"]), b"bytes");
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"bytes");
        assert_eq!(fs::read(root.join("good")).unwrap(), b"bytes");
    }
}

#[cfg(unix)]
#[test]
fn matches_gnu_for_options_duplicate_outputs_and_failures() {
    let Some(gnu) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    for args in [
        vec![],
        vec!["file"],
        vec!["file", "file"],
        vec!["-a", "file", "file"],
        vec!["-"],
        vec!["-p", "missing/file", "good"],
        vec!["--output-error=exit", "missing/file", "good"],
        vec!["--output-error", "warn"],
        vec!["--", "-a"],
    ] {
        let actual = tempfile::tempdir().unwrap();
        let reference = tempfile::tempdir().unwrap();
        fs::write(actual.path().join("file"), b"existing").unwrap();
        fs::write(reference.path().join("file"), b"existing").unwrap();
        let data: Vec<_> = (0..=255).cycle().take(330_017).collect();
        let output = capture(command(actual.path(), &args), &data);
        let mut expected = Command::new(Path::new(&gnu).join("tee"));
        expected
            .current_dir(reference.path())
            .env("LC_ALL", "C")
            .args(&args);
        let expected = capture(expected, &data);
        assert_eq!(output.status, expected.status, "{args:?}");
        assert_eq!(output.stdout, expected.stdout, "{args:?}");
        assert_eq!(
            output.stderr.is_empty(),
            expected.stderr.is_empty(),
            "{args:?}"
        );
        for name in ["file", "good", "-", "-a", "warn"] {
            assert_eq!(
                fs::read(actual.path().join(name)).ok(),
                fs::read(reference.path().join(name)).ok(),
                "{args:?}: {name}"
            );
        }
    }
}

#[test]
fn progress_stays_on_stderr_and_help_works_with_an_invalid_config() {
    let fixture = tempfile::tempdir().unwrap();
    let mut invocation = Command::new(env!("CARGO_BIN_EXE_gutil"));
    invocation.current_dir(fixture.path()).args([
        "--no-config",
        "--progress=always",
        "--color=never",
        "tee",
        "file",
    ]);
    let output = capture(invocation, b"unmodified bytes");
    success(&output);
    assert_eq!(output.stdout, b"unmodified bytes");
    assert!(String::from_utf8_lossy(&output.stderr).contains("bytes"));
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args(["--config", "missing-config.toml", "tee", "--help"])
        .output()
        .unwrap();
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Usage: gutil tee"));
}

#[cfg(unix)]
#[test]
fn native_path_handling_respects_filesystem_encoding_rules() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let name = std::ffi::OsStr::from_bytes(b"non-utf8-\xff");
    let probe = fs::write(root.join(name), b"probe");
    let mut invocation = command(root, &[]);
    invocation.arg(name).arg("valid");
    let output = capture(invocation, b"native path");
    match probe {
        Ok(()) => {
            success(&output);
            assert_eq!(fs::read(root.join(name)).unwrap(), b"native path");
        }
        Err(error) => {
            // APFS rejects invalid UTF-8 names. Verify the OS failure is reported
            // while an independent valid destination still receives the stream.
            assert_eq!(output.status.code(), Some(1));
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(diagnostic.contains("non-utf8-"));
            assert!(diagnostic.contains(&error.to_string()));
        }
    }
    assert_eq!(output.stdout, b"native path");
    assert_eq!(fs::read(root.join("valid")).unwrap(), b"native path");
}

#[test]
fn forwards_partial_lines_before_input_eof() {
    use std::io::Read;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let mut child = command(root, &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = [0; 7];
        sender
            .send(stdout.read_exact(&mut bytes).map(|()| bytes))
            .unwrap();
    });
    stdin.write_all(b"partial").unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(10));
    drop(stdin);
    let output = wait(child);
    reader.join().unwrap();
    success(&output);
    assert_eq!(result.unwrap().unwrap(), *b"partial");
}

#[test]
fn the_last_output_error_option_controls_broken_pipe_handling() {
    for (arguments, status) in [
        (vec!["-p"], 0),
        (vec!["--output-error=exit", "-p"], 0),
        (vec!["-p", "--output-error=exit"], 1),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        let mut child = command(fixture.path(), &arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(b"pipe policy").unwrap();
        let output = wait(child);
        assert_eq!(
            output.status.code(),
            Some(status),
            "{arguments:?}: {output:?}"
        );
        assert_eq!(output.stderr.is_empty(), status == 0);
        drop(stdin);
    }
}

#[test]
fn closed_stderr_does_not_mask_open_or_parse_failures() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(root.join("input"), b"complete stream").unwrap();
    for (arguments, code) in [
        (vec!["--output-error=warn", "missing/child", "survivor"], 1),
        (vec!["--unrecognized"], 2),
    ] {
        let mut child = command(root, &arguments)
            .stdin(fs::File::open(root.join("input")).unwrap())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stderr.take());
        let output = wait(child);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{arguments:?}: {output:?}"
        );
        if code == 1 {
            assert_eq!(output.stdout, b"complete stream");
            assert_eq!(fs::read(root.join("survivor")).unwrap(), b"complete stream");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn warn_modes_forward_surviving_outputs_even_when_diagnostics_cannot_be_written() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(root.join("input"), b"complete stream").unwrap();
    for mode in ["warn", "warn-nopipe", "exit", "exit-nopipe"] {
        let mut child = command(
            root,
            &[&format!("--output-error={mode}"), "/dev/full", "survivor"],
        )
        .stdin(fs::File::open(root.join("input")).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        drop(child.stderr.take());
        let output = wait(child);
        assert_eq!(output.status.code(), Some(1), "{mode}: {output:?}");
        assert_eq!(output.stdout, b"complete stream");
        let expected = if mode.starts_with("warn") {
            b"complete stream".as_slice()
        } else {
            b""
        };
        assert_eq!(fs::read(root.join("survivor")).unwrap(), expected);
    }
}
