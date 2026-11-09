#![allow(clippy::unwrap_used)]

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn run(directory: &Path, pwd: Option<&OsStr>, posix: bool, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .arg("pwd")
        .args(args)
        .current_dir(directory)
        .env_remove("PWD")
        .env_remove("POSIXLY_CORRECT");
    if let Some(pwd) = pwd {
        command.env("PWD", pwd);
    }
    if posix {
        command.env("POSIXLY_CORRECT", "");
    }
    command.output().unwrap()
}

fn path_line(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    let mut bytes = path.as_os_str().as_encoded_bytes().to_vec();
    #[cfg(not(unix))]
    let mut bytes = path.to_string_lossy().as_bytes().to_vec();
    bytes.push(b'\n');
    bytes
}

#[test]
fn prints_a_physical_path_when_pwd_is_unusable() {
    let directory = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let physical = fs::canonicalize(directory.path()).unwrap();
    let with_dot = directory.path().join(".");
    let with_parent = directory
        .path()
        .join("..")
        .join(directory.path().file_name().unwrap());

    let default = run(
        directory.path(),
        Some(elsewhere.path().as_os_str()),
        false,
        &[],
    );
    assert!(default.status.success());
    assert_eq!(default.stdout, path_line(&physical));
    for pwd in [
        None,
        Some(OsStr::new("relative")),
        Some(elsewhere.path().as_os_str()),
        Some(with_dot.as_os_str()),
        Some(with_parent.as_os_str()),
    ] {
        let output = run(directory.path(), pwd, false, &["--logical"]);
        assert!(output.status.success());
        assert_eq!(output.stdout, path_line(&physical));
    }
}

#[test]
fn accepts_pwd_when_it_names_the_current_directory() {
    let directory = tempfile::tempdir().unwrap();
    let result = run(
        directory.path(),
        Some(directory.path().as_os_str()),
        false,
        &["-L"],
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, path_line(directory.path()));
}

#[test]
fn prints_command_help_and_handles_extra_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let help = run(directory.path(), None, false, &["--help"]);
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: gutil pwd "));

    let version = run(directory.path(), None, false, &["--version"]);
    assert!(version.status.success());
    assert_eq!(
        version.stdout,
        format!("gutil pwd {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );

    for args in [&["--unknown"][..], &["-x"], &["--threads=0"], &["-Lj0"]] {
        let invalid = run(directory.path(), None, false, args);
        assert_eq!(invalid.status.code(), Some(1));
        assert_eq!(invalid.stdout, b"");
    }

    let extra = run(directory.path(), None, false, &["--", "--unknown"]);
    assert!(extra.status.success());
    assert!(String::from_utf8_lossy(&extra.stderr).contains("ignoring non-option arguments"));
    assert_eq!(
        extra.stdout,
        path_line(&fs::canonicalize(directory.path()).unwrap())
    );
}

#[cfg(unix)]
#[test]
fn uses_valid_symlink_paths_only_in_logical_mode() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    let alias = directory.path().join("alias");
    fs::create_dir(&real).unwrap();
    symlink("real", &alias).unwrap();
    let physical = fs::canonicalize(&real).unwrap();
    let pwd = Some(alias.as_os_str());

    for (posix, args, expected) in [
        (false, &[][..], &physical),
        (true, &[][..], &alias),
        (false, &["--log"], &alias),
        (false, &["-LP"], &physical),
        (false, &["-PLj2"], &alias),
        (false, &["-L", "--physical"], &physical),
        (false, &["--jobs=2", "extra", "-L"], &alias),
        (true, &["extra", "-P"], &alias),
    ] {
        let output = run(&real, pwd, posix, args);
        assert!(output.status.success(), "{args:?}: {:?}", output.stderr);
        assert_eq!(output.stdout, path_line(expected), "{args:?}");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn writes_non_utf8_directory_names_as_native_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().unwrap();
    let native = directory.path().join(OsStr::from_bytes(b"raw-\xff"));
    fs::create_dir(&native).unwrap();
    let output = run(&native, None, false, &["-P"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, path_line(&fs::canonicalize(native).unwrap()));
}
