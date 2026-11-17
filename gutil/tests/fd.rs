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
    let absolute = run(root, &["-ap", "-d2", "other\\.txt$"]);
    assert!(absolute.status.success());
    let absolute = String::from_utf8(absolute.stdout).unwrap();
    let absolute = Path::new(absolute.strip_suffix('\n').unwrap());
    assert!(absolute.is_absolute());
    assert_eq!(
        fs::canonicalize(absolute).unwrap(),
        fs::canonicalize(root.join("dir/other.txt")).unwrap()
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
        assert_eq!(run(root, &["^native-.$"]).stdout, b"native-\xff\n");
    }
}

#[test]
fn enforces_directory_boundaries_in_full_path_globs() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let sep = MAIN_SEPARATOR_STR;
    fs::create_dir_all(root.join("tree/a/b")).unwrap();
    fs::write(root.join("tree/a/leaf.rs"), []).unwrap();
    fs::write(root.join("tree/a/b/leaf.rs"), []).unwrap();
    check(
        &run(root, &["-gp", "**/tree/*/leaf.rs"]),
        &format!("tree{sep}a{sep}leaf.rs\n"),
    );
    check(
        &run(root, &["-gp", "**/tree/**/leaf.rs"]),
        &format!("tree{sep}a{sep}b{sep}leaf.rs\ntree{sep}a{sep}leaf.rs\n"),
    );
}

#[test]
fn reports_attached_ignore_file_diagnostics_without_stopping_the_search() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::write(root.join(".fdignore"), "[z-a]\n").unwrap();
    fs::write(root.join("match"), []).unwrap();
    let silent = run(root, &["match"]);
    check(&silent, "match\n");
    assert_eq!(silent.stderr, b"");
    for worker in ["-j1", "-j4"] {
        let output = run(root, &[worker, "--show-errors", "match"]);
        check(&output, "match\n");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(".fdignore") && stderr.contains("invalid range"));
    }
}

#[test]
fn restores_ignore_rules_and_validates_value_options() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join(".ignore"), "parent*.txt\n").unwrap();
    fs::write(root.join(".fdignore"), "ignore.txt\n").unwrap();
    fs::write(root.join(".gitignore"), "vcs.txt\n").unwrap();
    for name in [
        ".hide.txt",
        "ignore.txt",
        "vcs.txt",
        "-leaf.txt",
        "nested/parent.txt",
    ] {
        fs::write(root.join(name), []).unwrap();
    }
    let sep = MAIN_SEPARATOR_STR;
    check(&run(root, &["--hidden", "--no-hidden", "\\.hide"]), "");
    check(&run(root, &["--no-ignore", "--ignore", "^ignore"]), "");
    check(
        &run(
            root,
            &[
                "--no-require-git",
                "--no-ignore-vcs",
                "--ignore-vcs",
                "^vcs",
            ],
        ),
        "",
    );
    check(
        &run(root, &["--no-require-git", "--require-git", "^vcs"]),
        "vcs.txt\n",
    );
    check(
        &run(root, &["--no-ignore-parent", "parent", "nested"]),
        &format!("nested{sep}parent.txt\n"),
    );
    check(
        &run(
            root,
            &["--no-ignore-parent", "--ignore-parent", "parent", "nested"],
        ),
        "",
    );
    check(&run(root, &["-qj1", "--glob", "--regex", "^vcs"]), "");
    check(
        &run(
            root,
            &[
                "--max-depth",
                "1",
                "--extension=txt",
                "--exclude=vcs*",
                "--",
                "-leaf",
            ],
        ),
        "-leaf.txt\n",
    );
    for arguments in [
        &["--unknown"][..],
        &["-Z"],
        &["--extension"],
        &["--exclude", "[z-a]"],
        &["", "vcs.txt"],
    ] {
        let output = run(root, arguments);
        assert_eq!(output.status.code(), Some(1), "{arguments:?}");
        assert_ne!(output.stderr, b"");
    }
    let version = format!("gutil fd {}\n", env!("CARGO_PKG_VERSION"));
    check(&run(root, &["-V"]), &version);
}

#[cfg(unix)]
#[test]
fn reports_symlink_loops_and_invalid_option_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::{ffi::OsStrExt, fs::symlink};

    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    symlink(".", root.join("loop")).unwrap();
    let output = run(root, &["-Lj1", "--show-errors", "unavailable"]);
    check(&output, "");
    assert!(String::from_utf8(output.stderr).unwrap().contains("loop"));
    for raw in [b"--\xff".as_slice(), b"-e\xff"] {
        let result = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .current_dir(root)
            .args(["--no-config", "fd"])
            .arg(OsStr::from_bytes(raw))
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
    }
}
