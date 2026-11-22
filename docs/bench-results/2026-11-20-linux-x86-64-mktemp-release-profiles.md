# Linux x86-64 mktemp release profile comparison

Compare two gutil builds from commit `27ea9625aea91e54fa1022c48ad89f1e60d8a924` with GNU Coreutils 9.12. The candidate enables Thin LTO and uses one code generation unit. The baseline uses the repository’s default release profile. Both gutil builds use the same source and lockfile.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 4 logical CPUs, benchmark pinned to CPU 3 |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Filesystems | ext4 binaries, tmpfs fixtures |
| Compiler | rustc 1.100.0 with LLVM 23.1.5 |
| Reference | GNU Coreutils mktemp 9.12 |

The benchmark runs each executable 100 times per workload after 10 warmups in seeded interleaved order. Each timed observation includes process startup and exit. The harness verifies object type, path, permissions, and failure status outside the timed interval. See the raw samples for exact arguments, binary hashes, and the UTC measurement time.

| Workload | Default median (ms) | Thin LTO median (ms) | GNU median (ms) | Paired reduction vs. default (median and IQR) |
| --- | ---: | ---: | ---: | ---: |
| `file` | 2.670 | 2.582 | 1.447 | 4.42% [-5.20%, 10.36%] |
| `directory` | 2.720 | 2.499 | 1.390 | 8.44% [-1.54%, 16.74%] |
| `default-tmpdir` | 2.605 | 2.463 | 1.417 | 6.60% [-1.87%, 10.87%] |
| `dry-run` | 2.637 | 2.409 | 1.350 | 6.49% [-0.68%, 12.65%] |
| `quiet-missing-parent` | 2.627 | 2.451 | 1.348 | 8.03% [-1.15%, 13.10%] |

The executable shrank from 16,659,656 to 13,566,536 bytes, a reduction of 18.6%. Paired median latency improved by 4.4% to 8.4%, and the interquartile range crosses zero in every workload. GNU still has the lowest median in all five workloads. The measurements cover one Linux machine and do not establish a release-profile choice for other workloads or platforms.

[Raw samples and binary identities](2026-11-20-linux-x86-64-mktemp-release-profiles.json)

From the repository root, reproduce the builds and comparison with the same Rust toolchain.

```sh
cargo build --release --locked -p gutil
cp target/release/gutil target/mktemp-default
CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  cargo build --release --locked -p gutil --target-dir target/thin-profile
taskset -c 3 python3 benchmarks/mktemp.py target/thin-profile/release/gutil \
  --baseline target/mktemp-default --rounds 100 --warmup 10
```
