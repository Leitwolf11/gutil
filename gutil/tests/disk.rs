#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(utility: &str, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gutil"));
    command
        .args(["--no-config", "--color=never", "--progress=never", utility])
        .args(args)
        .env("LC_ALL", "C")
        .env_remove("POSIXLY_CORRECT");
    for variable in ["DF_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"] {
        command.env_remove(variable);
    }
    command.output().unwrap()
}

fn success(output: Output) -> String {
    let Output {
        status,
        stdout,
        stderr,
    } = output;
    assert!(status.success(), "{}", String::from_utf8_lossy(&stderr));
    String::from_utf8(stdout).unwrap()
}

#[test]
fn df_reports_the_filesystem_of_files_and_preserves_operand_order() {
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("first");
    let second = fixture.path().join("second");
    std::fs::write(&first, []).unwrap();
    std::fs::write(&second, []).unwrap();
    let args = [
        "--output=file,size,target",
        "-B1",
        first.to_str().unwrap(),
        second.to_str().unwrap(),
    ];
    let table = success(run("df", &args));
    let records: Vec<_> = table.lines().collect();
    assert_eq!(records.len(), 3);
    assert!(records[0].contains("1B-blocks"));
    assert!(records[1].contains("first"));
    assert!(records[2].contains("second"));
    let size_only = success(run(
        "df",
        &["--output=size", "-B1", first.to_str().unwrap()],
    ));
    assert!(
        size_only
            .lines()
            .nth(1)
            .unwrap()
            .trim()
            .parse::<u64>()
            .unwrap()
            > 0
    );

    let failure = run(
        "df",
        &[
            "--output=file",
            first.to_str().unwrap(),
            fixture.path().join("missing").to_str().unwrap(),
        ],
    );
    assert_eq!(failure.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failure.stdout).contains("first"));
    assert!(String::from_utf8_lossy(&failure.stderr).contains("missing"));
}

#[test]
fn df_selects_columns_formats_sizes_and_sums_rows() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().to_str().unwrap();
    let table = success(run(
        "df",
        &[
            "-P",
            "-B512",
            "-T",
            "-v",
            "-k",
            "--total",
            "--no-sync",
            path,
            path,
        ],
    ));
    let records: Vec<_> = table.lines().collect();
    assert_eq!(records.len(), 4);
    assert!(records[0].contains("Available") && records[0].contains("Capacity"));
    assert!(records[3].starts_with("total"));
    let inodes = success(run("df", &["--inodes", "--print-type", "--local", path]));
    assert!(inodes.contains("IUse%") && inodes.contains("Type"));
    let human = success(run("df", &["--human-readable", path]));
    assert!(human.contains("Size"));
    let decimal = success(run("df", &["--si", path]));
    assert!(decimal.contains("Size"));
    let fields = success(run("df", &["--output", path]));
    assert!(fields.contains("Inodes") && fields.contains("File"));
    for flags in [
        &["--output=size,size"][..],
        &["--output=undefined"],
        &["--output=size", "-Ti"],
        &["--block-size=0"],
        &["--block-size=invalid"],
        &["--type", "gutil-missing-filesystem-type"],
        &["-Q"],
        &["--no-such-flag"],
        &["-B"],
    ] {
        assert_eq!(run("df", flags).status.code(), Some(1), "{flags:?}");
    }
}

#[test]
fn duf_filters_json_snapshots_and_renders_plain_ascii_tables() {
    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().to_str().unwrap();
    let raw = success(run("duf", &["-json", "--sort=mountpoint", path, path]));
    let entries: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    let filesystem = entry["fs_type"].as_str().unwrap();
    let size = entry["total"].as_u64().unwrap();
    let mount = entry["mount_point"].as_str().unwrap();
    assert!(size > 0 && Path::new(mount).is_absolute());
    assert!(entry["free"].as_u64().unwrap() <= size);
    let included = success(run(
        "duf",
        &[
            "--json",
            "--only-fs",
            filesystem,
            "--only-mp=*",
            "--sort",
            "size",
            "--warnings",
            path,
        ],
    ));
    assert_eq!(
        serde_json::from_str::<Vec<Value>>(&included).unwrap().len(),
        1
    );
    let excluded = success(run("duf", &["--json", "--hide-fs", filesystem, path]));
    assert_eq!(excluded.trim(), "[]");
    let table = success(run(
        "duf",
        &[
            "-style",
            "ascii",
            "--width=60",
            "--output=mountpoint,size",
            path,
        ],
    ));
    assert!(table.contains("Mounted on") && table.contains("Size") && table.is_ascii());
    for flags in [
        &["--only=invalid"][..],
        &["--output=invalid"],
        &["--width=0"],
        &["--sort=invalid"],
        &["--style=invalid"],
        &["--not-a-flag"],
        &["--only-mp=[bad"],
    ] {
        assert_eq!(run("duf", flags).status.code(), Some(1), "{flags:?}");
    }
}

