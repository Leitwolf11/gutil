# Linux x86-64 system identity comparison

Compare gutil commit `031031bda6ff45f0c3e8a930ff3ecfa0bcf9b243` with GNU
Coreutils 9.12. The kernel workload runs `uname` without arguments, and
the labeled workload runs `uname -A`. The machine workload compares
`gutil arch` with GNU `uname -m`, which reports the same field. This
machine does not have a separate GNU `arch` executable.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 4 logical CPUs, pinned to CPU 3 |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Compiler | rustc 1.100.0 with LLVM 23.1.5, default release profile |
| Filesystem | Executables on ext4 |
| Reference | GNU Coreutils uname 9.12 |

Each workload runs 100 measurements per executable after 10 warmups, with
seeded interleaving. Timing includes process startup, execution, captured
output, and exit. Outside the timed interval, the harness checks output
equality, empty stderr, and successful exit. Both commands run with
`LC_ALL=C` and `TZ=UTC`. gutil uses `--no-config --progress=never`.

| Workload | gutil median (ms) | GNU median (ms) | Paired reduction vs. GNU (median and IQR) |
| --- | ---: | ---: | ---: |
| kernel | 2.193 | 1.127 | -87.8% [-102.0%, -75.6%] |
| labeled | 2.161 | 1.271 | -83.2% [-97.3%, -63.5%] |
| machine | 2.092 | 1.194 | -83.7% [-97.3%, -64.8%] |

GNU has the lowest median in all three workloads. The measurement includes
startup and cannot distinguish loader overhead from application processing.
The raw samples and hash records identify the executable used for each result.

[Raw measurements and binary hashes](2026-11-20-linux-x86-64-system-identity.json)

```sh
cargo build --locked --release -p gutil
taskset -c 3 python3 benchmarks/identity_bench.py target/release/gutil --gnu /usr/bin/uname
```
