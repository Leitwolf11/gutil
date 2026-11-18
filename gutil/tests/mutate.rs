#![allow(clippy::unwrap_used)]

use std::io::Write;
#[cfg(unix)]
#[path = "mutate/gnu_parity.rs"]
mod gnu_parity;

use std::path::Path;
use std::process::{Command, Output, Stdio};

fn command(root: &Path, utility: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env_remove("VERSION_CONTROL")
        .env_remove("SIMPLE_BACKUP_SUFFIX")
        .args(["--no-config", "--color=never", "--progress=never", utility])
        .args(args);
    command
}

fn run(root: &Path, utility: &str, args: &[&str]) -> Output {
    command(root, utility, args).output().unwrap()
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn link_and_unlink_preserve_content_and_refuse_extra_operands() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("original"), b"content").unwrap();
    success(&run(root, "link", &["--threads=2", "original", "linked"]));
    assert!(same_file::is_same_file(root.join("original"), root.join("linked")).unwrap());
    assert_eq!(
        run(root, "link", &["original", "linked"]).status.code(),
        Some(1)
    );
    success(&run(root, "unlink", &["linked"]));
    assert_eq!(std::fs::read(root.join("original")).unwrap(), b"content");
    std::fs::create_dir(root.join("directory")).unwrap();
    for (utility, args) in [
        ("link", &[][..]),
        ("link", &["original"]),
        ("link", &["original", "extra", "operand"]),
        ("unlink", &[]),
        ("unlink", &["original", "extra"]),
        ("unlink", &["directory"]),
        ("unlink", &["missing"]),
    ] {
        assert_eq!(
            run(root, utility, args).status.code(),
            Some(1),
            "{utility} {args:?}"
        );
    }
    assert!(root.join("directory").is_dir());
}

#[test]
fn removes_empty_parents_in_order_and_only_ignores_nonempty_directories() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("tree/branch/leaf")).unwrap();
    let verbose = run(root, "rmdir", &["-pvj2", "tree/branch/leaf/"]);
    assert!(String::from_utf8_lossy(&verbose.stdout).contains("tree/branch/leaf"));
    success(&verbose);
    assert!(!root.join("tree").exists());
    std::fs::create_dir_all(root.join("nonempty/child")).unwrap();
    std::fs::create_dir(root.join("empty")).unwrap();
    success(&run(
        root,
        "rmdir",
        &["--ignore-fail-on-non-empty", "nonempty", "empty"],
    ));
    assert!(root.join("nonempty/child").is_dir());
    assert!(!root.join("empty").exists());
    assert_eq!(
        run(root, "rmdir", &["--ignore-fail-on-non-empty", "missing"])
            .status
            .code(),
        Some(1)
    );
    std::fs::create_dir(root.join("later")).unwrap();
    assert_eq!(
        run(root, "rmdir", &["missing", "later"]).status.code(),
        Some(1)
    );
    assert!(!root.join("later").exists());
}

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

#[test]
fn links_multiple_files_to_directories_and_preserves_failed_replacements() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("a"), b"a").unwrap();
    std::fs::write(root.join("b"), b"b").unwrap();
    std::fs::create_dir(root.join("links")).unwrap();
    success(&run(root, "ln", &["-vtlinks", "a", "b"]));
    assert!(same_file::is_same_file(root.join("a"), root.join("links/a")).unwrap());
    assert!(same_file::is_same_file(root.join("b"), root.join("links/b")).unwrap());
    assert_eq!(
        run(root, "ln", &["-f", "missing", "links/a"]).status.code(),
        Some(1)
    );
    assert_eq!(std::fs::read(root.join("links/a")).unwrap(), b"a");
    assert_eq!(run(root, "ln", &["-f", "a", "a"]).status.code(), Some(1));
    assert_eq!(std::fs::read(root.join("a")).unwrap(), b"a");
    success(&run(root, "ln", &["-f", "b", "links/a"]));
    assert_eq!(std::fs::read(root.join("links/a")).unwrap(), b"b");
    assert_eq!(
        run(root, "ln", &["-bT", "a", "links"]).status.code(),
        Some(1)
    );
    assert!(root.join("links").is_dir());
}