#[test]
fn shows_help_and_version_for_each_disk_command() {
    for command in ["df", "duf"] {
        let help = success(run(command, &["--help"]));
        assert!(help.starts_with(&format!("Usage: gutil {command} ")));
        let version = success(run(command, &["--version"]));
        assert_eq!(
            version,
            format!("gutil {command} {}\n", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[test]
fn observes_df_block_size_labels_and_portable_defaults() {
    let location = tempfile::tempdir().unwrap();
    let path = location.path().to_str().unwrap();
    for (flag, heading, suffix) in [
        ("-BM", "1M-blocks", "M"),
        ("-B2MB", "2MB-blocks", ""),
        ("-BKiB", "1K-blocks", "KiB"),
        ("-B1", "1B-blocks", ""),
    ] {
        let output = success(run("df", &["--output=size", flag, "--", path]));
        let mut lines = output.lines().map(str::trim);
        assert_eq!(lines.next(), Some(heading));
        let size = lines.next().unwrap().strip_suffix(suffix).unwrap();
        assert!(size.parse::<u64>().unwrap() > 0);
    }
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args(["--no-config", "df", "-P", path])
        .env("LC_ALL", "C")
        .env("POSIXLY_CORRECT", "1")
        .env("DF_BLOCK_SIZE", "1")
        .output()
        .unwrap();
    assert!(success(output)
        .lines()
        .next()
        .unwrap()
        .contains("512-blocks"));
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args(["--no-config", "df", "--output=size", path])
        .env("LC_ALL", "C")
        .env("DF_BLOCK_SIZE", "MB")
        .output()
        .unwrap();
    let table = success(output);
    assert_eq!(table.lines().next().unwrap().trim(), "1MB-blocks");
    assert!(table.lines().nth(1).unwrap().ends_with("MB"));
}

#[test]
fn compares_consistent_mount_fields_across_worker_counts() {
    let read = |workers: &str| {
        let json = success(run("duf", &["--all", "--json", workers, "--sort=size"]));
        let rows: Vec<Value> = serde_json::from_str(&json).unwrap();
        assert_ne!(rows, Vec::<Value>::new());
        rows.into_iter()
            .map(|row| {
                (
                    row["mount_point"].as_str().unwrap().to_owned(),
                    row["total"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(read("--threads=1"), read("--jobs=4"));
    let ordinary = success(run(
        "df",
        &["--all", "--output=source,fstype,used,avail,pcent,target"],
    ));
    assert!(ordinary.contains("Mounted on") && ordinary.contains("Avail"));
    let mounted = success(run(
        "duf",
        &["--json", "--only=local,network,fuse,special,loops,binds"],
    ));
    assert_ne!(
        serde_json::from_str::<Vec<Value>>(&mounted).unwrap(),
        Vec::<Value>::new()
    );
    let excluded = success(run(
        "duf",
        &["--json", "--hide=local,network,fuse,special,loops,binds"],
    ));
    assert_eq!(excluded.trim(), "[]");
    let excluded = success(run("duf", &["--json", "--hide-mp=*"]));
    assert_eq!(excluded.trim(), "[]");
}

#[test]
fn supports_short_filter_flags_and_duf_inode_sort_options() {
    let location = tempfile::tempdir().unwrap();
    let path = location.path().to_str().unwrap();
    let json = success(run("duf", &["--json", path]));
    let entries: Vec<Value> = serde_json::from_str(&json).unwrap();
    let kind = entries[0]["fs_type"].as_str().unwrap();
    let output = success(run("df", &["-ahj1", "-t", kind, path]));
    assert!(output.contains("Size"));
    let output = success(run("df", &["-Hl", "-x", "unavailable", path]));
    assert!(output.contains("Size"));
    let rejected = run("df", &["--exclude-type", kind, path]);
    assert_eq!(rejected.status.code(), Some(1));
    let output = success(run(
        "df",
        &["--sync", "--no-sync", "--block-size=human-readable", path],
    ));
    assert!(output.contains("Size"));
    let output = success(run("df", &["--block-size=si", path]));
    assert!(output.contains("Size"));
    let output = success(run(
        "duf",
        &[
            "--all",
            "--inodes",
            "--style=unicode",
            "--sort=inodes_usage",
            "--output=mountpoint,inodes,inodes_used,inodes_avail,inodes_usage",
            path,
        ],
    ));
    assert!(output.contains("Inodes"));
    let output = Command::new(env!("CARGO_BIN_EXE_gutil"))
        .args([
            "--no-config",
            "--color=always",
            "duf",
            "--output=size",
            path,
        ])
        .output()
        .unwrap();
    assert!(success(output).contains("\x1b["));
}

#[cfg(target_os = "macos")]
#[test]
fn maps_firmlinked_operands_to_the_same_volume_device() {
    use std::os::unix::fs::MetadataExt;

    let fixture = tempfile::tempdir().unwrap();
    let path = fixture.path().to_str().unwrap();
    let output = success(run("duf", &["--json", path]));
    let mounts: Vec<Value> = serde_json::from_str(&output).unwrap();
    let mount = mounts[0]["mount_point"].as_str().unwrap();
    assert_eq!(
        std::fs::metadata(mount).unwrap().dev(),
        std::fs::metadata(fixture.path()).unwrap().dev()
    );
}
