# Linux x86-64 checksum backend comparison

Measured on November 18, 2026, in Europe/Istanbul, against GNU Coreutils
9.12. The baseline is `7e08da71f366e8b660f22a5c49c938b3404a73c8`. The
candidate is `2075b52af6b89bd308a852f593d931ed8876b466`. Both builds declare
gutil 0.13.0. The candidate uses AWS-LC for the four SHA-2 commands and
increases the checksum read buffer from 8 KiB to 64 KiB. MD5, SHA-1, and
cache identities retain their previous providers.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 physical cores, 4 logical CPUs |
| Instruction sets | AVX and SSSE3 available, SHA and AVX2 unavailable |
| CPU affinity | Criterion uses logical CPUs 0–3. The paired run uses logical CPU 3. |
| Frequency policy | `intel_cpufreq`, `schedutil` |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 executables and checkout, tmpfs fixtures under `/tmp` |
| Rust | rustc 1.100.0, Cargo 1.100.0, `x86_64-unknown-linux-gnu` |
| Harnesses | Criterion 0.8.2 and Python 3.14 standard library |
| Build profile | Default optimized Cargo release/bench profile, without LTO or custom linker flags |

## Four-file Criterion comparison

Every command hashes the same four 16 MiB regular files. A 64 KiB block
whose byte at index `i` is `i % 251` repeats to fill each file. Files are
created before timing and reused after warmup. Each timed iteration starts
one process, discards stdout, and waits for exit. The harness compares the
one-worker, four-worker, and GNU checksum records byte for byte before
collecting samples. Processes use `LC_ALL=C` and `TZ=UTC`. gutil receives
`--no-config --progress=never` and either `--jobs=1` or `--jobs=4`.

The baseline and candidate runs each use 10 samples, a 400 ms warmup, and
a two-second measurement target. Criterion extends measurements for slow
commands. Values below are mean elapsed milliseconds with 95% confidence
intervals. The GNU column is from the candidate run.

| Command | Before, 1 worker | After, 1 worker | Before, 4 workers | After, 4 workers | GNU |
| --- | ---: | ---: | ---: | ---: | ---: |
| `md5sum` | 161.2 [160.4–162.1] | 171.8 [161.9–186.8] | 61.9 [59.5–64.7] | 61.8 [59.6–64.3] | 132.0 [128.3–136.4] |
| `sha1sum` | 175.1 [173.3–176.7] | 183.7 [177.3–192.5] | 100.6 [98.2–103.1] | 105.1 [101.7–108.8] | 139.2 [134.8–144.2] |
| `sha224sum` | 433.6 [416.8–452.4] | 276.1 [270.4–282.7] | 250.6 [246.0–256.7] | 142.4 [136.4–149.3] | 266.1 [261.1–271.0] |
| `sha256sum` | 409.7 [405.5–413.6] | 268.8 [258.6–281.9] | 247.7 [244.2–253.0] | 148.3 [131.2–174.4] | 261.1 [256.9–265.2] |
| `sha384sum` | 314.5 [292.8–337.9] | 198.1 [188.4–214.5] | 181.3 [173.3–189.8] | 103.3 [100.9–106.1] | 189.1 [187.1–191.2] |
| `sha512sum` | 278.9 [264.5–301.0] | 200.9 [190.6–213.7] | 157.0 [148.5–167.4] | 108.2 [100.4–120.0] | 211.5 [197.2–230.2] |

The SHA-2 one-worker point estimates fall by 28.0–37.0%, and four-worker
estimates fall by 31.1–43.2%. Four-worker candidate point estimates are
lower than GNU for all six commands, using additional CPU resources.
The single-worker candidate remains slower than GNU for MD5 and SHA-1.
The SHA-2 single-worker comparisons are close, and their confidence
intervals overlap with GNU for all four algorithms.

The baseline and candidate runs execute consecutively on a shared desktop.
For example, the GNU SHA-512 mean changes from 193.6 ms in the baseline
run to 211.5 ms in the candidate run. The higher candidate MD5/SHA-1
one-worker estimates therefore need an interleaved check before attributing
them to the implementation.

## Interleaved single-file comparison

A second run hashes one 16 MiB file containing bytes 0 through 255 repeated
65,536 times. The Python harness checks every emitted digest against
`hashlib` before timing. It includes child creation and waiting, discards
stdout, and captures stderr. It uses the same locale, timezone, configuration,
progress, and one-worker options as the Criterion run.

Four warmup rounds precede 50 measured rounds for each algorithm. Each
round shuffles the baseline, candidate, and GNU invocation order with
Python's random seed 42. The harness and children run on logical CPU 3.
The elapsed columns contain median milliseconds and interquartile ranges.
The reduction column summarizes the per-round expression
`100 * (before - after) / before`, including its interquartile range.

| Command | Before | After | GNU | Paired reduction | Candidate wins |
| --- | ---: | ---: | ---: | ---: | ---: |
| `md5sum` | 41.828 [41.127–43.335] | 41.244 [40.579–42.966] | 33.585 [33.042–34.399] | 1.6% [-0.6–3.2%] | 35/50 |
| `sha1sum` | 47.239 [44.861–50.660] | 45.926 [44.073–48.595] | 33.602 [32.235–35.517] | 2.3% [-2.9–8.6%] | 31/50 |
| `sha224sum` | 105.397 [103.256–109.505] | 66.296 [64.589–68.916] | 66.527 [64.147–69.635] | 37.3% [36.1–38.7%] | 50/50 |
| `sha256sum` | 104.955 [101.960–113.505] | 66.295 [64.466–69.646] | 67.138 [65.351–70.135] | 36.8% [34.9–39.5%] | 50/50 |
| `sha384sum` | 66.068 [64.879–68.349] | 48.307 [46.929–50.288] | 48.352 [47.188–49.674] | 27.5% [25.0–29.1%] | 50/50 |
| `sha512sum` | 71.981 [67.453–79.369] | 51.389 [49.194–55.610] | 51.643 [49.409–55.392] | 28.0% [22.7–30.9%] | 49/50 |

The interleaved measurements support a SHA-2 improvement of 27.5–37.3%
on this workload. They do not show the MD5/SHA-1 regressions suggested by
the sequential Criterion point estimates. Candidate and GNU SHA-2
single-file medians are close. These results do not establish a general
single-worker performance advantage over GNU or measure process startup
in isolation. CPU frequency and scheduling still vary on this desktop.

## Reproduce

Save the release executable from each revision as the paths below. Run the
candidate's harness with the saved baseline executable, then with the
candidate executable. `GUTIL_BENCH_BIN` resolves relative paths before the
child enters its fixture directory.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin GUTIL_BENCH_BIN=target/release/gutil-checksum-before-20261118 cargo bench --locked -p gutil --bench checksum -- --noplot --save-baseline rustcrypto-before
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin GUTIL_BENCH_BIN=target/release/gutil-checksum-final-20261118 cargo bench --locked -p gutil --bench checksum -- --noplot --baseline rustcrypto-before
TMPDIR=/tmp taskset -c 3 python3 benchmarks/checksum.py target/release/gutil-checksum-before-20261118 target/release/gutil-checksum-final-20261118 --rounds 50
```

Binary SHA-256 identities:

- Before: `8d9a446e25f58c63e5be143369e36248a3250314c27fe8ad12ce005b4d2ca890`
- After: `b4e1a80efa34d29149a5d73b31e01afe202965bfb2553e0d6926d36227b3a4f3`
