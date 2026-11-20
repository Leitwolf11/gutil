#![allow(clippy::unwrap_used)]

use std::process::{Command, Output};

fn gutil(utility: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gutil"))
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .args(["--no-config", "--color=never", utility])
        .args(args)
        .output()
        .unwrap()
}

fn success(utility: &str, args: &[&str]) -> Vec<u8> {
    let result = gutil(utility, args);
    assert!(result.status.success(), "{result:?}");
    assert!(result.stderr.is_empty(), "{result:?}");
    result.stdout
}

#[test]
fn selects_fields_in_canonical_order_and_removes_duplicates() {
    assert_eq!(success("uname", &[]), success("uname", &["-s"]));
    assert_eq!(success("arch", &[]), success("uname", &["--machine"]));

    let mut expected = success("uname", &["-s"]);
    assert_eq!(expected.pop(), Some(b'\n'));
    expected.push(b' ');
    expected.extend(success("uname", &["-m"]));
    assert_eq!(success("uname", &["-m", "-ssm"]), expected);
    assert_eq!(success("uname", &["--mach", "--kernel-name"]), expected);
}

#[test]
fn prints_optional_fields_and_labeled_output() {
    assert_eq!(success("uname", &["-ip"]), b"unknown unknown\n");
    let plain = success("uname", &["-ai"]);
    let labeled = success("uname", &["-Ap", "-a"]);
    assert_eq!(success("uname", &["--all-labeled"]), labeled);
    let mut values = Vec::new();
    let mut lines = labeled.split_inclusive(|&byte| byte == b'\n');
    for label in [
        "Kernel name: ",
        "Node name: ",
        "Kernel release: ",
        "Kernel version: ",
        "Machine: ",
        "Operating system: ",
    ] {
        let value = lines
            .next()
            .unwrap()
            .strip_prefix(label.as_bytes())
            .unwrap();
        assert!(value.ends_with(b"\n"));
        values.push(&value[..value.len() - 1]);
    }
    assert!(lines.next().is_none());
    let mut flattened = values.join(&b' ');
    flattened.push(b'\n');
    assert_eq!(flattened, plain);
}

#[test]
fn handles_help_version_and_invalid_arguments() {
    for utility in ["uname", "arch"] {
        assert!(
            success(utility, &["--he"]).starts_with(format!("Usage: gutil {utility} ").as_bytes())
        );
        assert_eq!(
            success(utility, &["--version"]),
            format!("gutil {utility} {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
        for args in [&["-z"][..], &["--unknown"], &["extra"], &["--", "--help"]] {
            let output = gutil(utility, args);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert!(!output.stderr.is_empty(), "{output:?}");
        }
    }
    assert_eq!(gutil("arch", &["-m"]).status.code(), Some(1));
    assert_eq!(gutil("uname", &["--a"]).status.code(), Some(1));
}

#[cfg(target_os = "linux")]
#[test]
fn matches_gnu_uname_in_the_c_locale() {
    let Ok(directory) = std::env::var("GNU_COREUTILS_DIR") else {
        return;
    };
    let reference = std::path::Path::new(&directory).join("uname");
    for args in [
        &[][..],
        &["-a"],
        &["-A"],
        &["-srvo"],
        &["-m"],
        &["-i"],
        &["-p"],
        &["-n"],
        &["--kernel-release"],
        &["-aA"],
        &["--", "extra"],
        &["--a"],
    ] {
        let output = Command::new(&reference)
            .env("LC_ALL", "C")
            .env_remove("POSIXLY_CORRECT")
            .args(args)
            .output()
            .unwrap();
        let actual = gutil("uname", args);
        assert_eq!(output.status.code(), actual.status.code(), "{args:?}");
        assert_eq!(output.stdout, actual.stdout, "{args:?}");
    }
}
