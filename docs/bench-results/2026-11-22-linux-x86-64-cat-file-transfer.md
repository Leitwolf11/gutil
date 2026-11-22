# Linux x86-64 plain cat regular-file transfer

The candidate (`7708c66`) uses Linux `sendfile` for plain copies of named
regular files. The baseline (`33792cb`) uses the buffered path for every
operand. Both builds have the same locked dependencies and default release
profile. The reference is GNU Coreutils 9.12.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 cores and 4 logical CPUs |
| Affinity | Harness and all children pinned to logical CPU 3 |
| Memory | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Compiler | rustc 1.100.0 with LLVM 23.1.5 |
| Storage | Executables on ext4, fixtures and regular output on tmpfs |

All cases copy the same warm 64 MiB input. It contains 1,048,576 records of
63 ASCII `x` bytes followed by a newline. The destination is the null
device, a freshly truncated regular file, or a pipe drained by the harness
with a reusable 128 KiB buffer. The harness checks the complete output hash
with the regular-file and pipe destinations before timing. Exit status and
stderr are checked on every run.

Each variant runs 4 warmups and 12 measured rounds per destination, with
seeded interleaving. Elapsed time includes process startup, copying, and
waiting for exit. The pipe case also includes the time to drain the pipe.
Processes use `LC_ALL=C` and `TZ=UTC`. gutil additionally receives
`--no-config --progress=never --jobs=1`.

| Destination | Baseline ms | Candidate ms | GNU ms | Paired reduction vs. baseline, median [IQR] | Paired reduction vs. GNU, median [IQR] |
| --- | ---: | ---: | ---: | ---: | ---: |
| Null device | 15.04 | 6.20 | 4.69 | 57.4% [56.2%, 60.2%] | -27.5% [-45.4%, -16.5%] |
| Regular file | 40.34 | 35.46 | 34.07 | 6.7% [3.0%, 14.9%] | -2.3% [-8.8%, 0.9%] |
| Pipe | 50.84 | 29.22 | 22.74 | 42.6% [38.2%, 44.4%] | -28.0% [-47.3%, -14.2%] |

The candidate improved all three paired medians relative to gutil's baseline.
GNU retains a measured advantage for the null device and pipe; the
regular-file comparison against GNU has an interquartile range spanning zero.
The report covers warm data and plain output with progress disabled.

[Raw observations and executable hashes](2026-11-22-linux-x86-64-cat-file-transfer.json)

```sh
taskset -c 3 python3 benchmarks/cat_file.py \
  target/experiments/cat-file-transfer-candidate \
  --baseline target/experiments/cat-file-transfer-baseline --gnu /usr/bin/cat
```
