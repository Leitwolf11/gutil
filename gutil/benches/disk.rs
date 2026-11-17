#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::Value;

fn disk(criterion: &mut Criterion) {
    let fixture = tempfile::tempdir().expect("create disk usage fixture");
    fs::write(fixture.path().join("sample"), []).expect("create disk query operand");
    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let gnu = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    let duf = env::var_os("REFERENCE_DUF").map(PathBuf::from);

    for (label, operands) in [
        ("df-one", vec!["sample"]),
        (
            "df-repeated",
            vec!["sample"; if cfg!(debug_assertions) { 16 } else { 512 }],
        ),
    ] {
        let mut actual = process(gutil, fixture.path());
        actual
            .args([
                "--no-config",
                "--progress=never",
                "df",
                "--output=source,size,target",
            ])
            .args(&operands);
        let reference = df_rows(&mut actual);
        let mut group = criterion.benchmark_group(format!("disk/{label}"));
        group.sample_size(10);
        group.measurement_time(Duration::from_secs(1));
        group.warm_up_time(Duration::from_millis(400));
        group.bench_function("gutil", |bench| bench.iter(|| run(&mut actual)));
        if let Some(directory) = &gnu {
            let mut command = process(&directory.join("df"), fixture.path());
            command.arg("--output=source,size,target").args(&operands);
            assert_eq!(reference, df_rows(&mut command));
            group.bench_function("gnu", |bench| bench.iter(|| run(&mut command)));
        }
        group.finish();
    }

    let mut actual = process(gutil, fixture.path());
    actual.args(["--no-config", "--progress=never", "duf", "--all", "--json"]);
    let expected = disk_rows(&mut actual);
    let mut group = criterion.benchmark_group("disk/duf-json-inventory");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(1));
    group.warm_up_time(Duration::from_millis(400));
    group.bench_function("gutil", |bench| bench.iter(|| run(&mut actual)));
    if let Some(binary) = duf {
        let mut reference = process(&binary, fixture.path());
        reference.args(["-all", "-json"]);
        let values = disk_rows(&mut reference);
        let common = expected
            .iter()
            .filter(|(mount, _)| values.contains_key(*mount));
        assert!(common.clone().count() > 0);
        for (mount, capacity) in common {
            assert_eq!(Some(capacity), values.get(mount), "capacity for {mount}");
        }
        group.bench_function("duf", |bench| bench.iter(|| run(&mut reference)));
    }
    group.finish();
}

fn process(program: &Path, root: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(root)
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env_remove("POSIXLY_CORRECT")
        .env_remove("DF_BLOCK_SIZE")
        .env_remove("BLOCK_SIZE")
        .env_remove("BLOCKSIZE")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command) -> String {
    let output = command.output().expect("query disk capacity");
    assert!(output.status.success(), "{:?}", output.stderr);
    String::from_utf8(output.stdout).expect("decode matching disk records")
}

fn df_rows(command: &mut Command) -> Vec<Vec<String>> {
    output(command)
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect()
}

fn disk_rows(command: &mut Command) -> BTreeMap<String, u64> {
    serde_json::from_str::<Vec<Value>>(&output(command))
        .expect("parse JSON disk inventory")
        .into_iter()
        .filter_map(|row| {
            Some((
                row["mount_point"].as_str()?.to_owned(),
                row["total"].as_u64()?,
            ))
        })
        .collect()
}

fn run(command: &mut Command) {
    let status = command
        .stdout(Stdio::null())
        .status()
        .expect("run disk benchmark");
    assert!(status.success(), "{status}");
}

criterion_group!(benches, disk);
criterion_main!(benches);
