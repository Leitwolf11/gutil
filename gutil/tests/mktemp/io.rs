use super::*;

#[test]
fn quiet_suppresses_creation_errors_but_keeps_failure_status() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let noisy = invocation(root).arg("missing/nameXXX").output().unwrap();
    assert_eq!(noisy.status.code(), Some(1));
    assert_ne!(noisy.stderr.as_slice(), b"");
    let quiet = invocation(root)
        .args(["--quiet", "missing/nameXXX"])
        .output()
        .unwrap();
    assert_eq!(quiet.status.code(), Some(1));
    assert_eq!(quiet.stderr, b"");
    assert_eq!(quiet.stdout, b"");
}

#[test]
fn progress_and_output_failures_do_not_delete_created_files() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(root)
        .args([
            "--no-config",
            "--progress=always",
            "--color=never",
            "mktemp",
            "progress.XXX",
        ])
        .output()
        .unwrap();
    assert!(root.join(path(&output)).is_file());
    assert!(String::from_utf8_lossy(&output.stderr).contains("names"));
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = invocation(root)
        .arg("closed.XXX")
        .stdout(writer)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read_dir(root).unwrap().count(), 2);
}
