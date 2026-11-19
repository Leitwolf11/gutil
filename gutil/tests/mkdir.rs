#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn invocation(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command.current_dir(root).env("LC_ALL", "C").args([
        "--no-config",
        "--color=never",
        "--progress=never",
        "--jobs=1",
        "mkdir",
    ]);
    command
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn preserves_operand_order_and_continues_after_failure() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let output = invocation(root)
        .args(["-v", "first/child", "first", "after"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("first").is_dir());
    assert!(!root.join("first/child").exists());
    assert!(root.join("after").is_dir());
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 2);
    let output = invocation(root).args(["first", "last"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("last").is_dir());
}

#[test]
fn parses_clustered_options_worker_aliases_and_literal_names() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(
        &invocation(root)
            .args(["-pvj2", "deep/leaf", "--threads=1", "--", "-m", "-"])
            .output()
            .unwrap(),
    );
    for name in ["deep/leaf", "-m", "-"] {
        assert!(root.join(name).is_dir());
    }
    success(
        &invocation(root)
            .args(["--parents", "--jobs", "2", "deep/leaf"])
            .output()
            .unwrap(),
    );
    fs::write(root.join("file"), b"keep").unwrap();
    let output = invocation(root)
        .args(["-p", "file/child", "survivor"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(root.join("survivor").is_dir());
    assert_eq!(fs::read(root.join("file")).unwrap(), b"keep");
}

#[test]
fn rejects_invalid_requests_before_mutating_the_filesystem() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for args in [
        vec![],
        vec!["new", "-m"],
        vec!["new", "--mode=888"],
        vec!["new", "--mode=u+gg"],
        vec!["new", "--unknown"],
        vec!["new", "-j0"],
        vec!["new", "-q"],
        vec!["new", "--mode="],
        vec!["new", "-m", "u=rw,"],
    ] {
        let output = invocation(root).args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(!root.join("new").exists());
    }
    let output = invocation(root).args(["-p", ""]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn help_and_version_do_not_require_configuration() {
    for option in ["--help", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .args(["--config", "missing-config.toml", "mkdir", option])
            .output()
            .unwrap();
        success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("gutil mkdir"));
    }
}

#[test]
fn rich_progress_is_separate_from_verbose_creation_messages() {
    let fixture = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .current_dir(fixture.path())
        .args([
            "--no-config",
            "--color=never",
            "--progress=always",
            "mkdir",
            "-pv",
            "parent/leaf",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(String::from_utf8_lossy(&output.stdout).lines().count(), 2);
    assert!(String::from_utf8_lossy(&output.stderr).contains("directories"));
}

#[test]
fn context_requests_never_silently_claim_active_label_assignment() {
    for context in ["-Z", "--context", "--context=example"] {
        let fixture = tempfile::tempdir().unwrap();
        let output = invocation(fixture.path())
            .args([context, "created"])
            .output()
            .unwrap();
        if output.status.success() {
            assert!(fixture.path().join("created").is_dir());
            if context.contains('=') {
                assert!(String::from_utf8_lossy(&output.stderr).contains("ignoring context"));
            }
        } else {
            assert_eq!(output.status.code(), Some(1));
            assert!(!fixture.path().join("created").exists());
            assert!(String::from_utf8_lossy(&output.stderr).contains("not implemented"));
        }
    }
}

#[cfg(unix)]
fn masked(root: &Path, mask: &str, arguments: &[&str]) -> Output {
    Command::new("sh")
        .current_dir(root)
        .env("LC_ALL", "C")
        .args([
            "-c",
            "umask \"$1\"; shift; exec \"$@\"",
            "mkdir-fixture",
            mask,
            env!("CARGO_BIN_EXE_gutil"),
            "--no-config",
            "--progress=never",
            "--color=never",
            "--jobs=1",
            "mkdir",
        ])
        .args(arguments)
        .output()
        .unwrap()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
#[test]
fn applies_leaf_modes_and_preserves_existing_directory_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(&masked(root, "027", &["-p", "-m", "000", "parent/child"]));
    assert_eq!(mode(&root.join("parent")), 0o750);
    assert_eq!(mode(&root.join("parent/child")), 0);
    fs::set_permissions(root.join("parent"), fs::Permissions::from_mode(0o711)).unwrap();
    symlink("parent", root.join("alias")).unwrap();
    success(&masked(
        root,
        "0777",
        &["-p", "-m700", "parent", "alias", "new/leaf"],
    ));
    assert_eq!(mode(&root.join("parent")), 0o711);
    assert_eq!(mode(&root.join("new")), 0o300);
    assert_eq!(mode(&root.join("new/leaf")), 0o700);
}

#[cfg(unix)]
#[test]
fn trailing_dot_and_parent_components_keep_ancestor_modes() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    success(&masked(
        root,
        "022",
        &["-p", "-m000", "a/.", "b/../c", "d//./e/"],
    ));
    for path in ["a", "b", "d"] {
        assert_eq!(mode(&root.join(path)), 0o755);
    }
    for path in ["c", "d/e"] {
        assert_eq!(mode(&root.join(path)), 0);
    }
}

#[cfg(unix)]
#[test]
fn compares_permission_modes_with_gnu_across_masks_and_setgid_inheritance() {
    use std::os::unix::fs::PermissionsExt;
    let Some(directory) = std::env::var_os("GNU_COREUTILS_DIR") else {
        return;
    };
    let gnu = Path::new(&directory).join("mkdir");
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let mut index = 0;
    for inherited in [false, true] {
        for mask in ["000", "022", "027", "0777"] {
            for specification in GNU_MODES {
                index += 1;
                let base = root.join(index.to_string());
                fs::create_dir(&base).unwrap();
                fs::set_permissions(
                    &base,
                    fs::Permissions::from_mode(if inherited { 0o2770 } else { 0o770 }),
                )
                .unwrap();
                let actual = masked(&base, mask, &["-m", specification, "actual"]);
                let expected = Command::new("sh")
                    .current_dir(&base)
                    .env("LC_ALL", "C")
                    .args([
                        "-c",
                        "umask \"$1\"; shift; exec \"$@\"",
                        "mkdir-fixture",
                        mask,
                    ])
                    .arg(&gnu)
                    .args(["-m", specification, "expected"])
                    .output()
                    .unwrap();
                assert_eq!(
                    actual.status.success(),
                    expected.status.success(),
                    "{specification:?}, mask {mask}, inherited {inherited}"
                );
                if expected.status.success() {
                    assert_eq!(
                        mode(&base.join("actual")),
                        mode(&base.join("expected")),
                        "{specification:?}, mask {mask}, inherited {inherited}"
                    );
                } else {
                    assert!(!base.join("actual").exists());
                }
            }
        }
    }
    assert_eq!(index, 408);
}

#[cfg(unix)]
const GNU_MODES: [&str; 51] = [
    "755",
    "00755",
    "0",
    "00000",
    "07777",
    "+w",
    "-w",
    "=rw",
    "=0,u+r",
    "u+r-w+x",
    "g=u",
    "a=rwX",
    "u+t",
    "o+t",
    "u+s,g-s",
    "a=s",
    "g=",
    "u+s",
    "g+s",
    "a+s",
    "g-s",
    "a+t,u+r",
    "u+s-s",
    "o+t-t",
    "+0",
    "-0",
    "+1000",
    "-1000",
    "+2000",
    "-2000",
    "+4000",
    "+1",
    "-1",
    "u+X",
    "a=s-s",
    "a+r,a+t",
    "a=rwx,o+t",
    "a+rwXt",
    "u=rw,g=u,o=g",
    "u+rwx,g-w,o=",
    "u+",
    "+",
    "u+gg",
    "u+rxg",
    "777,u+r",
    "+1-r",
    "",
    ",",
    "u=rw,",
    "888",
    "10000",
];

#[cfg(target_os = "linux")]
#[test]
fn symbolic_and_operator_modes_preserve_documented_creation_semantics() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    for (index, (specification, expected)) in [
        ("0644", 0o644),
        ("00755", 0o755),
        ("u=rw,g=u,o=g", 0o666),
        ("a+t", 0o1755),
        ("u+s", 0o4777),
        ("g+s", 0o2777),
        ("u+s,g-s", 0o4777),
        ("a=rwx,o+t", 0o1777),
        ("u+r-w+x", 0o577),
        ("+1000", 0o1755),
        ("-2000", 0o755),
        ("=0,u+r", 0o400),
        ("g=u", 0o777),
        ("a+g", 0o777),
        ("u=o", 0o777),
        ("a=rwX", 0o777),
        ("-w", 0o577),
        ("=rw", 0o644),
        ("o=s", 0o770),
        ("u+t", 0o777),
        ("u+", 0o777),
    ]
    .into_iter()
    .enumerate()
    {
        let name = index.to_string();
        success(&masked(root, "022", &["-m", specification, &name]));
        assert_eq!(mode(&root.join(name)), expected, "{specification}");
    }
}
