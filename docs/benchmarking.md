# Benchmarking

The Criterion `ls` benchmark compares end-to-end command latency on a directory containing 1,000 regular files. It runs a name-only listing with `-1` and a long listing with `-la`. Both commands use the same fixture and the C locale. Each timed iteration starts a process, discards standard output, and waits for the process to exit. The fixture is created before timing begins.

## Run the comparison

From the repository root, set `GNU_LS` to the absolute path of the GNU coreutils `ls` executable. The example uses `/usr/bin/ls` on a Linux system where that path points to GNU `ls`.

```sh
/usr/bin/ls --version
LC_ALL=C TZ=UTC TMPDIR=/tmp GNU_LS=/usr/bin/ls cargo bench --locked --bench ls
```

On a system without GNU `ls`, omit `GNU_LS` to measure gutil alone. Use the default Criterion sampling for recorded comparisons. Treat `--quick` runs as exploratory measurements.

## Record a result

Add a Markdown file under `docs/bench-results/` for each recorded run. Use the run date, platform, and workload in the filename. Each report should record the following information.

- Machine details, including the CPU model and topology, available parallelism, RAM, operating system, kernel, and fixture filesystem.
- The gutil version and commit, the comparison tool and version, and the Rust and Criterion versions.
- The exact benchmark command, fixture contents, compared invocations, and environment settings that affect the run.
- The latency estimates and confidence intervals, along with any output differences, cache conditions, or sources of run-to-run variation.

Compare timings from the same run. Long-format results include metadata formatting, which differs between gutil and GNU `ls`. Keep conclusions tied to the recorded workload and machine.

## Recorded results

- [2026-11-09 · Linux x86-64 · ls](bench-results/2026-11-09-linux-x86-64-ls.md)
