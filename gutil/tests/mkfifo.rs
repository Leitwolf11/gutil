#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::Command;

const GUTIL: &str = env!("CARGO_BIN_EXE_gutil");

fn command(path: &Path) -> Command {
    let mut command = Command::new(GUTIL);
    command
        .current_dir(path)
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--progress=never", "--color=never", "mkfifo"]);
    command
}

#[test]
fn checks_help_version_and_invalid_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let help = command(path).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(help.stdout.starts_with(b"Usage: gutil mkfifo "));
    let version = command(path).arg("--vers").output().unwrap();
    assert!(version.status.success());
    assert!(version.stdout.starts_with(b"gutil mkfifo "));
    for args in [
        &[][..],
        &["--mode"],
        &["--unknown"],
        &["-W"],
        &["-m", "888", "bad"],
    ] {
        let output = command(path).args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.starts_with(b"gutil mkfifo:"));
    }
    assert!(!path.join("bad").exists());
}

#[cfg(not(unix))]
#[test]
fn explains_that_named_fifo_creation_is_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let output = command(path).arg("pipe").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("FIFO creation is unavailable"));
    assert!(!path.join("pipe").exists());
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::{symlink, FileTypeExt, PermissionsExt};
    use std::process::Output;

    use super::*;

    fn with_umask(binary: &Path, path: &Path, options: &[&str], args: &[&str]) -> Output {
        Command::new("sh")
            .current_dir(path)
            .env("LC_ALL", "C")
            .env_remove("POSIXLY_CORRECT")
            .args(["-c", "umask 027; exec \"$@\"", "sh"])
            .arg(binary)
            .args(options)
            .args(args)
            .output()
            .unwrap()
    }

    fn permissions(path: &Path) -> u32 {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(metadata.file_type().is_fifo());
        metadata.permissions().mode() & 0o7777
    }

    #[test]
    fn applies_default_and_explicit_fifo_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        for (args, mode) in [
            (&[][..], 0o640),
            (&["-m666"], 0o666),
            (&["--mode==rw"], 0o640),
            (&["--mo", "a+X"], 0o666),
            (&["-m", "u+x,g+X"], 0o776),
            (&["-m", "u=r,g=u,o="], 0o440),
            (&["-m", "u+x,u-X"], 0o666),
            (&["--jobs=2", "-m", "bad", "--mode=600"], 0o600),
        ] {
            let flags = ["--no-config", "--progress=never", "mkfifo"];
            let args = [args, &["pipe"]].concat();
            let actual = with_umask(Path::new(GUTIL), path, &flags, &args);
            assert!(actual.status.success(), "{actual:?}");
            assert_eq!(permissions(&path.join("pipe")), mode);
            fs::remove_file(path.join("pipe")).unwrap();
        }
        let invalid = command(path).args(["-m", "1600", "pipe"]).output().unwrap();
        assert_eq!(invalid.status.code(), Some(1));
        assert!(!path.join("pipe").exists());
    }

    #[test]
    fn creates_working_pipes_and_preserves_existing_objects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        fs::write(path.join("keep"), b"unchanged").unwrap();
        symlink("keep", path.join("alias")).unwrap();
        fs::create_dir(path.join("nested")).unwrap();
        let name = OsString::from_vec(b"pipe-\xff".to_vec());
        let output = command(path)
            .args([
                "-m600",
                "alias",
                "missing/pipe",
                "nested/fifo",
                "--",
                "-dash",
            ])
            .arg(&name)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"unchanged");
        assert!(path.join("alias").is_symlink());
        for fifo in [
            path.join("nested/fifo"),
            path.join("-dash"),
            path.join(name),
        ] {
            assert_eq!(permissions(&fifo), 0o600);
        }
        let fifo = path.join("nested/fifo");
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                fs::OpenOptions::new()
                    .write(true)
                    .open(&fifo)
                    .unwrap()
                    .write_all(b"message")
                    .unwrap();
            });
            let mut bytes = Vec::new();
            fs::File::open(&fifo)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            writer.join().unwrap();
            assert_eq!(bytes, b"message");
        });
    }

    #[test]
    fn handles_context_flags_and_keeps_progress_off_stdout() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        for (flags, name) in [
            ("--context", "default-context"),
            ("--context=requested", "explicit-context"),
            ("-Zj2", "clustered-context"),
        ] {
            let output = command(path).args([flags, name]).output().unwrap();
            assert_eq!(output.stdout, b"");
            if output.status.success() {
                assert!(fs::symlink_metadata(path.join(name))
                    .unwrap()
                    .file_type()
                    .is_fifo());
                if flags == "--context=requested" {
                    assert!(String::from_utf8_lossy(&output.stderr).contains("ignoring context"));
                }
            } else {
                assert!(String::from_utf8_lossy(&output.stderr)
                    .contains("context assignment is unavailable"));
                assert!(!path.join(name).exists());
            }
        }

        let output = Command::new(GUTIL)
            .current_dir(path)
            .env("POSIXLY_CORRECT", "1")
            .args([
                "--no-config",
                "--progress=always",
                "mkfifo",
                "first",
                "-last",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"");
        assert!(String::from_utf8_lossy(&output.stderr).contains("FIFOs"));
        assert!(path.join("-last").exists());
    }

    #[test]
    fn matches_gnu_output_status_and_modes_when_configured() {
        let Some(bin) = std::env::var_os("GNU_COREUTILS_DIR") else {
            return;
        };
        let reference = Path::new(&bin).join("mkfifo");
        let ours = tempfile::tempdir().unwrap();
        let gnu = tempfile::tempdir().unwrap();
        let options = ["--no-config", "--progress=never", "mkfifo"];
        for (index, args) in [
            &[][..],
            &["-m", "000"],
            &["-m", "-w"],
            &["-Z"],
            &["--context=label"],
            &["-m", "a+t"],
            &["-m", "a+X"],
            &["-m", "u+x,g+X"],
            &["-m", "u=rw,go="],
        ]
        .into_iter()
        .enumerate()
        {
            // Hosts with active labeling require native policy and independent fixtures.
            if cfg!(target_os = "linux")
                && matches!(args.first(), Some(&"-Z" | &"--context=label"))
                && Path::new("/sys/fs/selinux/enforce").exists()
            {
                continue;
            }
            let name = format!("pipe-{index}");
            let args = [args, &[name.as_str()]].concat();
            let actual = with_umask(Path::new(GUTIL), ours.path(), &options, &args);
            let desired = with_umask(&reference, gnu.path(), &[], &args);
            assert_eq!(
                actual.status.code(),
                desired.status.code(),
                "{args:?}: {actual:?}"
            );
            assert_eq!(actual.stdout, desired.stdout, "{args:?}");
            assert_eq!(
                actual.stderr.is_empty(),
                desired.stderr.is_empty(),
                "{args:?}"
            );
            if desired.status.success() {
                assert_eq!(
                    permissions(&ours.path().join(&name)),
                    permissions(&gnu.path().join(&name)),
                    "{args:?}"
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn preserves_inherited_acl_and_applies_the_requested_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path();
        let Ok(acl) = Command::new("setfacl")
            .args(["-m", "d:u::rw-,d:g::r--,d:o::---,d:u:65534:r--"])
            .arg(path)
            .output()
        else {
            return;
        };
        if !acl.status.success() {
            return;
        }
        let default = command(path).arg("inherited").output().unwrap();
        assert!(default.status.success(), "{default:?}");
        let explicit = command(path).args(["-m666", "explicit"]).output().unwrap();
        assert!(explicit.status.success(), "{explicit:?}");
        assert_eq!(permissions(&path.join("inherited")), 0o640);
        assert_eq!(permissions(&path.join("explicit")), 0o666);
        let acl = Command::new("getfacl")
            .args(["-cn", "explicit"])
            .current_dir(path)
            .output()
            .unwrap();
        assert!(acl.status.success(), "{acl:?}");
        assert!(String::from_utf8_lossy(&acl.stdout).contains("user:65534:r--"));
    }
}
