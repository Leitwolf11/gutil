# Linux x86-64 mixed UTF-8 word and character counting

The candidate (`6304b5c`) specializes counting on decoded UTF-8 spans when
display width is unused. The baseline (`cdc2477`) already has the ASCII
block optimization. Both executables use the same locked dependencies and
release build profile. The reference is GNU Coreutils 9.12.

| Item | Environment |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 cores and 4 logical CPUs |
| CPU affinity | Harness and executables pinned to logical CPU 3 |
| Memory | 7.61 GiB visible to Linux |
| OS | Arch Linux x86-64, kernel 7.3.4-zen2-2-zen |
| Compiler | rustc 1.100.0 with LLVM 23.1.5, default release profile |
| Storage | Executables on ext4, warm fixtures on tmpfs |

Five cases process 16 MiB regular files in the `C.UTF-8` locale. The mixed
fixture repeats the line `café mundo<TAB>日本語 autre<LF>` and pads the
end if a partial UTF-8 sequence is removed. The ASCII control uses the same
fixture pattern as the earlier ASCII report. Each executable completes three
warmup rounds and 12 measured rounds in seeded, interleaved order. Elapsed
time includes process startup, reading, and output capture. Both gutil
commands use `--jobs=1 --no-config --progress=never`. The harness checks
identical stdout, empty stderr, and successful exit for every invocation.

| File and flags | Baseline ms | Candidate ms | GNU ms | Paired reduction vs. baseline, median [IQR] | Paired reduction vs. GNU, median [IQR] |
| --- | ---: | ---: | ---: | ---: | ---: |
| ASCII `-w` | 9.55 | 9.83 | 89.89 | 1.5% [-5.8%, 8.5%] | 89.0% [88.5%, 90.0%] |
| ASCII `-wm` | 9.50 | 9.26 | 88.05 | 1.5% [-4.5%, 13.5%] | 89.7% [89.0%, 90.3%] |
| Mixed `-w` | 111.96 | 67.25 | 110.04 | 40.3% [39.3%, 41.0%] | 38.4% [36.9%, 42.0%] |
| Mixed `-m` | 82.35 | 36.42 | 107.50 | 55.2% [54.4%, 56.1%] | 66.3% [65.4%, 67.1%] |
| Mixed `-wm` | 111.55 | 68.63 | 109.76 | 38.6% [37.2%, 39.4%] | 37.2% [36.2%, 38.5%] |

The mixed fixture shows paired median elapsed-time reductions of 37–66%
against GNU. The changes to the two ASCII controls lie within the observed
interquartile variability relative to baseline. Display-width counting,
other text distributions, and other platforms were outside this measurement.

[Raw observations and binary hashes](2026-11-22-linux-x86-64-wc-utf8-mixed.json)

```sh
env -u POSIXLY_CORRECT taskset -c 3 python3 benchmarks/wc_words.py \
  target/experiments/wc-utf8-mixed-candidate \
  --baseline target/experiments/wc-utf8-mixed-baseline --gnu /usr/bin/wc --utf8
```
