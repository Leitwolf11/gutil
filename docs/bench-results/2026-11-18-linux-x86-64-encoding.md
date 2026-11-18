# Linux x86-64 base encoding benchmark

The initial run took place on November 17, 2026. The optimized run and
paired measurements took place on November 18, 2026, in Europe/Istanbul.
Both runs compare gutil 0.13.0 with GNU Coreutils 9.12.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 physical cores, 4 logical CPUs |
| Available CPUs | All 4 logical CPUs, without isolation |
| Frequency policy | `intel_cpufreq` driver, `schedutil` governor |
| RAM | 7.61 GiB visible to Linux |
| Operating system | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 checkout and executable, tmpfs fixtures under `/tmp` |
| Rust toolchain | rustc 1.100.0, Cargo 1.100.0, `x86_64-unknown-linux-gnu` |
| Harness | Criterion 0.8.2, optimized Cargo bench profile |
| Initial implementation | `516a395e43273bd5b58a9f985c9a4c2ccaed7708` |
| Optimized implementation | `ef4c5b921569fff68b6f58c11bf2e323c388ce18` |

## Workload

The input contains 8 MiB of the repeating byte sequence 0 through 255.
Each utility encodes it with the default 76-column wrapping and with
`-w0`. Decoding uses the default wrapped output. The harness verifies
byte-for-byte agreement with GNU before measuring each workload.

Each iteration starts a process, discards stdout, and waits for exit.
Processes use `LC_ALL=C`, `TZ=UTC`, and gutil additionally receives
`--no-config --progress=never`. Fixtures are prepared outside timing.
Criterion uses 10 samples, 400 ms warmup, and a one-second measurement
target. Data stays warm in the filesystem cache. The machine runs a
desktop session, with shared CPU resources and variable frequency.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench encoding -- --noplot
```

## Optimized Criterion results

Elapsed-time point estimates and 95% confidence intervals are in milliseconds.

| Workload | gutil | GNU |
| --- | ---: | ---: |
| base32, wrapped encode | 16.214 [15.470–16.840] | 18.845 [18.093–20.110] |
| base32, unwrapped encode | 14.208 [13.821–14.778] | 14.251 [13.953–14.503] |
| base32, wrapped decode | 21.232 [19.996–22.043] | 35.960 [35.525–36.460] |
| base64, wrapped encode | 14.415 [13.497–15.513] | 13.527 [13.008–14.226] |
| base64, unwrapped encode | 11.680 [11.385–12.275] | 10.705 [10.103–11.556] |
| base64, wrapped decode | 16.038 [15.692–16.678] | 42.252 [41.259–43.393] |

The initial wrapped-decode estimates were 97.880 ms [96.031–99.842]
for base32 and 110.190 ms [108.360–111.890] for base64. Line feeds
previously sent each input chunk through individual encoded quanta.
The optimized decoder removes line feeds into a reusable buffer and
completes a carried quantum before bulk decoding the remaining bytes.
Invalid data retains its validated prefix and error status.

## Paired decode comparison

A Python harness compares the saved initial and optimized release binaries
and GNU on the same wrapped fixtures. Every output is checked against the
original 8 MiB input. Four warmup rounds precede 30 measured rounds.
Invocation order is shuffled each round using Python's random seed 42.
Each timed invocation uses `-d encoded`, includes process startup, and
sends stdout to the null device. Values below are medians and
interquartile ranges in milliseconds.

| Workload | Initial gutil | Optimized gutil | GNU |
| --- | ---: | ---: | ---: |
| base32, wrapped decode | 99.044 [94.725–101.642] | 21.116 [18.059–22.526] | 43.346 [37.628–49.809] |
| base64, wrapped decode | 107.660 [103.534–110.675] | 16.169 [15.424–19.385] | 49.076 [41.795–55.219] |

Both optimized decoders finish sooner in all 30 paired rounds. The
median paired elapsed-time reduction is 79.4% [77.6–81.4% interquartile]
for base32 and 84.7% [82.2–85.6% interquartile] for base64.
These results apply to the recorded input, machine, and cache conditions.
Encoding performance varies by alphabet and wrapping, as shown above.
