use super::*;

#[test]
fn parses_clustered_options_worker_aliases_and_literal_names() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(
        &invocation(root)
            .args(["-pvj2", "deep/leaf", "--threads=1", "--", "-m", "-"])
            .output()
            .unwrap(),
    );
    for name in ["deep/leaf", "-m", "-"] {
        assert!(root.join(name).is_dir());
    }
    success(
        &invocation(root)
            .args(["--parents", "--verbose", "--jobs", "2", "deep/leaf"])
            .output()
            .unwrap(),
    );
    fs::write(root.join("file"), b"keep").unwrap();
    let output = invocation(root)
        .args(["-p", "file/child", "survivor"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("survivor").is_dir());
    assert_eq!(fs::read(root.join("file")).unwrap(), b"keep");
}

#[test]
fn rejects_invalid_requests_before_mutating_the_filesystem() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for args in [
        vec![],
        vec!["new", "-m"],
        vec!["new", "--mode=888"],
        vec!["new", "--mode=10000"],
        vec!["new", "--mode=+8"],
        vec!["new", "--mode=u+1"],
        vec!["new", "--mode=+1-r"],
        vec!["new", "--mode=u"],
        vec!["new", "--mode=u+gg"],
        vec!["new", "--unknown"],
        vec!["new", "-j0"],
        vec!["new", "-q"],
        vec!["new", "--mode="],
        vec!["new", "-m", "u=rw,"],
    ] {
        let output = invocation(root).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(!root.join("new").exists());
    }
    let output = invocation(root).args(["-p", ""]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn help_and_version_do_not_require_configuration() {
    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .args(["--config", "missing-config.toml", "mkdir", option])
            .output()
            .unwrap();
        success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("gutil mkdir"));
    }
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_mode_and_context_values_before_creation() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let fixture = tempfile::tempdir().unwrap();
    for option in [b"--mode=\xff".as_slice(), b"--context=\xff"] {
        let output = invocation(fixture.path())
            .arg("new")
            .arg(OsString::from_vec(option.to_vec()))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(!fixture.path().join("new").exists());
    }
}
