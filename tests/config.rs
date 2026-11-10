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
    assert!(String::from_utf8_lossy(&result.stdout).contains("1.0KB"));
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
    assert!(String::from_utf8_lossy(&result.stdout).contains("1.0kB"));
    assert!(String::from_utf8_lossy(&result.stdout).contains('T'));
}

#[test]
fn uses_listing_sort_modes_and_reports_operand_errors() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::create_dir(cwd.join("files")).unwrap();
    fs::create_dir(cwd.join("files/z-dir")).unwrap();
    for name in ["a.z", "z.a"] {
        fs::write(cwd.join("files").join(name), []).unwrap();
    }
    let result = run(
        cwd,
        cwd,
        &["ls", "--sort=extension", "--directories-first", "files"],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"z-dir\nz.a\na.z\n");

    let result = run(
        cwd,
        cwd,
        &[
            "ls",
            "--sort=none",
            "-r",
            "--directories-first",
            "-d",
            "files/a.z",
            "files/z-dir",
            "files/z.a",
        ],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"files/z-dir\nfiles/z.a\nfiles/a.z\n");

    let result = run(cwd, cwd, &["ls", "-C", "files/a.z", "missing"], b"");
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(result.stdout, b"files/a.z\n");
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));

    let result = run(cwd, cwd, &["ls", "--sort=modified", "files"], b"");
    assert!(result.status.success());
    assert_eq!(String::from_utf8_lossy(&result.stdout).lines().count(), 3);
}

#[test]
fn validates_explicit_sources_and_global_options() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    for args in [
        &["--config"][..],
        &["--config=missing.toml", "ls"],
        &["--profile", "unknown", "ls"],
        &["--color=invalid", "ls"],
        &["-j0", "ls"],
        &["unknown"],
    ] {
        let result = run(cwd, cwd, args, b"");
        assert_eq!(result.status.code(), Some(2), "{args:?}");
    }
    let result = run(cwd, cwd, &["--config=.", "ls"], b"");
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("regular file"));

    let result = run(cwd, cwd, &["--help"], b"");
    assert!(result.status.success());
    assert!(String::from_utf8_lossy(&result.stdout).contains("--progress=MODE"));

    let result = run(cwd, cwd, &["--version"], b"");
    assert!(result.status.success());
    assert!(result.stdout.starts_with(b"gutil "));
}

#[test]
fn handles_plain_columns_and_explicit_reverse_order() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    for name in ["one.a", "two.z", ".hidden"] {
        fs::write(cwd.join(name), []).unwrap();
    }
    let result = run(
        cwd,
        cwd,
        &[
            "ls",
            "-AC",
            "--sort=extension",
            "--reverse",
            "--no-directories-first",
        ],
        b"",
    );
    assert!(result.status.success());
    let listing = String::from_utf8(result.stdout).unwrap();
    let names: Vec<_> = listing.split_whitespace().collect();
    assert_eq!(names, ["two.z", "one.a", ".hidden"]);

    let result = run(
        cwd,
        cwd,
        &["ls", "-d", "--sort=none", "-r", "one.a", "two.z"],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"two.z\none.a\n");

    let result = run(cwd, cwd, &["ls", "--sort=unknown"], b"");
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn applies_explicit_cat_format_overrides() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(
        cwd.join("custom.toml"),
        "[cat]\nsqueeze-blank=true\nshow-ends=true\nshow-nonprinting=true\n",
    )
    .unwrap();
    for (numbering, expected) in [
        ("--number=all", &b"     1\t\n     2\t\n     3\t\x00"[..]),
        ("--number=nonblank", &b"\n\n     1\t\x00"[..]),
    ] {
        let result = run(
            cwd,
            cwd,
            &[
                "--config=custom.toml",
                "cat",
                numbering,
                "--no-squeeze-blank",
                "--no-show-ends",
                "--no-show-nonprinting",
            ],
            b"\n\n\x00",
        );
        assert!(result.status.success());
        assert_eq!(result.stdout, expected);
    }
}

#[test]
fn validates_profile_tables_and_discovered_file_types() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    let path = cwd.join("gutil.toml");
    for contents in ["profiles=1", "[profiles]\ninvalid=1"] {
        fs::write(&path, contents).unwrap();
        let result = run(cwd, cwd, &["pwd"], b"");
        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains("must be a table"));
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    let result = run(cwd, cwd, &["pwd"], b"");
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("regular file"));
}

#[test]
fn styles_text_diagnostics_and_reports_configured_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(
        cwd.join("gutil.toml"),
        "[global]\njobs=3\n[diagnostics]\nverbosity='verbose'\n",
    )
    .unwrap();
    let result = run(cwd, cwd, &["--color=always", "pwd", "extra"], b"");
    assert!(result.status.success());
    let errors = String::from_utf8(result.stderr).unwrap();
    assert!(errors.contains("jobs: 3"));
    assert!(errors.contains("\x1b["));
    assert!(errors.contains("ignoring non-option arguments"));

    let result = run(cwd, cwd, &["--color=always", "cat", "missing"], b"");
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing"));
}

#[cfg(unix)]
#[test]
fn quotes_control_bytes_in_styled_and_shell_listings() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join("escape\x1b"), []).unwrap();
    let result = run(cwd, cwd, &["--color=always", "ls"], b"");
    assert!(result.status.success());
    assert_eq!(result.stdout, b"escape?\n");
    let result = run(cwd, cwd, &["ls", "--quote=shell"], b"");
    assert!(result.status.success());
    assert_eq!(result.stdout, b"$'\\x65\\x73\\x63\\x61\\x70\\x65\\x1b'\n");
}

#[test]
fn preserves_explicit_short_listing_precedence_and_configurable_modes() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join(".hidden"), []).unwrap();
    for flags in [&["-aA"][..], &["--all", "--almost-all"]] {
        let mut args = vec!["ls"];
        args.extend(flags);
        let result = run(cwd, cwd, &args, b"");
        assert_eq!(result.stdout, b".\n..\n.hidden\n");
    }

    let result = run(cwd, cwd, &["ls", "-la1"], b"");
    assert!(result.status.success());
    assert!(result.stdout.starts_with(b"total "));
    let result = run(
        cwd,
        cwd,
        &["ls", "-la", "--layout=one-per-line", "--hidden=omit"],
        b"",
    );
    assert!(result.status.success());
    assert_eq!(result.stdout, b"");
}

#[test]
fn keeps_direct_command_help_available_with_invalid_config() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join("gutil.toml"), "[invalid]\nvalue=1").unwrap();
    for command in ["ls", "cat", "pwd"] {
        let help = run(cwd, cwd, &[command, "--help"], b"");
        assert!(help.status.success());
        assert!(help.stdout.starts_with(b"Usage: gutil "));
        let version = run(cwd, cwd, &[command, "--version"], b"");
        assert!(version.status.success());
        assert!(version.stdout.starts_with(b"gutil "));
    }
}
