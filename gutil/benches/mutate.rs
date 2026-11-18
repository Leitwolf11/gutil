#![allow(clippy::expect_used)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

#[derive(Clone, Copy)]
enum Case {
    Link,
    Ln,
    Unlink,
    Rmdir,
    TruncateOne,
    TruncateMany,
}

impl Case {
    const fn utility(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Ln => "ln",
            Self::Unlink => "unlink",
            Self::Rmdir => "rmdir",
            Self::TruncateOne | Self::TruncateMany => "truncate",
        }
    }

    fn count(self) -> usize {
        if matches!(self, Self::TruncateMany | Self::Rmdir) {
            if cfg!(debug_assertions) {
                8
            } else {
                512
            }
        } else {
            1
        }
    }

    fn operands(self) -> Vec<String> {
        match self {
            Self::Link | Self::Ln => vec!["source".into(), "linked".into()],
            Self::Unlink => vec!["source".into()],
            Self::Rmdir => (0..self.count())
                .map(|index| format!("item-{index}"))
                .collect(),
            Self::TruncateOne | Self::TruncateMany => std::iter::once("-s1024".into())
                .chain((0..self.count()).map(|index| format!("item-{index}")))
                .collect(),
        }
    }

    fn prepare(self) -> tempfile::TempDir {
        let fixture = tempfile::tempdir().expect("create mutation fixture");
        let root = fixture.path();
        match self {
            Self::Link | Self::Ln | Self::Unlink => {
                fs::write(root.join("source"), b"content").expect("write source");
            }
            Self::Rmdir => {
                for index in 0..self.count() {
                    fs::create_dir(root.join(format!("item-{index}")))
                        .expect("create empty directory");
                }
            }
            Self::TruncateOne | Self::TruncateMany => {
                for index in 0..self.count() {
                    fs::write(root.join(format!("item-{index}")), b"content").expect("write file");
                }
            }
        }
        fixture
    }

    fn verify(self, root: &Path) {
        match self {
            Self::Link | Self::Ln => {
                assert_eq!(
                    fs::read(root.join("linked")).expect("read link"),
                    b"content"
                );
                assert!(
                    same_file::is_same_file(root.join("source"), root.join("linked"))
                        .expect("compare hard links")
                );
            }
            Self::Unlink => assert!(!root.join("source").exists()),
            Self::Rmdir => {
                for index in 0..self.count() {
                    assert!(!root.join(format!("item-{index}")).exists());
                }
            }
            Self::TruncateOne | Self::TruncateMany => {
                for index in 0..self.count() {
                    let bytes =
                        fs::read(root.join(format!("item-{index}"))).expect("read resized file");
                    assert_eq!(bytes.len(), 1024);
                    assert_eq!(&bytes[..7], b"content");
                    assert!(bytes[7..].iter().all(|&byte| byte == 0));
                }
            }
        }
    }
}

fn command(binary: &Path, case: Case, gutil: bool) -> Command {
    let mut command = Command::new(binary);
    command
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env_remove("VERSION_CONTROL")
        .env_remove("SIMPLE_BACKUP_SUFFIX")
        .stdin(Stdio::null());
    if gutil {
        command.args([
            "--no-config",
            "--progress=never",
            "--jobs=1",
            case.utility(),
        ]);
    }
    command.args(case.operands());
    command
}

fn measure(criterion: &mut Criterion) {
    let gutil = Path::new(env!("CARGO_BIN_EXE_gutil"));
    let reference = env::var_os("GNU_COREUTILS_DIR").map(PathBuf::from);
    for (name, case) in [
        ("link", Case::Link),
        ("ln-hard", Case::Ln),
        ("unlink", Case::Unlink),
        ("rmdir-many", Case::Rmdir),
        ("truncate-one", Case::TruncateOne),
        ("truncate-many", Case::TruncateMany),
    ] {
        let mut commands = vec![("gutil", command(gutil, case, true))];
        if let Some(reference) = &reference {
            commands.push(("gnu", command(&reference.join(case.utility()), case, false)));
        }
        let mut group = criterion.benchmark_group(format!("mutate/{name}"));
        group
            .sample_size(10)
            .warm_up_time(Duration::from_millis(400))
            .measurement_time(Duration::from_secs(1));
        group.throughput(Throughput::Elements(case.count() as u64));
        for (variant, mut command) in commands {
            let fixture = case.prepare();
            let output = command
                .current_dir(fixture.path())
                .output()
                .expect("verify mutation");
            assert!(output.status.success(), "{:?}", output.stderr);
            assert_eq!(output.stdout, b"");
            case.verify(fixture.path());
            command.stdout(Stdio::null()).stderr(Stdio::null());
            group.bench_function(variant, |bench| {
                bench.iter_custom(|iterations| {
                    let mut elapsed = Duration::ZERO;
                    for _ in 0..iterations {
                        let fixture = case.prepare();
                        command.current_dir(fixture.path());
                        let start = Instant::now();
                        let status = command.status().expect("run mutation");
                        elapsed += start.elapsed();
                        assert!(status.success());
                        // Each iteration receives a fresh fixture. Preparation and
                        // cleanup stay outside the process elapsed-time interval.
                    }
                    elapsed
                });
            });
        }
        group.finish();
    }
}

criterion_group!(benches, measure);
criterion_main!(benches);
