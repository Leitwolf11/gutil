use super::*;

#[test]
fn preserves_operand_order_and_continues_after_failure() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = invocation(root)
        .args(["-v", "first/child", "first", "after"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("first").is_dir());
    assert!(!root.join("first/child").exists());
    assert!(root.join("after").is_dir());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 2);
    let output = invocation(root).args(["first", "last"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("last").is_dir());
}

#[test]
fn rich_progress_is_separate_from_verbose_creation_messages() {
    let fixture = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(fixture.path())
        .args([
            "--no-config",
            "--color=never",
            "--progress=always",
            "mkdir",
            "-pv",
            "parent/leaf",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("directories"));
}

#[test]
fn closed_stderr_preserves_failures_and_does_not_prevent_warning_only_operations() {
    use std::process::Stdio;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (arguments, code) in [
        (vec!["missing/child", "survivor"], 1),
        (vec!["--unrecognized"], 2),
        (vec!["-m755", "with-mode"], 0),
    ] {
        let mut child = invocation(root)
            .args(&arguments)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stderr.take());
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(code), "{arguments:?}");
    }
    assert!(root.join("survivor").is_dir());
    assert!(root.join("with-mode").is_dir());
    let mut child = invocation(root)
        .args(["--context=example", "context"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stderr.take());
    let status = child.wait().unwrap();
    assert_eq!(root.join("context").is_dir(), status.success());
    assert!(matches!(status.code(), Some(0 | 1)));
}

#[test]
fn verbose_output_failures_remain_nonzero_after_directory_creation() {
    use std::process::Stdio;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for count in [1, 128] {
        let names: Vec<_> = (0..count)
            .map(|index| format!("{count}-{index}-{}", "x".repeat(120)))
            .collect();
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let mut command = invocation(root);
        command.arg("-v");
        #[cfg(unix)]
        command.arg("-m1777");
        let output = command
            .args(&names)
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        for name in names {
            let path = root.join(name);
            assert!(path.is_dir());
            #[cfg(unix)]
            assert_eq!(mode(&path), 0o1777);
        }
    }
}
