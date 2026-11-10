#![allow(clippy::unwrap_used)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(directory: &Path, home: &Path, args: &[&str], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args(args)
        .current_dir(directory)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("NO_COLOR")
        .env_remove("POSIXLY_CORRECT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn uses_the_first_file_and_applies_only_the_selected_profile() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().join("project");
    let home = directory.path().join("home");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir_all(home.join(".gutil")).unwrap();
    fs::write(home.join(".gutil/config.toml"), "[cat]\nshow-ends = true\n").unwrap();

    let result = run(&cwd, &home, &["cat"], b"x\n");
    assert!(result.status.success());
    assert_eq!(result.stdout, b"x$\n");

    let project = "[cat]\nshow-tabs = true\n[profiles.review.cat]\nnumber = 'all'\n";
    fs::write(cwd.join("gutil.toml"), project).unwrap();
    let result = run(&cwd, &home, &["--profile", "review", "cat"], b"\tx\n");
    assert!(result.status.success());
    assert_eq!(result.stdout, b"     1\t^Ix\n");

    let result = run(
        &cwd,
        &home,
        &["--profile=review", "cat", "--number=none", "--no-show-tabs"],
        b"\tx\n",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"\tx\n");

    let result = run(&cwd, &home, &["--no-config", "cat"], b"\tx\n");
    assert!(result.status.success());
    assert_eq!(result.stdout, b"\tx\n");

    let result = run(
        &cwd,
        &home,
        &["--config", "../home/.gutil/config.toml", "cat"],
        b"x\n",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"x$\n");
}

#[test]
fn rejects_invalid_configurations_with_context() {
    let directory = tempfile::tempdir().unwrap();
    for (contents, expected) in [
        ("[global]\nmax_auto_jobs=2", "max_auto_jobs"),
        ("[global]\njobs=0", "global.jobs"),
        ("[global]\nmax-auto-jobs=513", "global.max-auto-jobs"),
        ("[output]\ncolor='sometimes'", "color"),
        ("[profiles.bad.ls]\nlayout='tree'", "profiles.bad"),
    ] {
        fs::write(directory.path().join("gutil.toml"), contents).unwrap();
        let result = run(directory.path(), directory.path(), &["pwd"], b"");
        assert_eq!(result.status.code(), Some(2));
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(
            error.contains("gutil.toml") && error.contains(expected),
            "{error}"
        );
    }
    fs::write(directory.path().join("gutil.toml"), "[global]\njobs='auto'").unwrap();
    let result = run(
        directory.path(),
        directory.path(),
        &["--profile", "missing", "pwd"],
        b"",
    );
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("unknown profile"));

    let result = run(
        directory.path(),
        directory.path(),
        &["--progress=maybe", "pwd"],
        b"",
    );
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("--progress expects"));
}

#[test]
fn sends_progress_and_json_diagnostics_only_to_stderr() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(
        cwd.join("gutil.toml"),
        "[global]\nprogress='always'\n[diagnostics]\nformat='json'\n",
    )
    .unwrap();

    let result = run(cwd, cwd, &["cat", "missing", "-"], b"binary\x00\xff");
    assert_eq!(result.status.code(), Some(1));
    assert_eq!(result.stdout, b"binary\x00\xff");
    let records: Vec<serde_json::Value> = String::from_utf8(result.stderr)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(records
        .iter()
        .any(|record| record["type"] == "diagnostic" && record["level"] == "error"));
    assert!(records.iter().any(|record| record["type"] == "progress"
        && record["unit"] == "bytes"
        && record["completed"] == 8
        && record["finished"] == true));

    let result = run(cwd, cwd, &["--progress=never", "cat"], b"x");
    assert_eq!(result.stdout, b"x");
    assert_eq!(result.stderr, b"");

    let result = run(cwd, cwd, &["pwd"], b"");
    assert!(result.status.success());
    assert_eq!(result.stderr, b"");

    let result = run(
        cwd,
        cwd,
        &["--no-config", "--progress=always", "ls", "-d", "."],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b".\n");
    let report = String::from_utf8(result.stderr).unwrap();
    assert!(report.contains("1 entries (done)"));
    assert!(!report.contains('\u{1b}'));
}