#[test]
fn backs_up_destinations_and_honors_interactive_answers() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"new").unwrap();
    std::fs::write(root.join("target"), b"old").unwrap();
    success(&run(root, "ln", &["-S.bak", "source", "target"]));
    assert_eq!(std::fs::read(root.join("target.bak")).unwrap(), b"old");
    for expected in ["target.~1~", "target.~2~"] {
        success(&run(root, "ln", &["--backup=numbered", "source", "target"]));
        assert_eq!(std::fs::read(root.join(expected)).unwrap(), b"new");
    }
    std::fs::write(root.join("independent"), b"keep").unwrap();
    for (answer, expected) in [("n\n", b"keep".as_slice()), ("y\n", b"new".as_slice())] {
        let mut child = command(root, "ln", &["--interactive", "source", "independent"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        success(&child.wait_with_output().unwrap());
        assert_eq!(std::fs::read(root.join("independent")).unwrap(), expected);
    }
}

#[test]
fn provides_help_versions_invalid_options_and_isolated_progress() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for utility in ["link", "ln", "unlink", "rmdir", "truncate"] {
        let output = run(root, utility, &["--help"]);
        assert!(output
            .stdout
            .starts_with(format!("Usage: gutil {utility} ").as_bytes()));
        success(&output);
        let output = run(root, utility, &["--version"]);
        assert_eq!(
            output.stdout,
            format!("gutil {utility} {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
        success(&output);
        for args in [
            &["--unknown"][..],
            &["-Z"],
            &["--jobs=0"],
            &["--verbose=bad"],
        ] {
            assert_eq!(run(root, utility, args).status.code(), Some(1));
        }
    }
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(root)
        .args([
            "--no-config",
            "--progress=always",
            "truncate",
            "-s1",
            "data",
        ])
        .output()
        .unwrap();
    assert_eq!(output.stdout, b"");
    assert!(String::from_utf8_lossy(&output.stderr).contains("operands"));
    success(&output);
}

#[cfg(unix)]
#[test]
fn preserves_symbolic_link_text_and_uses_relative_destinations() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("source")).unwrap();
    std::fs::create_dir_all(root.join("destination")).unwrap();
    std::fs::write(root.join("source/file"), b"data").unwrap();
    success(&run(
        root,
        "ln",
        &["-sr", "source/file", "destination/relative"],
    ));
    assert_eq!(
        std::fs::read_link(root.join("destination/relative")).unwrap(),
        Path::new("../source/file")
    );
    success(&run(root, "ln", &["-s", "missing", "dangling"]));
    success(&run(root, "ln", &["-P", "dangling", "physical"]));
    assert_eq!(
        std::fs::read_link(root.join("physical")).unwrap(),
        Path::new("missing")
    );
    success(&run(root, "ln", &["-L", "destination/relative", "logical"]));
    assert!(same_file::is_same_file(root.join("logical"), root.join("source/file")).unwrap());
    success(&run(root, "ln", &["-s", "destination", "dir-link"]));
    success(&run(root, "ln", &["-sfn", "source", "dir-link"]));
    assert_eq!(
        std::fs::read_link(root.join("dir-link")).unwrap(),
        Path::new("source")
    );
    assert_eq!(run(root, "rmdir", &["dir-link"]).status.code(), Some(1));
    success(&run(root, "unlink", &["dir-link"]));
    assert!(root.join("source").is_dir());
}

#[cfg(unix)]
#[test]
fn handles_native_names_io_block_units_and_literal_parent_components() {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let native = std::ffi::OsStr::from_bytes(b"native-\xff");
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
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    assert_eq!(run(root, "rmdir", &["-p", "a/./b"]).status.code(), Some(1));
    assert!(root.join("a").is_dir());
    std::fs::create_dir_all(root.join("space /b")).unwrap();
    success(&run(root, "rmdir", &["-p", "space /b"]));
    assert!(!root.join("space ").exists());
}

#[cfg(windows)]
#[test]
fn creates_windows_symbolic_links_or_explains_developer_mode() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"data").unwrap();
    let output = run(root, "ln", &["-s", "source", "symbolic"]);
    if output.status.success() {
        assert_eq!(std::fs::read(root.join("symbolic")).unwrap(), b"data");
    } else {
        assert!(String::from_utf8_lossy(&output.stderr).contains("Windows Developer Mode"));
        assert!(!root.join("symbolic").exists());
    }
}

#[test]
fn long_options_validate_link_plans_and_protect_backups() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (name, bytes) in [
        ("source", b"new".as_slice()),
        ("target", b"old"),
        ("target~", b"backup-source"),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    std::fs::create_dir(root.join("directory")).unwrap();
    success(&run(
        root,
        "ln",
        &[
            "--force",
            "--directory",
            "--verbose",
            "--target-directory=directory",
            "source",
        ],
    ));
    assert_eq!(
        std::fs::read(root.join("directory/source")).unwrap(),
        b"new"
    );
    for args in [
        &["--relative", "source", "new"][..],
        &["-t", "directory", "-T", "source"],
        &["--target-directory=missing", "source"],
        &["-T", "source"],
        &["source", "target", "missing"],
        &["source", "target"],
        &["source"],
        &["--backup=invalid", "source", "target"],
        &["--backup=off", "source", "target"],
        &["--backup=simple", "--suffix=", "source", "target"],
        &["-b", "target~", "target"],
    ] {
        assert_eq!(run(root, "ln", args).status.code(), Some(1), "{args:?}");
    }
    assert_eq!(std::fs::read(root.join("target")).unwrap(), b"old");
    success(&run(
        root,
        "ln",
        &["--backup=simple", "--suffix=.save", "source", "target"],
    ));
    assert_eq!(std::fs::read(root.join("target.save")).unwrap(), b"old");
    std::fs::write(root.join("target.~junk~"), b"ignored").unwrap();
    std::fs::write(root.join("target.~7~"), b"seven").unwrap();
    success(&run(root, "ln", &["--backup=existing", "source", "target"]));
    assert_eq!(std::fs::read(root.join("target.~8~")).unwrap(), b"new");
    std::fs::write(root.join("target.~18446744073709551615~"), b"max").unwrap();
    assert_eq!(
        run(root, "ln", &["--backup=numbered", "source", "target"])
            .status
            .code(),
        Some(1)
    );
    let output = command(root, "ln", &["--backup", "source", "directory/source"])
        .env("VERSION_CONTROL", "simple")
        .env("SIMPLE_BACKUP_SUFFIX", ".environment")
        .output()
        .unwrap();
    success(&output);
    assert_eq!(
        std::fs::read(root.join("directory/source.environment")).unwrap(),
        b"new"
    );
    std::fs::create_dir(root.join("inferred")).unwrap();
    success(&run(root, "ln", &["source", "inferred"]));
}

