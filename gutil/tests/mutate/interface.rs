use super::{run, success};
use std::process::Command;

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
