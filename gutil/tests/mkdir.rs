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

#[path = "mkdir/options.rs"]
mod options;

#[path = "mkdir/permissions.rs"]
mod permissions;

#[path = "mkdir/contexts.rs"]
mod contexts;

#[path = "mkdir/errors.rs"]
mod errors;

#[path = "mkdir/paths.rs"]
mod paths;

#[cfg(unix)]
#[path = "mkdir/gnu_parity.rs"]
mod gnu_parity;

#[cfg(unix)]
fn masked(root: &Path, mask: &str, arguments: &[&str]) -> Output {
    let mut command = Command::new("sh");
    command.current_dir(root).env("LC_ALL", "C").args([
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
    ]);
    if std::env::var_os("CARGO_LLVM_COV").is_some()
        && u32::from_str_radix(mask, 8).unwrap() & 0o600 != 0
    {
        use std::os::unix::fs::PermissionsExt;
        // Restrictive child umasks must affect directories, not make the
        // profiler's own output unreadable to the parent coverage collector.
        let template = std::env::var_os("LLVM_PROFILE_FILE").unwrap();
        let directory = Path::new(&template).parent().unwrap();
        let profile = tempfile::Builder::new()
            .prefix("gutil-mkdir-mask-")
            .suffix(".profraw")
            .tempfile_in(directory)
            .unwrap();
        profile
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
        let (file, path) = profile.keep().unwrap();
        drop(file);
        command.env("LLVM_PROFILE_FILE", path);
    }
    command.args(arguments).output().unwrap()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}
