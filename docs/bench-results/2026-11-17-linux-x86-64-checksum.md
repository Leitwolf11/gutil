# Linux x86-64 checksum benchmark

Measured on November 17, 2026, in Europe/Istanbul. This run compares gutil
0.7.0 at commit `9bc61ae2867812e59d4993c12cce787e5140048f` with GNU Coreutils 9.12.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 physical cores, 4 logical CPUs |
| Available CPUs | All 4 logical CPUs, without isolation |
| Frequency policy | `intel_cpufreq` driver and `schedutil` governor |
| RAM | 7.61 GiB visible to Linux |
| Operating system | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 for the checkout and executable, tmpfs for input files under `/tmp` |
| Rust toolchain | rustc 1.100.0 and Cargo 1.100.0, host `x86_64-unknown-linux-gnu` |
| Harness | Criterion 0.8.2, optimized Cargo bench profile |

## Workload

Each invocation hashes four 16 MiB regular files for 64 MiB of total input.
Each file contains a repeated 64 KiB block whose byte at offset `i` is
`i mod 251`. The gutil variants use `--no-config --progress=never` and either
`--jobs=1` or `--jobs=4`. The GNU variants use the matching standalone binary
from `/usr/bin`. All variants receive the same operands in the same order.
The harness checks exact stdout equality before timing. [Benchmark harness](../../gutil/benches/checksum.rs)

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench checksum -- --noplot
```

Each measurement includes startup, hashing, record formatting, and exit.
Stdout goes to the null device. The harness sets `LC_ALL=C` and `TZ=UTC`.
The input files are reused after a 400 ms warmup. Criterion collected 10
samples per variant, extending the time target when a variant needed more
than two seconds. The run used a shared desktop session.

## Results

Each timing cell shows Criterion's point estimate in milliseconds and its
95% confidence interval in parentheses. The percentage columns compare the
four-worker point estimate against the one-worker and GNU point estimates.

| Command | gutil, 1 worker | gutil, 4 workers | GNU | Time saved vs. 1 worker | Time saved vs. GNU |
| --- | ---: | ---: | ---: | ---: | ---: |
| `md5sum` | 161.1 (156.7–167.0) | 83.9 (80.0–94.0) | 134.4 (132.3–137.0) | 47.9% | 37.6% |
| `sha1sum` | 193.6 (178.1–212.9) | 102.3 (95.7–110.8) | 129.2 (124.0–134.6) | 47.1% | 20.8% |
| `sha224sum` | 413.7 (398.2–433.6) | 270.0 (238.1–311.7) | 348.2 (303.0–393.5) | 34.7% | 22.5% |
| `sha256sum` | 512.1 (444.1–595.4) | 260.9 (241.9–284.8) | 284.4 (263.1–314.7) | 49.0% | 8.2% |
| `sha384sum` | 327.7 (294.9–375.4) | 149.0 (141.3–158.0) | 188.3 (180.3–197.7) | 54.5% | 20.9% |
| `sha512sum` | 270.0 (252.6–291.0) | 148.7 (140.4–157.6) | 190.1 (179.8–202.0) | 44.9% | 21.8% |

Four gutil workers reduced time by 34.7–54.5% relative to one worker.
The GNU comparison showed 8.2–37.6% lower point estimates with four workers.
The GNU and four-worker confidence intervals overlapped for `sha224sum` and `sha256sum`.
The measurements describe warm tmpfs reads across four files on this machine.
