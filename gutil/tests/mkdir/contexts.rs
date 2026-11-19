use super::*;

#[test]
fn context_requests_never_silently_claim_active_label_assignment() {
    for context in ["-Z", "--context", "--context=example"] {
        let fixture = tempfile::tempdir().unwrap();
        let output = invocation(fixture.path())
            .args([context, "created"])
            .output()
            .unwrap();
        if output.status.success() {
            assert!(fixture.path().join("created").is_dir());
            if context.contains('=') {
                assert!(String::from_utf8_lossy(&output.stderr).contains("ignoring context"));
            }
        } else {
            assert_eq!(output.status.code(), Some(1));
            assert!(!fixture.path().join("created").exists());
            assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
        }
    }
}
