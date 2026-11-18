use super::{run, success};

#[test]
fn removes_empty_parents_in_order_and_only_ignores_nonempty_directories() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("tree/branch/leaf")).unwrap();
    let verbose = run(root, "rmdir", &["-pvj2", "tree/branch/leaf/"]);
    assert!(String::from_utf8_lossy(&verbose.stdout).contains("tree/branch/leaf"));
    success(&verbose);
    assert!(!root.join("tree").exists());
    std::fs::create_dir_all(root.join("nonempty/child")).unwrap();
    std::fs::create_dir(root.join("empty")).unwrap();
    success(&run(
        root,
        "rmdir",
        &["--ignore-fail-on-non-empty", "nonempty", "empty"],
    ));
    assert!(root.join("nonempty/child").is_dir());
    assert!(!root.join("empty").exists());
    assert_eq!(
        run(root, "rmdir", &["--ignore-fail-on-non-empty", "missing"])
            .status
            .code(),
        Some(1)
    );
    std::fs::create_dir(root.join("later")).unwrap();
    assert_eq!(
        run(root, "rmdir", &["missing", "later"]).status.code(),
        Some(1)
    );
    assert!(!root.join("later").exists());
}

#[cfg(unix)]
#[test]
fn preserves_literal_parent_components_during_directory_removal() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    assert_eq!(run(root, "rmdir", &["-p", "a/./b"]).status.code(), Some(1));
    assert!(root.join("a").is_dir());
    std::fs::create_dir_all(root.join("space /b")).unwrap();
    success(&run(root, "rmdir", &["-p", "space /b"]));
    assert!(!root.join("space ").exists());
}
