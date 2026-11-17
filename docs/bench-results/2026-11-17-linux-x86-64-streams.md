# Linux x86-64 stream and path benchmark

Measured on November 17, 2026, in Europe/Istanbul. This run compares gutil
0.8.0 at commit `c9305ae5bab546233657f65d97f756ac63f9e2b7` with GNU Coreutils 9.12.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 physical cores, 4 logical CPUs |
| Available CPUs | All 4 logical CPUs, without isolation |
| Frequency policy | `intel_cpufreq` driver and `schedutil` governor |
| RAM | 7.61 GiB visible to Linux |
| Operating system | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 for the checkout and executable, tmpfs for the input under `/tmp` |
| Rust toolchain | rustc 1.100.0 and Cargo 1.100.0, host `x86_64-unknown-linux-gnu` |
| Harness | Criterion 0.8.2, optimized Cargo bench profile |

## Workload

The fixture is a 64 MiB regular file named `input` with 1,048,576
64-byte lines. Each line contains 63 ASCII `x` bytes and one newline.
All children run from the fixture directory with `LC_ALL=C` and `TZ=UTC`.
gutil runs with `--no-config --progress=never`. The harness confirms that
each pair emits byte-identical stdout before measuring. Timed iterations
include process startup, discard stdout, and wait for exit.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench streams -- --noplot
```

Criterion uses 10 samples, a 400 ms warmup, and a one-second measurement
target per invocation. `tail -n1000` on stdin includes a producer that opens
and copies the fixture into the process pipe. The same producer is used
for both implementations. `pwd -P` prints the fixture directory's physical
path without accessing the file. Input remains resident in tmpfs.

## Results

Criterion reports point estimates and 95% confidence intervals. All entries
are elapsed milliseconds.

| Workload | gutil estimate [95% CI] | GNU estimate [95% CI] |
| --- | ---: | ---: |
| `head -n1000 input` | 2.544 [2.282–3.037] | 1.458 [1.325–1.587] |
| `tail -n1000 input` | 2.198 [2.032–2.533] | 0.993 [0.959–1.051] |
| `tail -n1000` via pipe, baseline | 70.076 [63.699–76.631] | 30.427 [28.746–31.987] |
| `wc -l input` | 19.382 [18.967–19.830] | 20.252 [19.815–21.066] |
| `wc -c input` | 2.023 [1.930–2.083] | 1.184 [1.153–1.207] |
| `cat input` | 14.452 [14.175–14.661] | 4.939 [4.572–5.288] |
| `pwd -P` | 2.606 [2.267–3.154] | 1.407 [1.188–1.559] |

The point estimate for `wc -l` was 4.3% lower in gutil. Its confidence
interval overlapped GNU's. GNU had lower intervals on the other measured
workloads. The file and pipe forms of `tail` select the same bytes. The
regular-file form can seek to the suffix, while the pipe form consumes
the entire input.

## Paired test of reusable pipe buffers

The optimized binary at commit `b7c16055eac11e5647b85608cb929602c28cbc0a`
uses reusable blocks and an aggregate delimiter count instead of buffering
each line separately. Independent Criterion runs had different background
load. Across those runs the GNU pipe point estimate moved from 30.427 ms to
86.759 ms. The paired test below interleaves the baseline, optimized binary,
and GNU `tail` to reduce the effect of that variation.

The binaries used the optimized Cargo profile and the same fixture and
arguments as the Criterion pipe workload. A Python 3 producer piped the
fixture to each child with `shutil.copyfileobj` using 64 KiB chunks and directed
stdout to the null device. Wall time included process startup, the producer,
and the child exit. A seeded shuffle (42) changed the order of the three
variants in each round. The first five rounds warmed up the test. The
following 30 rounds supplied the results.

| Variant | Median elapsed time | Interquartile range |
| --- | ---: | ---: |
| Baseline gutil | 188.900 ms | 174.031–208.942 ms |
| Optimized gutil | 96.190 ms | 86.497–107.354 ms |
| GNU Coreutils 9.12 | 94.465 ms | 88.425–106.200 ms |

The optimized binary finished sooner than the baseline in all 30 paired
rounds. The median of the paired elapsed-time reductions was 47.3%, with
an interquartile range of 43.2–52.9%. The optimized gutil and GNU
interquartile ranges overlapped.
