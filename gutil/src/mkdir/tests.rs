#![allow(clippy::unwrap_used)]

use std::io;

use super::{context_ready, Context};
use crate::{config::Settings, presentation::Presentation};

#[test]
fn inherited_contexts_do_not_probe_and_active_requests_are_rejected() {
    let ui = Presentation::new(&Settings::default());
    let mut errors = Vec::new();
    assert!(context_ready(
        &Context::Inherited,
        || panic!("default creation must not probe labels"),
        ui,
        &mut errors
    )
    .unwrap());
    for context in [Context::Default, Context::Explicit("example".to_owned())] {
        errors.clear();
        assert!(!context_ready(&context, || Ok(true), ui, &mut errors).unwrap());
        assert!(String::from_utf8_lossy(&errors).contains("not implemented"));
    }
    errors.clear();
    let error = context_ready(
        &Context::Default,
        || Err(io::Error::other("probe failed")),
        ui,
        &mut errors,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "probe failed");
}

#[cfg(unix)]
#[test]
fn permission_adjustment_rejects_replaced_files_and_symlinks() {
    use std::os::unix::fs::symlink;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("file"), b"preserved").unwrap();
    std::fs::create_dir(root.join("target")).unwrap();
    symlink("target", root.join("alias")).unwrap();
    for name in ["file", "alias"] {
        assert!(super::platform::permissions(&root.join(name)).is_err());
    }
    assert_eq!(std::fs::read(root.join("file")).unwrap(), b"preserved");
}

#[cfg(target_os = "linux")]
#[test]
fn label_probes_distinguish_absence_presence_and_read_errors() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let selinux = root.join("enforce");
    let smack = root.join("current");
    assert!(!super::platform::active_labels_at(&selinux, &smack).unwrap());
    std::fs::write(&selinux, b"0").unwrap();
    assert!(super::platform::active_labels_at(&selinux, &smack).unwrap());
    std::fs::remove_file(&selinux).unwrap();
    std::fs::write(&smack, b"example").unwrap();
    assert!(super::platform::active_labels_at(&selinux, &smack).unwrap());
    std::fs::remove_file(&smack).unwrap();
    std::fs::create_dir(&smack).unwrap();
    assert!(super::platform::active_labels_at(&selinux, &smack).is_err());
}
