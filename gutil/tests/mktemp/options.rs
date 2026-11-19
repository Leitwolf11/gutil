use super::*;

#[cfg(windows)]
#[test]
fn temporary_roots_reject_drive_relative_rooted_and_unc_templates() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for template in [
        r"C:driveXXX",
        r"\rootXXX",
        r"C:\absoluteXXX",
        r"\\server\share\nameXXX",
    ] {
        let output = invocation(root)
            .args(["--tmpdir", template])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{template:?}: {output:?}");
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
}

#[cfg(windows)]
#[test]
fn legacy_roots_also_reject_drive_prefixed_templates() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for flags in [vec!["-t"], vec!["-t", "--tmpdir"], vec!["-t", "-p", "."]] {
        let output = invocation(root)
            .args(&flags)
            .arg(r"C:tempXXX")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{flags:?}: {output:?}");
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
}

#[cfg(windows)]
#[test]
fn alternate_stream_templates_cannot_change_an_existing_file() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(root.join("preserved"), b"preserved").unwrap();
    for args in [
        vec!["preserved:XXX"],
        vec!["nameXXX:stream"],
        vec!["--suffix=:stream", "nameXXX"],
    ] {
        let output = invocation(root).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        assert_eq!(fs::read(root.join("preserved")).unwrap(), b"preserved");
    }
}

#[test]
fn rejects_invalid_requests_before_creating_anything() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for args in [
        vec!["name.XX"],
        vec!["name"],
        vec!["XXX/leaf"],
        vec!["XXX/"],
        vec!["oldXXX.newXX"],
        vec!["XXX", "extra"],
        vec!["-p"],
        vec!["--suffix"],
        vec!["--suffix=/bad", "XXX"],
        vec!["--suffix=.txt", "XXX.old"],
        vec!["--tmpdir", "/absoluteXXX"],
        vec!["-t", "parent/nameXXX"],
        vec!["-j0", "XXX"],
        vec!["-dqz", "XXX"],
        vec!["--unknown", "XXX"],
        vec!["-q", "--suffix=/", "XXX"],
    ] {
        let output = invocation(root).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert_ne!(output.stderr.as_slice(), b"");
        assert_eq!(fs::read_dir(root).unwrap().count(), 0);
    }
}

#[test]
fn direct_help_and_version_ignore_missing_configuration() {
    for argument in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .args(["--config", "missing.toml", "mktemp", argument])
            .output()
            .unwrap();
        success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("gutil mktemp"));
    }
}
