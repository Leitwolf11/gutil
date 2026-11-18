#[cfg(unix)]
use super::command;
use super::{run, success};

#[test]
fn truncates_sparse_files_and_applies_modifiers_in_operand_order() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("data"), b"abcdef").unwrap();
    for (size, length) in [
        ("3", 3),
        ("+2", 5),
        ("-9", 0),
        (">7", 7),
        ("<6", 6),
        ("%4", 8),
        ("/3", 6),
        ("1KiB", 1024),
    ] {
        success(&run(root, "truncate", &["--size", size, "data"]));
        assert_eq!(
            std::fs::metadata(root.join("data")).unwrap().len(),
            length,
            "{size}"
        );
    }
    assert_eq!(&std::fs::read(root.join("data")).unwrap()[..3], b"\0\0\0");
    success(&run(root, "truncate", &["-s+1", "data", "data"]));
    assert_eq!(std::fs::metadata(root.join("data")).unwrap().len(), 1026);
    success(&run(root, "truncate", &["-cs1", "missing"]));
    assert!(!root.join("missing").exists());
    success(&run(root, "truncate", &["-s0Q", "new"]));
    assert_eq!(std::fs::metadata(root.join("new")).unwrap().len(), 0);
    std::fs::create_dir(root.join("dir")).unwrap();
    assert_eq!(
        run(root, "truncate", &["-s2", "dir", "after-error"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        std::fs::metadata(root.join("after-error")).unwrap().len(),
        2
    );
}

#[test]
fn uses_reference_lengths_and_validates_before_creating_files() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("reference"), b"reference").unwrap();
    success(&run(root, "truncate", &["--reference=reference", "copy"]));
    assert_eq!(std::fs::metadata(root.join("copy")).unwrap().len(), 9);
    success(&run(
        root,
        "truncate",
        &["-rreference", "-s+3", "copy", "other"],
    ));
    for name in ["copy", "other"] {
        assert_eq!(std::fs::metadata(root.join(name)).unwrap().len(), 12);
    }
    for args in [
        &["file"][..],
        &["-s", "1", "-rreference", "file"],
        &["-o", "-rreference", "file"],
        &["-s/0", "file"],
        &["-s%0", "file"],
        &["-s1b", "file"],
        &["-s", "file"],
        &["-s9223372036854775808", "file"],
        &["-rmissing", "file"],
    ] {
        assert_eq!(
            run(root, "truncate", args).status.code(),
            Some(1),
            "{args:?}"
        );
        assert!(!root.join("file").exists());
    }
}

#[cfg(unix)]
#[test]
fn handles_native_reference_names_and_io_block_units() {
    #[cfg(target_os = "linux")]
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    #[cfg(target_os = "linux")]
    let native = std::ffi::OsStr::from_bytes(b"native-\xff");
    #[cfg(not(target_os = "linux"))]
    let native = std::ffi::OsStr::new("native-é");
    std::fs::write(root.join(native), b"abc").unwrap();
    let mut flag = std::ffi::OsString::from("--reference=");
    flag.push(native);
    let output = command(root, "truncate", &["-s+1", "copy"])
        .arg(flag)
        .output()
        .unwrap();
    success(&output);
    assert_eq!(std::fs::metadata(root.join("copy")).unwrap().len(), 4);
    let block = std::fs::metadata(root.join("copy")).unwrap().blksize();
    success(&run(root, "truncate", &["--io-blocks", "-s2", "copy"]));
    assert_eq!(
        std::fs::metadata(root.join("copy")).unwrap().len(),
        block * 2
    );
}

#[cfg(windows)]
#[test]
fn uses_windows_filesystem_allocation_units_for_io_block_sizes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(fixture.path()).unwrap();
    let block = fs4::allocation_granularity(&root).unwrap();
    success(&run(
        &root,
        "truncate",
        &["--io-blocks", "--size=2", "new-file"],
    ));
    assert_eq!(
        std::fs::metadata(root.join("new-file")).unwrap().len(),
        2 * block
    );
}

#[cfg(target_os = "linux")]
#[test]
fn reports_fifo_operands_without_waiting_for_a_reader() {
    use rustix::fs::{mkfifoat, Mode, OFlags, CWD};
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let fifo = root.join("fifo");
    mkfifoat(CWD, &fifo, Mode::from_raw_mode(0o600)).unwrap();
    for with_reader in [false, true] {
        let _reader = with_reader.then(|| {
            rustix::fs::open(&fifo, OFlags::RDONLY | OFlags::NONBLOCK, Mode::empty()).unwrap()
        });
        let mut child = command(root, "truncate", &["-s2", "fifo", "later"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("truncate blocked on a FIFO");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(std::fs::metadata(root.join("later")).unwrap().len(), 2);
    }
    success(&run(root, "truncate", &["--reference=fifo", "later"]));
    assert_eq!(std::fs::metadata(root.join("later")).unwrap().len(), 0);
}
