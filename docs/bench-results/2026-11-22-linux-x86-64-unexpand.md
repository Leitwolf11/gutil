# Linux x86-64 blank-to-tab conversion

Compare release build gutil `3ccdc82` with GNU Coreutils 9.12 `unexpand`.
The workloads measure conversion of leading or interior blanks in
regular files, using default, explicit, and repeating tab positions.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 cores and 4 logical CPUs |
| Affinity | Harness and child processes pinned to logical CPU 3 |
| Memory | 7.61 GiB visible to Linux |
| System | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Toolchain | rustc 1.100.0 with LLVM 23.1.5, default release profile |
| Storage | Executables on ext4, input and output on tmpfs |

The three C-locale cases read the same 16,777,215-byte fixture with leading
indentation and interior runs of spaces. They use default leading-only
conversion, `-a`, or `-t3,11,+5`. The C.UTF-8 case reads 16,776,992 bytes of
accented Latin, Japanese, and emoji text with ASCII spaces and uses `-a`.
Each invocation writes a regular file. gutil uses
`--no-config --progress=never --jobs=1`.

Four warmups precede 12 measured rounds in seeded, interleaved order.
Timing includes process startup, reading, conversion, writing, and exit.
The harness verifies the exit status, empty stderr, and the full output
hash for every invocation. Output hashing occurs after timing.

| Workload | gutil median (ms) | GNU median (ms) | Paired reduction vs. GNU, median [IQR] |
| --- | ---: | ---: | ---: |
| ASCII, leading only | 15.35 | 296.95 | 94.9% [94.3%, 95.2%] |
| ASCII, all blanks | 79.33 | 531.66 | 84.9% [84.7%, 85.1%] |
| ASCII, custom stops | 105.47 | 535.96 | 80.1% [79.5%, 80.7%] |
| UTF-8, all blanks | 310.81 | 477.20 | 34.6% [33.7%, 36.1%] |

The paired median elapsed times were 35–95% lower than GNU's in these
warm-file workloads on the recorded Linux host.

[Raw observations and executable hashes](2026-11-22-linux-x86-64-unexpand.json)

```sh
cargo build --locked --release -p gutil --bin gutil
TMPDIR=/dev/shm taskset -c 3 python3 benchmarks/expand_bench.py target/release/gutil \
  --utility unexpand --gnu /usr/bin/unexpand
```
