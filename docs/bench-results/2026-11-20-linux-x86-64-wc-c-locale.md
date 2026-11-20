# Linux x86-64 C-locale word counting comparison

Compare gutil baseline `92c90c0`, candidate
`d5ad2582e3713614307f48248b0298229add068b`, and GNU Coreutils 9.12
on a warm 16 MiB regular file. The fixture repeats an ASCII sentence with
tabs, vertical tabs, spaces, and newlines. The harness measures `wc -w`
and the default line, word, and byte counts with `LC_ALL=C`. A third case
runs `wc -w` under `LC_ALL=C.UTF-8` on the same fixture as a control.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 4 logical CPUs, pinned to logical CPU 3 |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Compiler | rustc 1.100.0 with LLVM 23.1.5, default release profile |
| Filesystems | Executables on ext4, fixture on tmpfs |
| Reference | GNU Coreutils wc 9.12 |

Twelve measured rounds follow three warmup rounds. Each round interleaves
all three executables in seeded random order. The harness checks output
equality, success, and empty stderr outside the elapsed-time interval.
The interval includes process startup, reading the file, and collecting
the result. Both gutil builds use `--jobs=1 --no-config --progress=never`.

| Workload | Baseline ms | Candidate ms | GNU ms | Paired reduction vs. baseline (median and IQR) | Paired reduction vs. GNU (median and IQR) |
| --- | ---: | ---: | ---: | ---: | ---: |
| words | 43.09 | 8.93 | 59.26 | 79.0% [78.0%, 80.8%] | 85.3% [84.6%, 86.1%] |
| default | 44.22 | 10.11 | 58.88 | 77.0% [76.1%, 78.6%] | 83.2% [82.5%, 83.3%] |
| utf8-words | 95.61 | 100.29 | 86.44 | -4.7% [-6.2%, -3.9%] | -15.9% [-17.6%, -14.7%] |

On this host, the candidate reduced paired median elapsed time by about
77–79% compared with the previous gutil and 83–85% compared with GNU for
the C-locale workloads. The UTF-8 path retained its general character
counter. Its control remained slower than GNU, and these measurements
support a performance claim for the C-locale fixture only.

[Raw observations and binary hashes](2026-11-20-linux-x86-64-wc-c-locale.json)

```sh
# Save release binaries from the two revisions before running the harness.
taskset -c 3 python3 benchmarks/wc_words.py target/release/gutil \
  --baseline target/experiments/wc-c-baseline --gnu /usr/bin/wc
```
