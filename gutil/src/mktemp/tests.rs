#![allow(clippy::unwrap_used)]

use std::fs;
use std::io;

use super::{close, options::Action, reserve};

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn template_generation_preserves_literal_prefixes_and_suffixes(
        prefix in "[a-z]{1,12}", suffix in "\\.[a-z]{0,8}", length in 3usize..20
    ) {
        let value = format!("{prefix}{}{suffix}", "X".repeat(length));
        let template = super::template::Template::parse(std::ffi::OsStr::new(&value), None).unwrap();
        let generated = template.name(None).unwrap();
        let name = generated.to_str().unwrap();
        let random = name.strip_prefix(&prefix).unwrap().strip_suffix(&suffix).unwrap();
        prop_assert_eq!(random.len(), length);
        prop_assert!(random.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    }
}

#[test]
fn templates_reject_nuls_before_any_filesystem_operation() {
    use std::ffi::{OsStr, OsString};
    for (template, suffix) in [("name\0XXX", None), ("nameXXX", Some("bad\0suffix"))] {
        assert!(
            super::template::Template::parse(OsStr::new(template), suffix.map(OsString::from))
                .is_err()
        );
    }
}

#[cfg(windows)]
#[test]
fn a_windows_device_handle_is_not_a_reserved_disk_file() {
    let device = fs::File::open("NUL").unwrap();
    let error = super::native::validate_disk_file(&device).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    let fixture = tempfile::tempfile().unwrap();
    super::native::validate_disk_file(&fixture).unwrap();
}

#[test]
fn collisions_preserve_existing_objects_and_retry_another_name() {
    let fixture = tempfile::tempdir().unwrap();
    let existing = fixture.path().join("existing");
    let fresh = fixture.path().join("fresh");
    fs::write(&existing, b"preserved").unwrap();
    for action in [Action::File, Action::Directory, Action::NameOnly] {
        let mut attempts = 0;
        let result = reserve(
            action,
            3,
            || {
                attempts += 1;
                Ok(if attempts == 1 {
                    existing.clone()
                } else {
                    fresh.clone()
                })
            },
            close,
        )
        .unwrap();
        assert_eq!(attempts, 2);
        assert_eq!(result, fresh);
        assert_eq!(fs::read(&existing).unwrap(), b"preserved");
        match action {
            Action::File => {
                assert!(fresh.is_file());
                fs::remove_file(&fresh).unwrap();
            }
            Action::Directory => {
                assert!(fresh.is_dir());
                fs::remove_dir(&fresh).unwrap();
            }
            Action::NameOnly => assert!(!fresh.exists()),
        }
    }
}

#[test]
fn reservation_exhaustion_and_randomness_failures_are_reported() {
    let fixture = tempfile::tempdir().unwrap();
    let existing = fixture.path().join("existing");
    fs::write(&existing, b"preserved").unwrap();
    let mut attempts = 0;
    let error = reserve(
        Action::File,
        3,
        || {
            attempts += 1;
            Ok(existing.clone())
        },
        close,
    )
    .unwrap_err();
    assert_eq!(attempts, 3);
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    let error = reserve(
        Action::File,
        3,
        || Err(io::Error::other("random source failed")),
        close,
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "random source failed");
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

#[test]
fn close_failures_do_not_retry_or_delete_the_reserved_file() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().join("reserved");
    let mut attempts = 0;
    let error = reserve(
        Action::File,
        3,
        || {
            attempts += 1;
            Ok(path.clone())
        },
        |file| {
            close(file)?;
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(attempts, 1);
    assert!(path.is_file());
}

#[test]
fn invalid_parent_errors_stop_without_retrying_or_changing_the_parent() {
    let fixture = tempfile::tempdir().unwrap();
    let parent = fixture.path().join("blocked");
    fs::write(&parent, b"preserved").unwrap();
    for child in ["child", "missing/child"] {
        for action in [Action::File, Action::Directory, Action::NameOnly] {
            let mut attempts = 0;
            let result = reserve(
                action,
                3,
                || {
                    attempts += 1;
                    Ok(parent.join(child))
                },
                close,
            );
            assert!(result.is_err());
            assert_eq!(attempts, 1);
            assert_eq!(fs::read(&parent).unwrap(), b"preserved");
        }
    }
}

#[cfg(unix)]
#[test]
fn dangling_symlinks_are_collisions_even_for_unreserved_names() {
    use std::os::unix::fs::symlink;
    let fixture = tempfile::tempdir().unwrap();
    let link = fixture.path().join("dangling");
    symlink("absent-target", &link).unwrap();
    for action in [Action::File, Action::Directory, Action::NameOnly] {
        let error = reserve(action, 2, || Ok(link.clone()), close).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }
    assert_eq!(
        fs::read_link(link).unwrap(),
        std::path::Path::new("absent-target")
    );
}
