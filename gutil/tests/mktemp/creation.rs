use super::*;

#[test]
fn unicode_templates_suffixes_and_attached_roots_keep_native_characters() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("箱")).unwrap();
    let output = invocation(root)
        .args(["-dqp箱", "--suffix=.例", "λ📁-XXXX"])
        .output()
        .unwrap();
    let created = path(&output);
    assert_eq!(created.parent(), Some(Path::new("箱")));
    assert_name(&created, "λ📁-", ".例", 4);
    assert!(root.join(created).is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn native_non_utf8_templates_and_suffixes_round_trip_without_loss() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = invocation(root)
        .arg(OsStr::from_bytes(b"native-\xff-XXXX"))
        .arg(OsStr::from_bytes(b"--suffix=.\xfe"))
        .output()
        .unwrap();
    success(&output);
    let name = output.stdout.strip_suffix(b"\n").unwrap();
    assert!(name.starts_with(b"native-\xff-"));
    assert!(name.ends_with(b".\xfe"));
    assert_eq!(name.len(), b"native-\xff-XXXX.\xfe".len());
    assert!(root.join(OsStr::from_bytes(name)).is_file());
}

#[test]
fn defaults_use_tmpdir_and_keep_the_created_file_after_exit() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = invocation(root).env("TMPDIR", root).output().unwrap();
    let created = path(&output);
    assert_eq!(created.parent(), Some(root));
    assert_name(&created, "tmp.", "", 10);
    assert!(created.is_file());
    assert_eq!(fs::metadata(created).unwrap().len(), 0);
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn default_directory_fallback_works_without_tmpdir() {
    let fixture = tempfile::tempdir().unwrap();
    let output = invocation(fixture.path()).output().unwrap();
    let created = path(&output);
    assert!(created.is_absolute());
    assert_name(&created, "tmp.", "", 10);
    #[cfg(unix)]
    assert_eq!(created.parent(), Some(Path::new("/tmp")));
    assert!(created.is_file());
    assert_eq!(fs::metadata(&created).unwrap().len(), 0);
    fs::remove_file(created).unwrap();
}

#[test]
fn preserves_template_structure_inferred_and_explicit_suffixes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (args, prefix, suffix, count) in [
        (vec!["local.XXXX"], "local.", "", 4),
        (vec!["XXX.old.XXXXXX.txt"], "XXX.old.", ".txt", 6),
        (vec!["--suffix=X.txt", "name.XXX"], "name.", "X.txt", 3),
        (vec!["--suffix", ".json", "name.XXXXX"], "name.", ".json", 5),
        (vec!["--suffix=", "empty.XXX"], "empty.", "", 3),
        (vec!["-j2", "--threads", "1", "--", "-XXX"], "-", "", 3),
    ] {
        let output = invocation(root).args(&args).output().unwrap();
        let created = path(&output);
        assert!(!created.is_absolute(), "{args:?}");
        assert_name(&created, prefix, suffix, count);
        assert!(root.join(&created).is_file());
    }
}

#[test]
fn creates_only_the_final_component_and_accepts_absolute_templates() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("parent")).unwrap();
    let output = invocation(root)
        .args(["--directory", "parent/child.XXXXX"])
        .output()
        .unwrap();
    let created = path(&output);
    assert!(root.join(created).is_dir());
    let created = path(
        &invocation(root)
            .arg(root.join("absolute.XXXXX"))
            .output()
            .unwrap(),
    );
    assert!(created.is_absolute());
    assert!(created.is_file());
    let failed = invocation(root)
        .args(["-d", "missing/child.XXX"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(!root.join("missing").exists());
}

#[cfg(unix)]
#[test]
fn unix_permissions_respect_owner_restrictions_in_the_umask() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for mask in ["000", "027", "200", "100"] {
        for directory in [false, true] {
            let mut command = Command::new("sh");
            command.current_dir(root).args([
                "-c",
                "umask \"$1\"; shift; exec \"$@\"",
                "mktemp-fixture",
                mask,
                env!("CARGO_BIN_EXE_gutil"),
                "--no-config",
                "--progress=never",
                "mktemp",
            ]);
            if directory {
                command.arg("-d");
            }
            let created = path(&command.arg("masked.XXXXXX").output().unwrap());
            let actual = fs::metadata(root.join(created))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let allowed = if directory { 0o700 } else { 0o600 };
            assert_eq!(actual, allowed & !u32::from_str_radix(mask, 8).unwrap());
        }
    }
}