#[test]
fn renders_listing_profiles_with_rich_columns_and_hyperlinks() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::create_dir(cwd.join("files")).unwrap();
    fs::create_dir(cwd.join("files/z-dir")).unwrap();
    fs::write(cwd.join("files/a name"), b"abc").unwrap();
    fs::write(cwd.join("files/b.txt"), vec![0; 1024]).unwrap();
    fs::write(cwd.join("gutil.toml"), "[ls]\nsort='size'\ndirectories-first=true\n[profiles.grid.ls]\nlayout='columns'\nsort='name'\nquote='shell'\n").unwrap();
    let result = run(cwd, cwd, &["--profile", "grid", "ls", "files"], b"");
    assert!(result.status.success(), "{:?}", result.stderr);
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(output.lines().count(), 1, "{output:?}");
    assert!(output.starts_with("z-dir"));
    assert!(output.contains("'a name'"));
    assert!(output.ends_with("b.txt\n"));

    let result = run(
        cwd,
        cwd,
        &[
            "--color=always",
            "--hyperlinks=always",
            "ls",
            "--sort=name",
            "files",
        ],
        b"",
    );
    assert!(result.status.success());
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("\x1b["));
    assert!(output.contains("\x1b]8;;file:"));
    assert!(output.contains("a%20name"));
    assert!(output.contains("\x1b]8;;\x1b\\"));

    let result = run(
        cwd,
        cwd,
        &[
            "ls",
            "-l",
            "--size-style=human",
            "--owner-format=none",
            "--time-style=unix",
            "files/b.txt",
        ],
        b"",
    );
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("1.0 KB"));
}

#[test]
fn applies_verbosity_and_job_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join("gutil.toml"), "[global]\nmax-auto-jobs=1\n[diagnostics]\nverbosity='quiet'\n[profiles.debug.diagnostics]\nverbosity='verbose'\n").unwrap();
    let result = run(cwd, cwd, &["pwd", "ignored"], b"");
    assert!(result.status.success());
    assert_eq!(result.stderr, b"");

    let result = run(cwd, cwd, &["--profile=debug", "--jobs=3", "pwd"], b"");
    assert!(result.status.success());
    let diagnostic = String::from_utf8(result.stderr).unwrap();
    assert!(diagnostic.contains("gutil.toml; profile: debug; jobs: 3"));
}

#[test]
fn reports_a_known_file_size_and_keeps_automatic_progress_off_pipes() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join("file"), b"contents").unwrap();

    let automatic = run(cwd, cwd, &["cat", "file"], b"");
    assert!(automatic.status.success());
    assert_eq!(automatic.stdout, b"contents");
    assert_eq!(automatic.stderr, b"");

    let always = run(cwd, cwd, &["--progress=always", "cat", "file"], b"");
    assert!(always.status.success());
    assert_eq!(always.stdout, b"contents");
    assert!(String::from_utf8_lossy(&always.stderr).contains("8 bytes / 8 bytes (done)"));
}

#[cfg(unix)]
#[test]
fn uses_configured_pwd_mode_until_a_flag_overrides_it() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    let alias = directory.path().join("alias");
    fs::create_dir(&real).unwrap();
    symlink("real", &alias).unwrap();
    fs::write(real.join("gutil.toml"), "[pwd]\nmode='logical'\n").unwrap();
    let physical = fs::canonicalize(&real).unwrap();
    for (arguments, expected) in [
        (&["pwd"][..], alias.as_path()),
        (&["pwd", "-P"], physical.as_path()),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_gutil"))
            .args(arguments)
            .current_dir(&real)
            .env("PWD", &alias)
            .env_remove("POSIXLY_CORRECT")
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut path = expected.as_os_str().as_encoded_bytes().to_vec();
        path.push(b'\n');
        assert_eq!(result.stdout, path);
    }
}

#[test]
fn lets_ls_flags_override_profiles_and_long_format_settings() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::create_dir(cwd.join("files")).unwrap();
    for (name, size) in [("big.a", 1_000), ("small.z", 1), (".hidden", 2)] {
        fs::write(cwd.join("files").join(name), vec![0; size]).unwrap();
    }
    fs::write(cwd.join("gutil.toml"), "[ls]\nlayout='long'\nhidden='all'\nsort='size'\nreverse=true\n[ls.long]\nsize-style='si'\ntime-style='rfc3339'\ntimezone='local'\n").unwrap();
    let result = run(
        cwd,
        cwd,
        &["ls", "-1", "--hidden=omit", "--no-reverse", "files"],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"big.a\nsmall.z\n");

    let result = run(cwd, cwd, &["ls", "-l", "files/big.a"], b"");
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("1.0 kB"));
    assert!(String::from_utf8_lossy(&result.stdout).contains('T'));
}
