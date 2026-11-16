#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use criterion::{criterion_group, criterion_main, Criterion};

fn list_directories(criterion: &mut Criterion) {
    let directory = tempfile::tempdir().expect("create listing fixture");
    for index in 0..1_000 {
        fs::write(
            directory.path().join(format!("file-{index:04}")),
            b"fixture",
        )
        .expect("write fixture file");
    }

    let gnu_ls = env::var_os("GNU_LS");
    let mut group = criterion.benchmark_group("ls/1000-files");
    for flag in ["-1", "-la"] {
        group.bench_function(format!("gutil/{flag}"), |bench| {
            bench.iter(|| {
                run(
                    Path::new(env!("CARGO_BIN_EXE_gutil")),
                    &["--no-config", "ls", flag, "."],
                    directory.path(),
                );
            });
        });
        if let Some(executable) = gnu_ls.as_deref() {
            group.bench_function(format!("gnu/{flag}"), |bench| {
                bench.iter(|| run(Path::new(executable), &[flag, "."], directory.path()));
            });
        }
    }
    group.finish();
}

fn run(program: &Path, args: &[&str], directory: &Path) {
    let status = Command::new(program)
        .args(args)
        .current_dir(directory)
        .env("LC_ALL", "C")
        .stdout(Stdio::null())
        .status()
        .expect("run listing command");
    assert!(status.success(), "listing failed with {status}");
}

criterion_group!(benches, list_directories);
criterion_main!(benches);
