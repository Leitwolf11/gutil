#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

const FILTERS: [&str; 5] = ["--type", "file", "--extension", "txt", "^match-"];

fn fd(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("create search fixture");
    let root = fixture.path();
    let branches = if cfg!(debug_assertions) { 2 } else { 64 };
    let files = if cfg!(debug_assertions) { 8 } else { 128 };
    fs::write(root.join(".fdignore"), "ignore-me/\n").expect("write ignore rules");
    for branch in 0..branches {
        let directory = root.join(format!("dir-{branch:03}"));
        fs::create_dir_all(directory.join("ignore-me")).expect("create search fixture directory");
        for index in 0..files {
            let prefix = if index % 16 == 0 { "match" } else { "other" };
            let path = directory.join(format!("{prefix}-{index:03}.txt"));
            fs::write(path, []).expect("create fixture file");
        }
        fs::write(directory.join("ignore-me/match-hidden.txt"), [])
            .expect("create ignored fixture file");
    }

    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let reference = env::var_os("REFERENCE_FD").map(PathBuf::from);
    let mut group = criterion.benchmark_group("fd/ignore-and-regex");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    group.warm_up_time(Duration::from_millis(400));
    let mut expected = None;
    for (workers, gutil_label, reference_label) in
        [("1", "gutil-j1", "fd-j1"), ("4", "gutil-j4", "fd-j4")]
    {
        let mut gutil = process(gutil, root);
        gutil
            .args(["--no-config", "--progress=never", "--jobs", workers, "fd"])
            .args(FILTERS);
        let actual = output(&mut gutil);
        if let Some(expected) = &expected {
            assert_eq!(*expected, actual);
        }
        group.bench_function(gutil_label, |bench| bench.iter(|| run(&mut gutil)));
        if let Some(reference) = &reference {
            let mut command = process(reference, root);
            command
                .args(["--color=never", "--threads", workers])
                .args(FILTERS);
            assert_eq!(actual, output(&mut command));
            group.bench_function(reference_label, |bench| bench.iter(|| run(&mut command)));
        }
        expected = Some(actual);
    }
    group.finish();
}

fn process(program: &Path, root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command) -> Vec<Vec<u8>> {
    let output = command.output().expect("run search parity check");
    assert!(output.status.success(), "{:?}", output.stderr);
    let mut entries: Vec<_> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(<[u8]>::to_vec)
        .collect();
    entries.sort_unstable();
    entries
}

fn run(command: &mut Command) {
    let status = command
        .stdout(Stdio::null())
        .status()
        .expect("run search benchmark");
    assert!(status.success(), "{status}");
}

criterion_group!(benches, fd);
criterion_main!(benches);
