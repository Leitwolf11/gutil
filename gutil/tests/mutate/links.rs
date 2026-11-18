use super::{command, run, success};
use std::io::Write;
#[cfg(unix)]
use std::path::Path;
use std::process::Stdio;

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

#[cfg(windows)]
#[test]
fn creates_windows_symbolic_links_or_explains_developer_mode() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"data").unwrap();
    let output = run(root, "ln", &["-s", "source", "symbolic"]);
    if output.status.success() {
        assert_eq!(std::fs::read(root.join("symbolic")).unwrap(), b"data");
        std::fs::create_dir(root.join("first-directory")).unwrap();
        std::fs::create_dir(root.join("second-directory")).unwrap();
        std::fs::write(root.join("first-directory/keep"), b"keep").unwrap();
        success(&run(
            root,
            "ln",
            &["-s", "first-directory", "directory-link"],
        ));
        success(&run(
            root,
            "ln",
            &["-sfn", "second-directory", "directory-link"],
        ));
        success(&run(root, "ln", &["-sfn", "source", "directory-link"]));
        assert_eq!(std::fs::read(root.join("directory-link")).unwrap(), b"data");
        assert_eq!(
            std::fs::read(root.join("first-directory/keep")).unwrap(),
            b"keep"
        );
        assert!(root.join("second-directory").is_dir());
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

#[test]
fn protects_just_created_hard_links_and_allows_numbered_replacements() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for directory in ["first", "second", "out", "numbered"] {
        std::fs::create_dir(root.join(directory)).unwrap();
    }
    std::fs::write(root.join("first/item"), b"first").unwrap();
    std::fs::write(root.join("second/item"), b"second").unwrap();
    std::fs::write(root.join("first/other"), b"other").unwrap();
    let output = run(
        root,
        "ln",
        &["-ft", "out", "first/item", "second/item", "first/other"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(std::fs::read(root.join("out/item")).unwrap(), b"first");
    assert_eq!(std::fs::read(root.join("out/other")).unwrap(), b"other");
    success(&run(
        root,
        "ln",
        &[
            "--backup=numbered",
            "-ft",
            "numbered",
            "first/item",
            "second/item",
        ],
    ));
    assert_eq!(
        std::fs::read(root.join("numbered/item")).unwrap(),
        b"second"
    );
    assert_eq!(
        std::fs::read(root.join("numbered/item.~1~")).unwrap(),
        b"first"
    );
    std::fs::hard_link(root.join("first/item"), root.join("alias")).unwrap();
    success(&run(root, "ln", &["-f", "first/item", "alias"]));
    assert_eq!(std::fs::read(root.join("alias")).unwrap(), b"first");
}

#[cfg(unix)]
#[test]
fn preserves_single_link_sources_reached_through_symbolic_aliases() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"data").unwrap();
    std::os::unix::fs::symlink("source", root.join("alias")).unwrap();
    assert_eq!(
        run(root, "ln", &["-sf", "alias", "source"]).status.code(),
        Some(1)
    );
    assert_eq!(std::fs::read(root.join("source")).unwrap(), b"data");
    success(&run(root, "ln", &["-sf", "alias", "alias"]));
    assert_eq!(
        std::fs::read_link(root.join("alias")).unwrap(),
        Path::new("alias")
    );
    std::fs::create_dir(root.join("out")).unwrap();
    let output = run(root, "ln", &["-srv", "source", "out/relative"]);
    success(&output);
    assert!(String::from_utf8_lossy(&output.stdout).contains("../source"));
    let repeated = run(root, "ln", &["-Pft", "out", "alias", "alias"]);
    assert_eq!(repeated.status.code(), Some(1));
    assert_eq!(
        std::fs::read_link(root.join("out/alias")).unwrap(),
        Path::new("alias")
    );
}

#[test]
fn retains_the_destination_when_a_backup_cannot_be_created() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    std::fs::write(root.join("source"), b"new").unwrap();
    std::fs::write(root.join("target"), b"keep").unwrap();
    let failure = run(
        root,
        "ln",
        &["--suffix=.missing/backup", "source", "target"],
    );
    assert_eq!(failure.status.code(), Some(1));
    assert_eq!(std::fs::read(root.join("target")).unwrap(), b"keep");
    assert_eq!(std::fs::read(root.join("source")).unwrap(), b"new");
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 2);
}
