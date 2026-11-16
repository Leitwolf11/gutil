#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::tempdir;

fn gutil(dir: &Path) -> Command {
    let mut process = Command::new(env!("CARGO_BIN_EXE_gutil"));
    process
        .current_dir(dir)
        .env("HOME", dir)
        .env("USERPROFILE", dir);
    process
}

fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    success(&output);
}

#[test]
fn validates_syntax_and_reports_uninstalled_commands() {
    let home = tempdir().unwrap();
    let dir = home.path();
    for args in [
        &["plug-in"][..],
        &["plug-in", "install"],
        &["plug-in", "install", "https://github.com/owner/repo"],
        &["plug-in", "install", "github.com:owner/repo/extra"],
        &["plug-in", "install", "github.com:owner/.."],
        &["plug-in", "use", "../command"],
        &["plug-in", "use", "MixedCase"],
    ] {
        let result = gutil(dir).args(args).output().unwrap();
        assert_eq!(result.status.code(), Some(2));
        assert!(result.stderr.starts_with(b"gutil plug-in:"));
    }
    let result = gutil(dir)
        .args(["plug-in", "use", "missing"])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("is not installed"));
    let result = gutil(dir)
        .args(["plug-in", "use", "missing"])
        .env_remove("HOME")
        .env_remove("USERPROFILE")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("absolute home directory"));
    fs::write(dir.join("gutil.toml"), "[").unwrap();
    let result = gutil(dir).args(["plug-in", "--help"]).output().unwrap();
    success(&result);
    assert!(String::from_utf8_lossy(&result.stdout).contains("plug-in use"));
    let result = gutil(dir).args(["plug-in", "--version"]).output().unwrap();
    success(&result);
}

fn create_fixture(repo: &Path) {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/gutil-plugin-sdk");
    fs::write(
        repo.join("Cargo.toml"),
        format!(
            "[package]\nname = 'gutil-plugin-command'\nversion = '0.1.0'\nedition = '2021'\n[dependencies]\ngutil-plugin-sdk = {{ path = {} }}\n",
            serde_json::to_string(&sdk).unwrap()
        ),
    )
    .unwrap();
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/main.rs"),
        r#"
fn main() -> Result<std::process::ExitCode, Box<dyn std::error::Error>> {
    let invocation = gutil_plugin_sdk::Invocation::from_env()?;
    let args: Vec<_> = invocation.into_args().collect();
    if args.first().is_some_and(|arg| arg == "--exit") {
        return Ok(std::process::ExitCode::from(17));
    }
    if args.first().is_some_and(|arg| arg == "--raw") {
        use std::io::Write;
        std::io::stdout().write_all(args[1].as_encoded_bytes())?;
        return Ok(std::process::ExitCode::SUCCESS);
    }
    for argument in args {
        println!("{}", argument.to_string_lossy());
    }
    Ok(std::process::ExitCode::SUCCESS)
}
"#,
    )
    .unwrap();
    success(
        &Command::new("cargo")
            .current_dir(repo)
            .args(["generate-lockfile", "--offline"])
            .output()
            .unwrap(),
    );
    git(repo, &["init", "-q"]);
    git(repo, &["add", "."]);
    git(
        repo,
        &[
            "-c",
            "user.name=gutil test",
            "-c",
            "user.email=gutil@example.test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Add the plug-in fixture",
        ],
    );
}

#[test]
fn installs_and_launches_a_rust_plug_in() {
    let fixture = tempdir().unwrap();
    let home = tempdir().unwrap();
    let dir = home.path();
    let repo = fixture.path();
    create_fixture(repo);
    let url = reqwest::Url::from_directory_path(repo).unwrap();
    let install = || {
        gutil(dir)
            .args([
                "plug-in",
                "install",
                "github.com:fixture-owner/fixture-repo",
            ])
            .env("CARGO_NET_GIT_FETCH_WITH_CLI", "true")
            .env("CARGO_TARGET_DIR", dir.join("build"))
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", format!("url.{url}.insteadOf"))
            .env(
                "GIT_CONFIG_VALUE_0",
                "https://github.com/fixture-owner/fixture-repo",
            )
            .output()
            .unwrap()
    };
    let output = install();
    success(&output);
    assert_eq!(output.stdout, b"Installed plugin-command\n");
    let executable = dir.join(".gutil/plugins").join(format!(
        "gutil-plugin-command{}",
        std::env::consts::EXE_SUFFIX
    ));
    for protocol in [None, Some("999")] {
        let mut direct = Command::new(&executable);
        if let Some(version) = protocol {
            direct.env("GUTIL_PLUGIN_PROTOCOL", version);
        } else {
            direct.env_remove("GUTIL_PLUGIN_PROTOCOL");
        }
        let output = direct.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let expected = if protocol.is_some() {
            "Unsupported"
        } else {
            "Missing"
        };
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }

    let output = gutil(dir)
        .args([
            "plug-in",
            "use",
            "plugin-command",
            "--example-flag",
            "a b",
            "--",
            "--help",
        ])
        .output()
        .unwrap();
    success(&output);
    assert_eq!(output.stdout, b"--example-flag\na b\n--\n--help\n");

    let output = gutil(dir)
        .args(["plug-in", "use", "plugin-command", "--exit"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(17));

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let argument = b"native-\xff";
        let output = gutil(dir)
            .args(["plug-in", "use", "plugin-command", "--raw"])
            .arg(std::ffi::OsStr::from_bytes(argument))
            .output()
            .unwrap();
        success(&output);
        assert_eq!(output.stdout, argument);
    }

    let output = install();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already installed"));
}
