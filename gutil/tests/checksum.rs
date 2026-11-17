#![allow(clippy::unwrap_used)]

use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(directory: &Path, arguments: &[&OsStr], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(directory)
        .arg("--no-config")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if !input.is_empty() {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn checksum(directory: &Path, arguments: &[&str], input: &[u8]) -> Output {
    let arguments: Vec<_> = arguments.iter().map(OsStr::new).collect();
    run(directory, &arguments, input)
}

#[test]
fn hashes_the_same_input_with_each_algorithm() {
    let directory = tempfile::tempdir().unwrap();
    for (algorithm, digest) in [
        ("md5sum", "900150983cd24fb0d6963f7d28e17f72"),
        ("sha1sum", "a9993e364706816aba3e25717850c26c9cd0d89d"),
        (
            "sha224sum",
            "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7",
        ),
        (
            "sha256sum",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "sha384sum",
            concat!(
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded163",
                "1a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
            ),
        ),
        (
            "sha512sum",
            concat!(
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
            ),
        ),
    ] {
        let output = checksum(directory.path(), &[algorithm], b"abc");
        assert!(output.status.success());
        assert_eq!(output.stdout, format!("{digest}  -\n").as_bytes());
    }
}

#[test]
fn verifies_records_and_reports_missing_and_incorrect_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("good"), b"abc").unwrap();
    fs::write(path.join("other"), b"abc").unwrap();
    fs::write(path.join("lost"), b"abc").unwrap();
    let record = checksum(path, &["sha256sum", "-b", "good", "other", "lost"], b"");
    assert!(record.status.success());
    fs::write(path.join("manifest"), &record.stdout).unwrap();

    let verified = checksum(path, &["sha256sum", "--check", "manifest"], b"");
    assert!(verified.status.success());
    assert_eq!(verified.stdout, b"good: OK\nother: OK\nlost: OK\n");

    fs::remove_file(path.join("lost")).unwrap();
    let ignored = checksum(
        path,
        &["sha256sum", "-c", "--ignore-missing"],
        &record.stdout,
    );
    assert!(ignored.status.success());
    assert_eq!(ignored.stdout, b"good: OK\nother: OK\n");
    let quiet = checksum(path, &["sha256sum", "-c", "--quiet", "manifest"], b"");
    assert!(!quiet.status.success());
    assert_eq!(quiet.stdout, b"lost: FAILED open or read\n");
    assert!(String::from_utf8_lossy(&quiet.stderr).contains("could not be read"));

    fs::write(path.join("other"), b"wrong digest").unwrap();
    let failed = checksum(
        path,
        &["sha256sum", "-c", "--ignore-missing", "manifest"],
        b"",
    );
    assert!(!failed.status.success());
    assert_eq!(failed.stdout, b"good: OK\nother: FAILED\n");
    let status = checksum(
        path,
        &[
            "sha256sum",
            "-c",
            "--status",
            "--ignore-missing",
            "manifest",
        ],
        b"",
    );
    assert!(!status.status.success());
    assert_eq!(status.stdout, b"");
    assert_eq!(status.stderr, b"");
}

#[test]
fn supports_tags_and_enforces_manifest_policy() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("name(with parentheses)"), b"abc").unwrap();
    let tag = checksum(path, &["sha1sum", "--tag", "name(with parentheses)"], b"");
    assert!(tag.status.success());
    assert!(tag.stdout.starts_with(b"SHA1 (name(with parentheses)) = "));
    let mut manifest = b"# comment\nmalformed\n".to_vec();
    manifest.extend(tag.stdout);
    let normal = checksum(path, &["sha1sum", "-c"], &manifest);
    assert!(normal.status.success());
    assert_eq!(normal.stdout, b"name(with parentheses): OK\n");
    let strict = checksum(path, &["sha1sum", "-cw", "--strict"], &manifest);
    assert!(!strict.status.success());
    assert!(String::from_utf8_lossy(&strict.stderr).contains(": 2: improperly formatted"));
    let mismatch_algorithm = checksum(path, &["sha256sum", "--check"], &manifest);
    assert!(!mismatch_algorithm.status.success());
    assert!(String::from_utf8_lossy(&mismatch_algorithm.stderr).contains("no properly formatted"));

    for flags in [
        &["--zero", "--check"][..],
        &["--tag", "--text"],
        &["--quiet"],
        &["--unknown"],
    ] {
        let arguments = [&["sha1sum"][..], flags].concat();
        assert!(!checksum(path, &arguments, b"").status.success());
    }
}

