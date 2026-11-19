use super::*;

#[test]
fn directory_selection_matches_explicit_and_legacy_precedence() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let environment = root.join("environment");
    let explicit = root.join("explicit");
    fs::create_dir(&environment).unwrap();
    fs::create_dir(&explicit).unwrap();
    for (flags, expected) in [
        (vec!["-p", "explicit"], explicit.as_path()),
        (vec!["--tmpdir=explicit"], explicit.as_path()),
        (vec!["--tmpdir"], environment.as_path()),
        (vec!["-p", ""], environment.as_path()),
        (vec!["--tmpdir="], environment.as_path()),
        (vec!["-t", "-p", "explicit"], environment.as_path()),
        (vec!["-p", "explicit", "-t"], environment.as_path()),
    ] {
        let output = invocation(root)
            .env("TMPDIR", &environment)
            .args(&flags)
            .arg("selected.XXXXX")
            .output()
            .unwrap();
        let created = path(&output);
        assert!(
            root.join(&created).starts_with(expected),
            "{flags:?}: {created:?}"
        );
        assert!(root.join(created).is_file());
    }
    let created = path(
        &invocation(root)
            .args(["-t", "-p", "explicit", "legacy.XXX"])
            .output()
            .unwrap(),
    );
    assert!(root.join(created).starts_with(&explicit));
    let created = path(&invocation(root).arg("-pexplicit").output().unwrap());
    assert!(root.join(created).starts_with(&explicit));
}

#[test]
fn dry_run_never_reserves_or_removes_names() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(root.join("preserved"), b"preserved").unwrap();
    for args in [
        vec!["-u", "name.XXXXXX"],
        vec!["-duj2", "dir.XXXXXX"],
        vec!["--dry-run", "missing/name.XXX"],
    ] {
        let created = path(&invocation(root).args(&args).output().unwrap());
        assert!(!root.join(created).exists());
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        assert_eq!(fs::read(root.join("preserved")).unwrap(), b"preserved");
    }
}
