#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn invocation(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env_remove("TMPDIR")
        .args([
            "--no-config",
            "--color=never",
            "--progress=never",
            "--jobs=1",
            "mktemp",
        ]);
    command
}

fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}

fn path(output: &Output) -> PathBuf {
    success(output);
    let text = String::from_utf8(output.stdout.clone()).unwrap();
    PathBuf::from(text.strip_suffix('\n').unwrap().trim_end_matches('\r'))
}

fn assert_name(path: &Path, prefix: &str, suffix: &str, count: usize) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let random = name
        .strip_prefix(prefix)
        .unwrap()
        .strip_suffix(suffix)
        .unwrap();
    assert_eq!(random.len(), count);
    assert!(random.bytes().all(|byte| byte.is_ascii_alphanumeric()));
}

#[path = "mktemp/creation.rs"]
mod creation;

#[path = "mktemp/selection.rs"]
mod selection;

#[path = "mktemp/options.rs"]
mod options;

#[path = "mktemp/io.rs"]
mod io;

#[cfg(unix)]
#[path = "mktemp/gnu_parity.rs"]
mod gnu_parity;
