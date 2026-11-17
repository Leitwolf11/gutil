#![allow(clippy::expect_used)]

use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

fn streams(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("create stream fixture");
    let root = fixture.path();
    let input = root.join("input");
    let mut block = vec![b'x'; 64 * 1024];
    for record in block.as_chunks_mut::<64>().0 {
        record[63] = b'\n';
    }
    let mut file = File::create(&input).expect("create stream input");
    let blocks = if cfg!(debug_assertions) { 1 } else { 1024 };
    for _ in 0..blocks {
        file.write_all(&block).expect("write stream input");
    }
    drop(file);

    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    for (label, utility, args, piped) in [
        ("head-lines", "head", &["-n1000", "input"][..], false),
        ("tail-file", "tail", &["-n1000", "input"], false),
        ("tail-pipe", "tail", &["-n1000"], true),
        ("wc-lines", "wc", &["-l", "input"], false),
        ("wc-bytes", "wc", &["-c", "input"], false),
        ("cat", "cat", &["input"], false),
        ("pwd", "pwd", &["-P"], false),
    ] {
        let stdin = piped.then_some(input.as_path());
        let mut actual = process(gutil, root);
        actual
            .args(["--no-config", "--progress=never", utility])
            .args(args);
        let mut reference = gnu.as_ref().map(|gnu| {
            let mut command = process(&gnu.join(utility), root);
            command.args(args);
            command
        });
        if let Some(reference) = &mut reference {
            assert_eq!(output(&mut actual, stdin), output(reference, stdin));
        }

        let mut group = criterion.benchmark_group(format!("streams/{label}"));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(1));
        group.warm_up_time(Duration::from_millis(400));
        group.bench_function("gutil", |bench| bench.iter(|| run(&mut actual, stdin)));
        if let Some(reference) = &mut reference {
            group.bench_function("gnu", |bench| bench.iter(|| run(reference, stdin)));
        }
        group.finish();
    }
}

fn process(program: &Path, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(directory)
        .env("LC_ALL", "C")
        .env("TZ", "UTC");
    command
}

fn output(command: &mut Command, input: Option<&Path>) -> Vec<u8> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = if let Some(input) = input {
        let mut source = File::open(input).expect("open parity input");
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .expect("start parity check");
        let mut writer = child.stdin.take().expect("get parity stdin");
        std::thread::scope(|scope| {
            let write = scope.spawn(move || {
                io::copy(&mut source, &mut writer).expect("write parity stdin");
            });
            let output = child.wait_with_output().expect("collect parity output");
            write.join().expect("join parity writer");
            output
        })
    } else {
        command
            .stdin(Stdio::null())
            .output()
            .expect("run parity check")
    };
    assert!(output.status.success(), "{:?}", output.stderr);
    output.stdout
}

fn run(command: &mut Command, input: Option<&Path>) {
    command.stdout(Stdio::null()).stderr(Stdio::inherit());
    let status = if let Some(input) = input {
        let mut source = File::open(input).expect("open benchmark input");
        let mut child = command
            .stdin(Stdio::piped())
            .spawn()
            .expect("start benchmark");
        io::copy(
            &mut source,
            &mut child.stdin.take().expect("get benchmark stdin"),
        )
        .expect("write benchmark stdin");
        child.wait().expect("wait for benchmark")
    } else {
        command
            .stdin(Stdio::null())
            .status()
            .expect("run benchmark")
    };
    assert!(status.success(), "{status}");
}

criterion_group!(benches, streams);
criterion_main!(benches);
