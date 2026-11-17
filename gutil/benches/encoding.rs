#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn encoding(criterion: &mut Criterion) {
    let directory = tempfile::tempdir().expect("create fixture");
    let root = directory.path();
    let size = if cfg!(debug_assertions) {
        16 * 1024
    } else {
        8 * 1024 * 1024
    };
    let input: Vec<u8> = (0u8..=255).cycle().take(size).collect();
    fs::write(root.join("input"), &input).expect("write encoding fixture");
    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    for utility in ["base32", "base64"] {
        let mut prepare = gutil_command(gutil, root, utility);
        prepare.arg("input");
        let encoded = output(&mut prepare);
        fs::write(root.join("encoded"), &encoded).expect("write wrapped fixture");
        for (case, operands) in [
            ("encode-wrapped", &["input"][..]),
            ("encode-unwrapped", &["-w0", "input"]),
            ("decode-wrapped", &["-d", "encoded"]),
        ] {
            let mut actual = gutil_command(gutil, root, utility);
            actual.args(operands);
            let result = output(&mut actual);
            if case == "decode-wrapped" {
                assert_eq!(result, input);
            }
            let mut group = criterion.benchmark_group(format!("encoding/{utility}/{case}"));
            group.sample_size(10);
            group.warm_up_time(Duration::from_millis(400));
            group.measurement_time(Duration::from_secs(1));
            group.throughput(Throughput::Bytes(size as u64));
            group.bench_function("gutil", |bench| bench.iter(|| run(&mut actual)));
            if let Some(reference) = &gnu {
                let mut standard = command(&reference.join(utility), root);
                standard.args(operands);
                assert_eq!(result, output(&mut standard));
                group.bench_function("gnu", |bench| bench.iter(|| run(&mut standard)));
            }
            group.finish();
        }
    }
}

fn command(binary: &Path, root: &Path) -> Command {
    let mut cmd = Command::new(binary);
    cmd.current_dir(root)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    cmd
}

fn gutil_command(binary: &Path, root: &Path, utility: &str) -> Command {
    let mut cmd = command(binary, root);
    cmd.args(["--no-config", "--progress=never", utility]);
    cmd
}

fn output(command: &mut Command) -> Vec<u8> {
    let result = command.output().expect("capture encoding");
    assert!(result.status.success(), "{:?}", result.stderr);
    result.stdout
}

fn run(command: &mut Command) {
    let status = command
        .stdout(Stdio::null())
        .status()
        .expect("run codec benchmark");
    assert!(status.success(), "{status}");
}

criterion_group!(benches, encoding);
criterion_main!(benches);
