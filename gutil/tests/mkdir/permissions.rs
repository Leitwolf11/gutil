use super::*;

#[cfg(unix)]
#[test]
fn applies_leaf_modes_and_preserves_existing_directory_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(&masked(
        root,
        "027",
        &["-p", "--mode", "000", "parent/child"],
    ));
    assert_eq!(mode(&root.join("parent")), 0o750);
    assert_eq!(mode(&root.join("parent/child")), 0);
    fs::set_permissions(root.join("parent"), fs::Permissions::from_mode(0o711)).unwrap();
    symlink("parent", root.join("alias")).unwrap();
    success(&masked(
        root,
        "0777",
        &["-p", "-m700", "parent", "alias", "new/leaf"],
    ));
    assert_eq!(mode(&root.join("parent")), 0o711);
    assert_eq!(mode(&root.join("new")), 0o300);
    assert_eq!(mode(&root.join("new/leaf")), 0o700);
}

#[cfg(unix)]
#[test]
fn trailing_dot_and_parent_components_keep_ancestor_modes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(&masked(
        root,
        "022",
        &["-p", "-m000", "a/.", "b/../c", "d//./e/"],
    ));
    for path in ["a", "b", "d"] {
        assert_eq!(mode(&root.join(path)), 0o755);
    }
    for path in ["c", "d/e"] {
        assert_eq!(mode(&root.join(path)), 0);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn symbolic_and_operator_modes_preserve_documented_creation_semantics() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (index, (specification, expected)) in [
        ("0644", 0o644),
        ("07777", 0o7777),
        ("1777", 0o1777),
        ("u=rwx,g=u,o=g,u-w,g-w,o-w", 0o555),
        ("00755", 0o755),
        ("u=rw,g=u,o=g", 0o666),
        ("a+t", 0o1755),
        ("u+s", 0o4777),
        ("g+s", 0o2777),
        ("u+s,g-s", 0o4777),
        ("a=rwx,o+t", 0o1777),
        ("u+r-w+x", 0o577),
        ("+1000", 0o1755),
        ("-2000", 0o755),
        ("=0,u+r", 0o400),
        ("g=u", 0o777),
        ("a+g", 0o777),
        ("u=o", 0o777),
        ("a=rwX", 0o777),
        ("-w", 0o577),
        ("=rw", 0o644),
        ("o=s", 0o770),
        ("u+t", 0o777),
        ("u+", 0o777),
    ]
    .into_iter()
    .enumerate()
    {
        let name = index.to_string();
        success(&masked(root, "022", &["-m", specification, &name]));
        assert_eq!(mode(&root.join(name)), expected, "{specification}");
    }
}

#[cfg(unix)]
#[test]
fn explicit_modes_work_on_created_operands_with_trailing_separators() {
    let fixture = tempfile::tempdir().unwrap();
    success(&masked(fixture.path(), "022", &["-m1777", "trailing/"]));
    assert_eq!(mode(&fixture.path().join("trailing")), 0o1777);
}

#[test]
fn trailing_dot_uses_parent_creation_semantics_before_the_first_attempt() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(
        &invocation(root)
            .args(["-p", "-m000", "a/.", "b/./"])
            .output()
            .unwrap(),
    );
    for name in ["a", "b"] {
        let path = root.join(name);
        assert!(path.is_dir());
        #[cfg(unix)]
        assert_eq!(mode(&path) & 0o300, 0o300);
    }
}
