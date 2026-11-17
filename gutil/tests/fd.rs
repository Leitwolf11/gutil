#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, MAIN_SEPARATOR_STR};
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(root)
        .env("LC_ALL", "C")
        .args(["--no-config", "--progress=never", "fd"])
        .args(args)
        .output()
        .unwrap()
}

fn check(output: &Output, expected: &str) {
    assert!(output.status.success(), "{:?}", output.stderr);
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn searches_with_ignore_rules_and_preserves_sorted_worker_results() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("nest")).unwrap();
    fs::write(root.join(".fdignore"), "skip*\n").unwrap();
    fs::write(root.join(".gitignore"), "vcs*\n").unwrap();
    fs::write(root.join(".hide.rs"), []).unwrap();
    fs::write(root.join("Alpha.RS"), []).unwrap();
    fs::write(root.join("skip.rs"), []).unwrap();
    fs::write(root.join("vcs.rs"), []).unwrap();
    fs::write(root.join("nest/alpha.rs"), []).unwrap();
    let alpha = format!("nest{MAIN_SEPARATOR_STR}alpha.rs\n");
    for jobs in ["--jobs=1", "--threads=4"] {
        check(
            &run(root, &[jobs, "-tf", "-e", ".RS", "alpha"]),
            &format!("Alpha.RS\n{alpha}"),
        );
    }
    check(&run(root, &["-sF", "alpha"]), &alpha);
    check(&run(root, &["-g", "A*.RS"]), "Alpha.RS\n");
    check(
        &run(root, &["-i", "--exact", "ALPHA.RS"]),
        &format!("Alpha.RS\n{alpha}"),
    );
    check(&run(root, &["--no-require-git", "--exact", "vcs.rs"]), "");
    check(
        &run(
            root,
            &["--no-require-git", "--no-ignore-vcs", "--exact", "vcs.rs"],
        ),
        "vcs.rs\n",
    );
    check(&run(root, &["-u", "--exact", ".hide.rs"]), ".hide.rs\n");
    check(&run(root, &["-I", "--exact", "skip.rs"]), "skip.rs\n");
    check(
        &run(root, &["-E", "nest", "--exact", "alpha.rs"]),
        "Alpha.RS\n",
    );
}

#[test]
fn formats_paths_and_honors_depth_and_quiet_options() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir_all(root.join("dir/sub")).unwrap();
    fs::write(root.join("dir/sub/file.txt"), []).unwrap();
    fs::write(root.join("dir/other.txt"), []).unwrap();
    let sep = MAIN_SEPARATOR_STR;
    check(
        &run(root, &["-0", "-d1", "-td"]),
        &format!(".{sep}dir{sep}\0"),
    );
    check(
        &run(root, &["--exact-depth=2", "-tf", "", "dir"]),
        &format!("dir{sep}sub{sep}file.txt\n"),
    );
    let absolute = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let expected = if cfg!(windows) {
        root
    } else {
        absolute.as_path()
    };
    check(
        &run(root, &["-ap", "-d2", "other\\.txt$"]),
        &format!("{}{sep}dir{sep}other.txt\n", expected.display()),
    );
    check(&run(root, &["-q", "file"]), "");
    assert_eq!(run(root, &["-q", "unavailable"]).status.code(), Some(1));
    for args in [
        &["--type=invalid"][..],
        &["--min-depth=4", "-d2"],
        &["[invalid"],
        &["--glob", "[invalid"],
        &["", "missing"],
    ] {
        let output = run(root, args);
        assert_eq!(output.status.code(), Some(1));
        assert_ne!(output.stderr, b"");
    }
    assert!(run(root, &["--help"])
        .stdout
        .starts_with(b"Usage: gutil fd"));
}

#[cfg(unix)]
#[test]
fn handles_symlinks_and_native_name_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::{ffi::OsStrExt, fs::symlink};

    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file"), []).unwrap();
    symlink("source", root.join("alias")).unwrap();
    check(&run(root, &["-tl"]), "alias\n");
    check(
        &run(root, &["-L", "-tf", "--exact", "file"]),
        "alias/file\nsource/file\n",
    );
    if !cfg!(target_vendor = "apple") {
        fs::write(root.join(OsStr::from_bytes(b"native-\xff")), []).unwrap();
        assert_eq!(run(root, &["-F", "native-"]).stdout, b"native-\xff\n");
    }
}
