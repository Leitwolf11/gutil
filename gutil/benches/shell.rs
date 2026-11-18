#![allow(clippy::expect_used)]

use std::env;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn shell(criterion: &mut Criterion) {
    let binary = env::var_os("GUTIL_BENCH_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_gutil")), PathBuf::from)
        .canonicalize()
        .expect("resolve the shell benchmark executable");
    let gutil = binary.as_path();
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);

    for (name, arguments, status) in [
        ("true", &[][..], 0),
        ("false", &[][..], 1),
        ("echo", &["-e", r"hello\tworld", r"\x21"], 0),
        ("printenv", &["-0", "GUTIL_BENCH_A", "GUTIL_BENCH_B"], 0),
        ("nproc", &["--ignore=1"], 0),
        ("sleep", &["0.002s"], 0),
    ] {
        let mut actual = process(gutil);
        actual
            .args(["--no-config", "--progress=never", name])
            .args(arguments);
        let result = output(&mut actual, status);
        let mut group = criterion.benchmark_group(format!("shell/{name}"));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(1));
        group.warm_up_time(Duration::from_millis(400));
        group.bench_function("gutil", |benchmark| {
            benchmark.iter(|| run(&mut actual, status));
        });
        if name == "true" {
            let mut fixed = process(gutil);
            fixed.args(["--no-config", "--progress=never", "--jobs=1", "true"]);
            assert_eq!(result, output(&mut fixed, status));
            group.bench_function("gutil-j1", |benchmark| {
                benchmark.iter(|| run(&mut fixed, status));
            });
        }
        if let Some(dir) = &gnu {
            let mut reference = process(&dir.join(name));
            reference.args(arguments);
            assert_eq!(result, output(&mut reference, status));
            group.bench_function("gnu", |benchmark| {
                benchmark.iter(|| run(&mut reference, status));
            });
        }
        group.finish();
    }

    let bytes = if cfg!(debug_assertions) {
        64 * 1024
    } else {
        16 * 1024 * 1024
    };
    let mut group = criterion.benchmark_group("shell/yes-pipe");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(2));
    group.warm_up_time(Duration::from_millis(400));
    group.throughput(Throughput::Bytes(bytes));
    let mut actual = process(gutil);
    actual.args(["--no-config", "--progress=never", "yes"]);
    verify_prefix(&mut actual);
    group.bench_function("gutil", |benchmark| {
        benchmark.iter(|| pipe(&mut actual, bytes, &mut io::sink()));
    });
    if let Some(dir) = gnu {
        let mut reference = process(&dir.join("yes"));
        verify_prefix(&mut reference);
        group.bench_function("gnu", |benchmark| {
            benchmark.iter(|| pipe(&mut reference, bytes, &mut io::sink()));
        });
    }
    group.finish();
}

fn process(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env_remove("POSIXLY_CORRECT")
        .env("OMP_NUM_THREADS", "4")
        .env("OMP_THREAD_LIMIT", "3")
        .env("GUTIL_BENCH_A", "alpha")
        .env("GUTIL_BENCH_B", "beta\ngamma")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command, status: i32) -> Vec<u8> {
    let output = command.output().expect("run shell parity check");
    assert_eq!(output.status.code(), Some(status));
    output.stdout
}

fn run(command: &mut Command, status: i32) {
    let status_actual = command
        .stdout(Stdio::null())
        .status()
        .expect("run shell benchmark");
    assert_eq!(status_actual.code(), Some(status));
}

fn verify_prefix(command: &mut Command) {
    let mut prefix = Vec::new();
    pipe(command, 8192, &mut prefix);
    assert_eq!(prefix, b"y\n".repeat(4096));
}

fn pipe(command: &mut Command, bytes: u64, output: &mut impl io::Write) {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start yes");
    let count = io::copy(
        &mut child.stdout.take().expect("take yes output").take(bytes),
        output,
    )
    .expect("drain yes output");
    assert_eq!(count, bytes);
    child.wait().expect("wait for yes");
}

criterion_group!(benches, shell);
criterion_main!(benches);
