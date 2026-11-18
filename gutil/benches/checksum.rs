#![allow(clippy::expect_used)]

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

const FILE_BYTES: usize = 16 * 1024 * 1024;
const FILES: [&str; 4] = ["input-1", "input-2", "input-3", "input-4"];

fn checksums(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("create checksum fixture");
    let root = fixture.path();
    let binary = env::var_os("GUTIL_BENCH_BIN")
        .map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_gutil")), PathBuf::from)
        .canonicalize()
        .expect("resolve the checksum benchmark executable");
    let gutil = binary.as_path();
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    let mut block = vec![0u8; 64 * 1024];
    let file_bytes = if cfg!(debug_assertions) {
        block.len()
    } else {
        FILE_BYTES
    };
    for (index, byte) in block.iter_mut().enumerate() {
        *byte = u8::try_from(index % 251).expect("fixture value fits a byte");
    }
    for name in FILES {
        let mut file = File::create(root.join(name)).expect("create checksum input");
        for _ in 0..file_bytes / block.len() {
            file.write_all(&block).expect("write checksum input");
        }
    }

    for algorithm in [
        "md5sum",
        "sha1sum",
        "sha224sum",
        "sha256sum",
        "sha384sum",
        "sha512sum",
    ] {
        let mut serial = process(gutil, root);
        serial.args(["--no-config", "--progress=never", "--jobs=1", algorithm]);
        serial.args(FILES);
        let mut parallel = process(gutil, root);
        parallel.args(["--no-config", "--progress=never", "--jobs=4", algorithm]);
        parallel.args(FILES);
        let reference = output(&mut serial);
        assert_eq!(reference, output(&mut parallel));

        let mut group = criterion.benchmark_group(format!("checksum/{algorithm}/4x16MiB"));
        group.throughput(Throughput::Bytes((file_bytes * FILES.len()) as u64));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(2));
        group.warm_up_time(Duration::from_millis(400));
        for (name, command) in [("gutil-j1", &mut serial), ("gutil-j4", &mut parallel)] {
            group.bench_function(name, |bench| bench.iter(|| run(command)));
        }
        if let Some(gnu) = &gnu {
            let mut command = process(&gnu.join(algorithm), root);
            command.args(FILES);
            assert_eq!(reference, output(&mut command));
            group.bench_function("gnu", |bench| bench.iter(|| run(&mut command)));
        }
        group.finish();
    }
}

fn process(program: &Path, directory: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(directory)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command) -> Vec<u8> {
    let output = command.output().expect("run checksum parity check");
    assert!(output.status.success(), "{:?}", output.stderr);
    output.stdout
}

fn run(command: &mut Command) {
    let status = command
        .stdout(Stdio::null())
        .status()
        .expect("run checksum benchmark");
    assert!(status.success(), "{status}");
}

criterion_group!(benches, checksums);
criterion_main!(benches);