#[test]
fn reports_progress_for_each_mutation_and_rejects_size_overflow() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"data").unwrap();
    std::fs::write(root.join("-native"), b"native").unwrap();
    std::fs::create_dir_all(root.join("parent/child")).unwrap();
    success(&run(root, "link", &["--", "-native", "native-link"]));
    success(&run(
        root,
        "rmdir",
        &["--parents", "--verbose", "parent/child"],
    ));
    success(&run(
        root,
        "truncate",
        &["--no-create", "--size=1KB", "missing"],
    ));
    for args in [
        &["-s+K", "source"][..],
        &["-s1Kx", "source"],
        &["-s+9223372036854775807", "source"],
        &["-os9223372036854775807", "source"],
    ] {
        assert_eq!(
            run(root, "truncate", args).status.code(),
            Some(1),
            "{args:?}"
        );
        assert_eq!(std::fs::read(root.join("source")).unwrap(), b"data");
    }
    std::fs::create_dir(root.join("empty")).unwrap();
    for (utility, args) in [
        ("link", &["source", "progress-link"][..]),
        ("ln", &["source", "progress-ln"]),
        ("unlink", &["progress-link"]),
        ("rmdir", &["empty"]),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .current_dir(root)
            .args(["--no-config", "--progress=always", utility])
            .args(args)
            .output()
            .unwrap();
        success(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("operands"));
    }
}

#[cfg(unix)]
#[test]
fn long_symbolic_options_preserve_missing_targets_and_logical_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"data").unwrap();
    std::fs::write(root.join("target"), b"old").unwrap();
    success(&run(
        root,
        "ln",
        &[
            "--symbolic",
            "--relative",
            "--force",
            "--no-dereference",
            "--no-target-directory",
            "--verbose",
            "source",
            "target",
        ],
    ));
    assert_eq!(
        std::fs::read_link(root.join("target")).unwrap(),
        Path::new("source")
    );
    assert_eq!(
        run(root, "ln", &["--force", "--logical", "target", "source"])
            .status
            .code(),
        Some(1)
    );
    success(&run(root, "ln", &["--physical", "target", "physical"]));
    assert_eq!(
        std::fs::read_link(root.join("physical")).unwrap(),
        Path::new("source")
    );
    success(&run(root, "ln", &["-sf", "missing/target", "target"]));
    assert_eq!(
        std::fs::read_link(root.join("target")).unwrap(),
        Path::new("missing/target")
    );
}

#[test]
fn resolves_empty_backup_controls_and_suffix_policies_from_the_environment() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"new").unwrap();
    for (target, flag) in [("first", "--backup="), ("second", "--suffix=.save")] {
        std::fs::write(root.join(target), b"old").unwrap();
        let output = command(root, "ln", &[flag, "source", target])
            .env("VERSION_CONTROL", "numbered")
            .output()
            .unwrap();
        success(&output);
        assert_eq!(
            std::fs::read(root.join(format!("{target}.~1~"))).unwrap(),
            b"old"
        );
    }
    std::fs::write(root.join("protected"), b"keep").unwrap();
    let failure = command(root, "ln", &["-S.save", "source", "protected"])
        .env("VERSION_CONTROL", "invalid")
        .output()
        .unwrap();
    assert_eq!(failure.status.code(), Some(1));
    assert_eq!(std::fs::read(root.join("protected")).unwrap(), b"keep");
    assert_eq!(
        run(
            root,
            "ln",
            &["--backup=off", "-S.save", "source", "protected"]
        )
        .status
        .code(),
        Some(1)
    );
}

#[cfg(windows)]
#[test]
fn rejects_unavailable_io_block_units_before_creating_a_file() {
    let fixture = tempfile::tempdir().unwrap();
    let output = run(
        fixture.path(),
        "truncate",
        &["--io-blocks", "--size=1", "missing"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(!fixture.path().join("missing").exists());
}
