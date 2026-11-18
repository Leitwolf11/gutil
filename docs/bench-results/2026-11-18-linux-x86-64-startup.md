# Linux x86-64 lazy worker discovery comparison

Measured on November 18, 2026, in Europe/Istanbul. The baseline is
`4aad3dfda0aba55c064cdccf0c386fe529c3f5f7`. The candidate is
`10a552d3637ad88d3c94eca0d3963881d8b998c5`. Both declare gutil 0.13.0
and include the AWS-LC checksum improvement. GNU Coreutils is version 9.12.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz, 2 physical cores, 4 logical CPUs |
| CPU affinity | Logical CPUs 0–3 available to the harness and children |
| Frequency policy | `intel_cpufreq`, `schedutil` |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 checkout and executables, tmpfs fixtures under `/tmp` |
| Rust | rustc 1.100.0, Cargo 1.100.0, `x86_64-unknown-linux-gnu` |
| Tools | Criterion 0.8.2 and strace 7.3 |
| Build profile | Default optimized release/bench profile, without LTO or custom linker flags |

## Change and syscall evidence

The baseline resolves the automatic filesystem worker count before dispatch,
including for shell commands that do not use that limit. On this host, the
lookup reads the process's cgroup membership and searches ancestor quota
files. The candidate defers discovery with `LazyCell`. A command that receives
the worker count still resolves it, and verbose settings diagnostics resolve
it once even for a shell command.

The following traces run `true` with `--no-config --progress=never`. The fixed
control additionally passes `--jobs=1`. Counts include process initialization
and exit. No shell timing instrumentation or loader debug flags are enabled.

| Variant | Total syscalls | `/proc/self/cgroup` opens | `cpu.max` lookups |
| --- | ---: | ---: | ---: |
| Baseline, automatic workers | 99 | 1 | 6 |
| Candidate, automatic workers | 65 | 0 | 0 |
| Baseline, explicit one worker | 65 | 0 | 0 |
| Candidate, explicit one worker | 65 | 0 | 0 |

An additional trace with a TOML configuration containing
`[diagnostics]` and `verbosity = "verbose"` records one cgroup-membership
open in each build. Both report `jobs: 4`. The optimization preserves the
configured-worker diagnostic and removes the lookup when the count is unused.

## Workload and timing

The Criterion shell harness compares command output before timing. Every
iteration starts a process, discards stdout, and waits for its exit. The
`false` command must return status 1. The other finite commands return 0.
The `yes` case drains 16 MiB through a pipe and checks its repeated-line
prefix before timing. Input setup and verification occur outside timing.

| Workload | Operands |
| --- | --- |
| `true` and `false` | No operands |
| `echo` | `-e 'hello\tworld' '\x21'` |
| `printenv` | `-0 GUTIL_BENCH_A GUTIL_BENCH_B` |
| `nproc` | `--ignore=1` |
| `sleep` | `0.002s` |
| `yes-pipe` | No operands, 16 MiB drained by the parent |

Processes use `LC_ALL=C`, `TZ=UTC`, `OMP_NUM_THREADS=4`,
`OMP_THREAD_LIMIT=3`, `GUTIL_BENCH_A=alpha`, and `GUTIL_BENCH_B=beta` followed
by a newline and `gamma`. `POSIXLY_CORRECT` is removed. gutil receives
`--no-config --progress=never`, leaving worker selection automatic except
for the explicit one-worker control.

Each variant uses 10 samples and a 400 ms warmup. The measurement target is
one second for finite commands and two seconds for `yes-pipe`. The baseline
and candidate runs execute consecutively on a shared desktop. Values below
are mean milliseconds with 95% confidence intervals. GNU values come from
the candidate run.

| Workload | Before | After | GNU |
| --- | ---: | ---: | ---: |
| `true` | 2.070 [1.826–2.442] | 1.853 [1.736–1.983] | 0.802 [0.742–0.864] |
| `false` | 1.931 [1.873–1.987] | 1.769 [1.663–1.886] | 0.792 [0.732–0.851] |
| `echo` | 1.862 [1.755–1.981] | 1.786 [1.642–1.932] | 0.872 [0.805–0.944] |
| `printenv` | 2.110 [2.046–2.162] | 1.799 [1.707–1.893] | 1.338 [0.824–2.095] |
| `nproc` | 2.092 [2.004–2.159] | 1.848 [1.746–1.949] | 0.964 [0.927–1.007] |
| `sleep` | 5.329 [5.254–5.411] | 5.825 [5.352–6.472] | 3.369 [3.270–3.485] |
| `yes-pipe` | 4.563 [4.449–4.706] | 4.555 [4.500–4.612] | 4.079 [4.007–4.150] |

The explicit `--jobs=1 true` control changes from 1.663 ms
[1.569–1.759] to 1.808 ms [1.722–1.891]. Its unchanged syscall counts and
higher elapsed estimate illustrate the variation between consecutive runs.
The candidate has lower point estimates for `true`, `false`, `echo`,
`printenv`, and `nproc`, a higher `sleep` estimate, and a similar `yes-pipe`
estimate. Several intervals overlap. These timings do not establish an
improvement for every shell workload.

The trace confirms 34 fewer syscalls for an automatic-worker `true` on this
host. GNU retains lower elapsed-time point estimates for these workloads.
Rust runtime initialization still contributes to latency. Asynchronous
execution and rusty-rich presentation remain available in their applicable
modes, and further work is needed to establish a startup advantage over GNU.

## Reproduce

Save release executables from the baseline and candidate revisions under
the paths below, then run the candidate's shell harness for both builds.
The sample, warmup, and duration settings are encoded in that harness.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin GUTIL_BENCH_BIN=target/release/gutil-lazy-workers-before-20261118 cargo bench --locked -p gutil --bench shell -- --noplot --save-baseline eager-workers
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin GUTIL_BENCH_BIN=target/release/gutil-lazy-workers-after-20261118 cargo bench --locked -p gutil --bench shell -- --noplot --baseline eager-workers
strace -qq -o /tmp/gutil-worker-count.strace target/release/gutil-lazy-workers-after-20261118 --no-config --progress=never true
```

Binary SHA-256 identities:

- Before: `030f60803dead604c509172c935d1029f659693d50ac73935449f50a8a123324`
- After: `2845fe36f5caedb8502a104b0bf7efe9404a6d17c1f50de5442ded1ee29bf285`
