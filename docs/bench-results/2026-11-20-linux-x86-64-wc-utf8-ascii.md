# Linux x86-64 ASCII blocks in UTF-8 word counting

Compare two gutil release builds with GNU Coreutils 9.12 using `LC_ALL=C.UTF-8`.
The baseline (`d5ad258`) includes the C-locale scan. The candidate
(`10e22b2`) also uses it for entirely ASCII reads without pending UTF-8 bytes.
The two ASCII workloads use `wc -w` and `wc -wm` on the same 16 MiB regular
file. A control uses `wc -w` on a 16 MiB file with repeated Latin and Japanese
characters.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 4 logical CPUs, pinned to logical CPU 3 |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Compiler | rustc 1.100.0 with LLVM 23.1.5, default release profile |
| Filesystems | Executables on ext4, fixtures on tmpfs |
| Reference | GNU Coreutils wc 9.12 |

Twelve measured rounds follow three warmup rounds, with the executables
interleaved in seeded order. Timing includes process startup, reading, and
output capture. The harness verifies the output and status of each run.
Both gutil builds use `--jobs=1 --no-config --progress=never`.

| Workload | Baseline ms | Candidate ms | GNU ms | Paired reduction vs. baseline (median and IQR) | Paired reduction vs. GNU (median and IQR) |
| --- | ---: | ---: | ---: | ---: | ---: |
| utf8-ascii-words | 227.31 | 23.20 | 165.59 | 90.5% [88.4%, 92.8%] | 87.3% [84.1%, 89.0%] |
| utf8-ascii-words-chars | 206.46 | 24.85 | 186.65 | 89.4% [87.1%, 90.7%] | 86.4% [85.5%, 88.3%] |
| utf8-mixed-words | 215.24 | 232.33 | 206.62 | -3.8% [-19.8%, 1.2%] | 5.6% [-14.3%, 11.6%] |

The candidate reduced paired median elapsed time by 89–91% against baseline
gutil and 86–87% against GNU on the ASCII file. The mixed fixture uses the
general UTF-8 decoder. Its candidate median is higher than the other two
executables, and both paired interquartile ranges cross zero. These samples
support a speedup for the tested ASCII work and leave the mixed case uncertain.

[Raw observations and binary hashes](2026-11-20-linux-x86-64-wc-utf8-ascii.json)

```sh
taskset -c 3 python3 benchmarks/wc_words.py target/experiments/wc-utf8-after \
  --baseline target/experiments/wc-utf8-before --gnu /usr/bin/wc --utf8
```
