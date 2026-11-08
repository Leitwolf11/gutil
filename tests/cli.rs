#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use proptest::prelude::*;
use tempfile::tempdir;

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

#[test]
fn lists_hidden_entries_only_when_requested() {
    let directory = tempdir().unwrap();
    for name in ["zeta", "alpha", ".hidden"] {
        fs::write(directory.path().join(name), []).unwrap();
    }

    let plain = run(directory.path(), &["ls"]);
    assert!(plain.status.success());
    insta::assert_snapshot!(String::from_utf8(plain.stdout).unwrap(), @"
    alpha
    zeta
    ");

    let all = run(directory.path(), &["ls", "-a", "."]);
    assert!(all.status.success());
    assert_eq!(all.stdout, b".\n..\n.hidden\nalpha\nzeta\n");

    let almost_all = run(directory.path(), &["ls", "--almost-all"]);
    assert!(almost_all.status.success());
    assert_eq!(almost_all.stdout, b".hidden\nalpha\nzeta\n");

    let long = run(directory.path(), &["ls", "-la", "."]);
    assert!(long.status.success());
    let listing = String::from_utf8(long.stdout).unwrap();
    assert!(listing.starts_with("total "));
    assert_eq!(
        listing
            .lines()
            .skip(1)
            .map(|line| line.rsplit_once(' ').unwrap().1)
            .collect::<Vec<_>>(),
        [".", "..", ".hidden", "alpha", "zeta"]
    );
}

#[test]
fn reports_missing_operands_and_lists_remaining_paths() {
    let directory = tempdir().unwrap();
    fs::write(directory.path().join("-file"), []).unwrap();

    let output = run(directory.path(), &["ls", "--", "-file", "missing"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"-file\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing"));

    let invalid = run(directory.path(), &["ls", "-z"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert_eq!(invalid.stdout, b"");
}

#[test]
fn lists_files_before_directory_sections() {
    let directory = tempdir().unwrap();
    fs::create_dir(directory.path().join("dir")).unwrap();
    fs::write(directory.path().join("dir/entry"), []).unwrap();
    fs::write(directory.path().join("file"), []).unwrap();

    let output = run(directory.path(), &["ls", "dir", "file"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"file\n\ndir:\nentry\n");
    assert_eq!(run(directory.path(), &["ls", "-d", "dir"]).stdout, b"dir\n");
}

#[test]
fn job_options_preserve_listing_order_and_output() {
    let directory = tempdir().unwrap();
    let listing = directory.path().join("listing");
    fs::create_dir(&listing).unwrap();
    for index in (0..64).rev() {
        fs::write(listing.join(format!("file-{index:02}")), []).unwrap();
    }

    let serial = run(&listing, &["-j", "1", "ls", "-la"]);
    assert!(serial.status.success());
    let serial = String::from_utf8(serial.stdout).unwrap();
    for args in [
        &["--threads", "3", "ls", "-la"][..],
        &["--jobs=4", "ls", "-la"],
        &["ls", "-la", "--jobs", "2"],
        &["ls", "-la", "--threads=2"],
        &["ls", "-la", "-j4"],
        &["ls", "-laj4"],
    ] {
        let concurrent = run(&listing, args);
        assert!(
            concurrent.status.success(),
            "{args:?}: {:?}",
            concurrent.stderr
        );
        assert_eq!(
            String::from_utf8(concurrent.stdout).unwrap(),
            serial,
            "{args:?}"
        );
    }
}

#[test]
fn rejects_missing_or_invalid_job_counts() {
    let directory = tempdir().unwrap();
    for args in [
        &["-j"][..],
        &["--jobs=0", "ls"],
        &["--threads", "-1", "ls"],
        &["ls", "-j"],
        &["ls", "--jobs="],
        &["ls", "--threads", "many"],
    ] {
        let output = run(directory.path(), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(output.stdout, b"");
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("job count must be a positive integer"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn preserves_non_utf8_names_and_symlink_targets() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    let name = OsStr::from_bytes(b"target-\xff");
    fs::write(directory.path().join(name), []).unwrap();

    let output = run(directory.path(), &["ls"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"target-\xff\n");

    symlink(name, directory.path().join("link")).unwrap();
    let long = run(directory.path(), &["ls", "-l", "link"]);
    assert!(long.status.success());
    assert!(long.stdout.ends_with(b" link -> target-\xff\n"));
}

#[cfg(unix)]
#[test]
fn lists_symlink_targets_in_long_format() {
    use std::os::unix::fs::symlink;

    let directory = tempdir().unwrap();
    fs::write(directory.path().join("target"), []).unwrap();
    symlink("target", directory.path().join("link")).unwrap();

    let long = run(directory.path(), &["ls", "-l", "link"]);
    assert!(long.status.success());
    assert!(long.stdout.ends_with(b" link -> target\n"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn sorts_file_names(names in prop::collection::btree_set("file-[a-z0-9]{1,8}", 0..20)) {
        let directory = tempdir().unwrap();
        for name in &names {
            fs::write(directory.path().join(name), []).unwrap();
        }
        let output = run(directory.path(), &["ls", "-1"]);
        prop_assert!(output.status.success());
        let actual = String::from_utf8(output.stdout).unwrap();
        let expected: Vec<_> = names.into_iter().collect();
        prop_assert_eq!(actual.lines().collect::<Vec<_>>(), expected);
    }
}