#[test]
fn respects_operand_order_option_end_and_command_help() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    fs::write(path.join("-left"), b"one").unwrap();
    fs::write(path.join("right"), vec![b'x'; 1024 * 1024]).unwrap();
    let sequential = checksum(path, &["--jobs=1", "md5sum", "--", "-left", "right"], b"");
    assert!(sequential.status.success());
    let parallel = checksum(path, &["--jobs=4", "md5sum", "--", "-left", "right"], b"");
    assert!(parallel.status.success());
    assert_eq!(parallel.stdout, sequential.stdout);
    let failed = checksum(path, &["md5sum", "--", "missing", "-left"], b"");
    assert!(!failed.status.success());
    assert!(failed.stdout.ends_with(b"  -left\n"));

    let progress = checksum(
        path,
        &[
            "--progress=always",
            "md5sum",
            "--binary",
            "--text",
            "--",
            "-left",
            "right",
        ],
        b"",
    );
    assert!(progress.status.success());
    assert_eq!(progress.stdout, sequential.stdout);
    assert!(String::from_utf8_lossy(&progress.stderr).contains("bytes"));

    fs::write(path.join("gutil.toml"), b"invalid toml").unwrap();
    for flag in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .current_dir(path)
            .args(["sha384sum", flag])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("gutil sha384sum"));
    }
}

#[test]
fn handles_unreadable_manifests_and_skipped_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let missing = checksum(path, &["sha1sum", "-c", "missing-manifest"], b"");
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing-manifest"));

    let manifest = b"da39a3ee5e6b4b0d3255bfef95601890afd80709  missing\n";
    let ignored = checksum(path, &["sha1sum", "-c", "--ignore-missing"], manifest);
    assert!(!ignored.status.success());
    assert_eq!(ignored.stdout, b"");
    assert!(String::from_utf8_lossy(&ignored.stderr).contains("no file was verified"));

    let malformed = b"\\da39a3ee5e6b4b0d3255bfef95601890afd80709  bad\\q\n";
    let warned = checksum(path, &["sha1sum", "-c", "--warn"], malformed);
    assert!(!warned.status.success());
    assert!(String::from_utf8_lossy(&warned.stderr).contains(": 1: improperly formatted"));
}

#[cfg(unix)]
#[test]
fn round_trips_native_filenames_with_escapes() {
    use std::os::unix::ffi::OsStrExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let name = OsStr::from_bytes(b"file\\name\n\r\xff");
    fs::write(path.join(name), b"abc").unwrap();

    for flags in [
        vec![OsStr::new("sha512sum")],
        vec![OsStr::new("sha512sum"), OsStr::new("--tag")],
    ] {
        let output = run(path, &[flags, vec![name]].concat(), b"");
        assert!(output.status.success());
        assert!(output.stdout.starts_with(b"\\"));
        assert!(output
            .stdout
            .windows(b"file\\\\name\\n\\r\xff".len())
            .any(|part| part == b"file\\\\name\\n\\r\xff"));
        let verified = checksum(path, &["sha512sum", "--check"], &output.stdout);
        assert!(verified.status.success());
        assert_eq!(verified.stdout, b"\\file\\\\name\\n\\r\xff: OK\n");
    }
    let zero = run(
        path,
        &[OsStr::new("sha512sum"), OsStr::new("-z"), name],
        b"",
    );
    assert!(zero.status.success());
    assert!(zero.stdout.ends_with(b"  file\\name\n\r\xff\0"));
    assert_ne!(zero.stdout[0], b'\\');
}

#[test]
fn keeps_a_stdin_manifest_separate_from_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path();
    let remaining = b"da39a3ee5e6b4b0d3255bfef95601890afd80709  missing\n";
    let stdin_record = checksum(path, &["sha1sum"], remaining);
    assert!(stdin_record.status.success());
    let mut manifest = stdin_record.stdout.clone();
    manifest.extend(remaining);

    let rejected = checksum(path, &["sha1sum", "--check", "--warn"], &manifest);
    assert_eq!(rejected.status.code(), Some(1));
    assert_eq!(rejected.stdout, b"missing: FAILED open or read\n");
    assert!(String::from_utf8_lossy(&rejected.stderr).contains(": 1: improperly formatted"));

    fs::write(path.join("manifest"), &stdin_record.stdout).unwrap();
    let verified = checksum(path, &["sha1sum", "--check", "manifest"], remaining);
    assert!(verified.status.success());
    assert_eq!(verified.stdout, b"-: OK\n");
}
