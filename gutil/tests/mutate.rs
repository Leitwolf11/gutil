#![allow(clippy::unwrap_used)]

#[cfg(unix)]
#[path = "mutate/gnu_parity.rs"]
mod gnu_parity;

use std::path::Path;
use std::process::{Command, Output};

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

#[path = "mutate/links.rs"]
mod links;

#[path = "mutate/remove.rs"]
mod remove;

#[path = "mutate/truncate.rs"]
mod truncate;

#[path = "mutate/interface.rs"]
mod interface;
