#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(directory)
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--progress=never"]);
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn verify(
    Output {
        status,
        stdout,
        stderr,
    }: Output,
    expected: &[u8],
) {
    assert!(status.success(), "{}", String::from_utf8_lossy(&stderr));
    assert_eq!(stdout, expected);
}

#[test]
fn extracts_components_without_accessing_the_filesystem() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    verify(
        run(directory, &["basename", "dir/hidden/../next///", "xt"]),
        b"ne\n",
    );
    verify(run(directory, &["basename", "dir/.txt", ".txt"]), b".txt\n");
    verify(
        run(
            directory,
            &[
                "basename",
                "-zs.txt",
                "nested/a.txt",
                "",
                "////",
                "-leading.txt",
            ],
        ),
        b"a\0\0/\0-leading\0",
    );
    verify(
        run(
            directory,
            &["dirname", "-z", "/", "", "leaf", "a//b/c///", "a/.", "a//b"],
        ),
        b"/\0.\0.\0a//b\0a\0a\0",
    );
    verify(run(directory, &["basename", "--", "--help"]), b"--help\n");
    assert_eq!(run(directory, &["basename"]).status.code(), Some(1));
    assert_eq!(
        run(directory, &["basename", "a", "b", "c"]).status.code(),
        Some(1)
    );
    assert_eq!(
        run(directory, &["dirname", "--multiple", "a"])
            .status
            .code(),
        Some(1)
    );
}

#[cfg(unix)]
#[test]
fn preserves_native_name_bytes() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    verify(
        command(directory)
            .args(["basename", "-z"])
            .arg(OsStr::from_bytes(b"a/\xff.\xfe"))
            .arg(OsStr::from_bytes(b".\xfe"))
            .output()
            .unwrap(),
        b"\xff\0",
    );
    verify(
        command(directory)
            .args(["dirname", "-z"])
            .arg(OsStr::from_bytes(b"\xff/x"))
            .output()
            .unwrap(),
        b"\xff\0",
    );
}

#[cfg(windows)]
#[test]
fn handles_drive_and_unc_paths() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    verify(run(directory, &["basename", r"C:\one\two\\"]), b"two\n");
    verify(run(directory, &["dirname", r"C:\one\two\\"]), b"C:\\one\n");
    verify(
        run(directory, &["dirname", r"\\server\share\one"]),
        b"\\\\server\\share\\\n",
    );
}

#[test]
fn canonicalizes_missing_paths_and_formats_relative_names() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    fs::create_dir_all(directory.join("one/two")).unwrap();
    fs::write(directory.join("one/file"), b"file").unwrap();
    verify(
        run(
            directory,
            &[
                "realpath",
                "--relative-to=one",
                "one/two/../file",
                "one/missing/",
            ],
        ),
        b"file\nmissing\n",
    );
    let failure = run(
        directory,
        &[
            "realpath",
            "-q",
            "-e",
            "one/missing",
            "--relative-to=one",
            "one/file",
        ],
    );
    assert_eq!(failure.status.code(), Some(1));
    assert_eq!(failure.stdout, b"file\n");
    assert_eq!(failure.stderr, b"");
    let failure = run(directory, &["realpath", "one/file/", "one/missing/."]);
    assert_eq!(failure.status.code(), Some(1));
    assert_eq!(failure.stdout, b"");
    verify(
        run(
            directory,
            &[
                "realpath",
                "-mz",
                "--relative-to=one/two",
                "one/unavailable/foo/../out",
            ],
        ),
        if cfg!(windows) {
            b"..\\unavailable\\out\0"
        } else {
            b"../unavailable/out\0"
        },
    );
    verify(
        run(directory, &["realpath", "--relative-base=one", "one/two"]),
        b"two\n",
    );
    assert_eq!(
        run(directory, &["realpath", "--relative-to"]).status.code(),
        Some(1)
    );
}

#[cfg(unix)]
#[test]
fn reads_symlink_targets_and_resolves_parent_components_in_the_selected_order() {
    use std::os::unix::fs::symlink;

    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    fs::create_dir_all(directory.join("real/deep")).unwrap();
    symlink("real/deep", directory.join("alias")).unwrap();
    symlink("gone", directory.join("dangling")).unwrap();
    symlink("loop", directory.join("loop")).unwrap();
    verify(run(directory, &["readlink", "-n", "alias"]), b"real/deep");
    verify(
        run(directory, &["readlink", "-z", "alias", "dangling"]),
        b"real/deep\0gone\0",
    );
    let quiet = run(directory, &["readlink", "real"]);
    assert_eq!(quiet.status.code(), Some(1));
    assert_eq!(quiet.stderr, b"");
    let verbose = run(directory, &["readlink", "-v", "real"]);
    assert_eq!(verbose.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&verbose.stderr).contains("real"));
    verify(
        run(directory, &["realpath", "--relative-to=.", "alias/.."]),
        b"real\n",
    );
    verify(
        run(
            directory,
            &["realpath", "-L", "--relative-to=.", "alias/.."],
        ),
        b".\n",
    );
    verify(
        run(directory, &["realpath", "-s", "--relative-to=.", "alias"]),
        b"alias\n",
    );
    verify(
        run(directory, &["realpath", "--relative-to=.", "dangling/"]),
        b"gone\n",
    );
    let looped = run(directory, &["readlink", "-f", "loop"]);
    assert_eq!(looped.status.code(), Some(1));
    assert_eq!(looped.stdout, b"");
    let canonical = fs::canonicalize(directory).unwrap().join("gone");
    verify(
        run(directory, &["readlink", "-f", "dangling"]),
        format!("{}\n", canonical.display()).as_bytes(),
    );
}

#[test]
fn keeps_worker_results_in_operand_order_and_prints_command_help() {
    let fixture = tempfile::tempdir().unwrap();
    let directory = fixture.path();
    let mut args = vec!["realpath", "--threads=4", "--relative-to=."];
    let names = ["z", "q", "b", "s", "a", "d", "v", "m", "n"];
    args.extend(names);
    let expected = format!("{}\n", names.join("\n"));
    verify(run(directory, &args), expected.as_bytes());
    for name in ["basename", "dirname", "readlink", "realpath"] {
        let output = run(directory, &[name, "--help"]);
        assert!(output.status.success());
        assert!(output
            .stdout
            .starts_with(format!("Usage: gutil {name}").as_bytes()));
        verify(
            run(directory, &[name, "--version"]),
            format!("gutil {name} {}\n", env!("CARGO_PKG_VERSION")).as_bytes(),
        );
    }
}
