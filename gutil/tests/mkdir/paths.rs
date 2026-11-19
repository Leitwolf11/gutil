use super::*;

#[test]
fn creates_absolute_paths_and_leading_current_directory_components() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(
        &invocation(root)
            .args(["--parents", "./leading/leaf"])
            .arg(root.join("absolute/leaf"))
            .output()
            .unwrap(),
    );
    assert!(root.join("leading/leaf").is_dir());
    assert!(root.join("absolute/leaf").is_dir());
}

#[test]
fn alternating_existing_and_new_operands_keep_filesystem_state_current() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("existing")).unwrap();
    fs::write(root.join("blocked"), b"unchanged").unwrap();
    let output = invocation(root)
        .args([
            "-pv",
            "existing",
            "new-a",
            "existing",
            "new-b",
            "blocked/child",
            "new-c",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 3);
    for name in ["existing", "new-a", "new-b", "new-c"] {
        assert!(root.join(name).is_dir());
    }
    assert_eq!(fs::read(root.join("blocked")).unwrap(), b"unchanged");
}
