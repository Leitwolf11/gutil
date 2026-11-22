#![allow(clippy::unwrap_used)]

use std::process::{Command, Output, Stdio};

fn gutil() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .args(["--no-config", "--color=never", "tty"])
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT")
        .stdin(Stdio::null());
    command
}

fn expect(output: &Output, status: i32, stdout: &[u8]) {
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert_eq!(output.stdout, stdout);
}

#[test]
fn identifies_nonterminal_input_and_honors_all_quiet_spellings() {
    expect(&gutil().output().unwrap(), 1, b"not a tty\n");
    for option in ["-s", "-ss", "--silent", "--quiet", "--sil", "--q"] {
        let output = gutil().arg(option).output().unwrap();
        expect(&output, 1, b"");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let file = tempfile::tempfile().unwrap();
    expect(&gutil().stdin(file).output().unwrap(), 1, b"not a tty\n");
    expect(
        &gutil().stdin(Stdio::piped()).output().unwrap(),
        1,
        b"not a tty\n",
    );
}

#[test]
fn distinguishes_usage_errors_and_information_options() {
    for args in [
        &["-q"][..],
        &["--unknown"],
        &["--silent=yes"],
        &["extra"],
        &["-"],
        &["--", "--help"],
        &["-s", "extra"],
    ] {
        let output = gutil().args(args).output().unwrap();
        expect(&output, 2, b"");
        assert!(!output.stderr.is_empty(), "{output:?}");
    }
    let help = gutil().args(["operand", "--he"]).output().unwrap();
    assert_eq!(help.status.code(), Some(0));
    assert!(help.stdout.starts_with(b"Usage: gutil tty "));
    let posix = gutil()
        .env("POSIXLY_CORRECT", "1")
        .args(["operand", "--help"])
        .output()
        .unwrap();
    expect(&posix, 2, b"");
    let version = format!("gutil tty {}\n", env!("CARGO_PKG_VERSION"));
    expect(
        &gutil().arg("--ver").output().unwrap(),
        0,
        version.as_bytes(),
    );
}

#[test]
fn gives_output_errors_a_distinct_status() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let output = gutil().stdout(writer).output().unwrap();
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(!output.stderr.is_empty(), "{output:?}");
}

#[cfg(all(unix, not(target_os = "fuchsia")))]
#[test]
fn reports_the_actual_pty_device_name_and_quiet_success() {
    let pty = nix::pty::openpty(None, None).unwrap();
    let path = nix::unistd::ttyname(&pty.slave).unwrap();
    let mut expected = path.as_os_str().as_encoded_bytes().to_vec();
    expected.push(b'\n');
    let output = gutil()
        .stdin(pty.slave.try_clone().unwrap())
        .output()
        .unwrap();
    expect(&output, 0, &expected);
    let output = gutil().stdin(pty.slave).arg("-s").output().unwrap();
    expect(&output, 0, b"");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[cfg(all(unix, not(target_os = "fuchsia")))]
#[test]
fn matches_gnu_for_terminal_and_redirected_stdin() {
    let Ok(directory) = std::env::var("GNU_COREUTILS_DIR") else {
        return;
    };
    let reference = std::path::Path::new(&directory).join("tty");
    for args in [
        &[][..],
        &["--quiet"],
        &["--si"],
        &["-ss"],
        &["--"],
        &["extra"],
        &["-q"],
        &["--", "-s"],
    ] {
        let pty = nix::pty::openpty(None, None).unwrap();
        for terminal in [false, true] {
            let input = || {
                if terminal {
                    Stdio::from(pty.slave.try_clone().unwrap())
                } else {
                    Stdio::null()
                }
            };
            let expected = Command::new(&reference)
                .args(args)
                .env("LC_ALL", "C")
                .env_remove("POSIXLY_CORRECT")
                .stdin(input())
                .output()
                .unwrap();
            let actual = gutil().args(args).stdin(input()).output().unwrap();
            assert_eq!(actual.status.code(), expected.status.code(), "{args:?}");
            assert_eq!(actual.stdout, expected.stdout, "{args:?}");
        }
    }
}
