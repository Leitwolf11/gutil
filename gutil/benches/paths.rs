#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

struct Workload<'a> {
    command: &'a str,
    flags: &'a [&'a str],
    names: &'a [PathBuf],
    parallel: bool,
}

fn paths(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("create path fixture");
    let root = fixture.path();
    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    let count = if cfg!(debug_assertions) { 24 } else { 512 };
    let mut files = Vec::with_capacity(count);
    #[cfg(unix)]
    let mut links = Vec::with_capacity(count);
    #[cfg(not(unix))]
    let links: Vec<PathBuf> = Vec::new();
    fs::create_dir(root.join("inputs")).expect("create input directory");
    for i in 0..count {
        let name = PathBuf::from(format!("inputs/item-{i:04}.txt"));
        fs::write(root.join(&name), b"test fixture\n").expect("create input file");
        #[cfg(unix)]
        {
            let link = PathBuf::from(format!("link-{i:04}"));
            std::os::unix::fs::symlink(&name, root.join(&link)).expect("create symlink");
            links.push(link);
        }
        files.push(name);
    }
    let mut workloads = vec![
        Workload {
            command: "basename",
            flags: &["-zs.txt"],
            names: &files,
            parallel: false,
        },
        Workload {
            command: "dirname",
            flags: &["-z"],
            names: &files,
            parallel: false,
        },
        Workload {
            command: "realpath",
            flags: &["-e", "-z"],
            names: &files,
            parallel: true,
        },
    ];
    if !links.is_empty() {
        workloads.push(Workload {
            command: "readlink",
            flags: &["-z"],
            names: &links,
            parallel: true,
        });
    }
    for workload in workloads {
        let mut serial = process(gutil, root);
        serial.args([
            "--no-config",
            "--progress=never",
            "--jobs=1",
            workload.command,
        ]);
        serial.args(workload.flags).args(workload.names);
        let expected = output(&mut serial);
        let mut group = criterion.benchmark_group(format!("paths/{}", workload.command));
        group.throughput(Throughput::Elements(count as u64));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(1));
        group.warm_up_time(Duration::from_millis(400));
        group.bench_function("gutil-j1", |bench| bench.iter(|| run(&mut serial)));
        if workload.parallel {
            let mut parallel = process(gutil, root);
            parallel.args([
                "--no-config",
                "--progress=never",
                "--jobs=4",
                workload.command,
            ]);
            parallel.args(workload.flags).args(workload.names);
            assert_eq!(expected, output(&mut parallel));
            group.bench_function("gutil-j4", |bench| bench.iter(|| run(&mut parallel)));
        }
        if let Some(gnu) = &gnu {
            let mut reference = process(&gnu.join(workload.command), root);
            reference.args(workload.flags).args(workload.names);
            assert_eq!(expected, output(&mut reference));
            group.bench_function("gnu", |bench| bench.iter(|| run(&mut reference)));
        }
        group.finish();
    }
}

fn process(program: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env_remove("POSIXLY_CORRECT")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command) -> Vec<u8> {
    let output = command.output().expect("run path parity check");
    assert!(output.status.success(), "{:?}", output.stderr);
    output.stdout
}

fn run(command: &mut Command) {
    let status = command
        .stdout(Stdio::null())
        .status()
        .expect("run path benchmark");
    assert!(status.success(), "{status}");
}

criterion_group!(benches, paths);
criterion_main!(benches);
