# Benchmarking

The Criterion `ls` benchmark compares end-to-end command latency on a directory containing 1,000 regular files. It runs a name-only listing with `-1` and a long listing with `-la`. Both commands use the same fixture and the C locale. gutil runs with `--no-config` so user and project configuration cannot affect measurements. Each timed iteration starts a process, discards standard output, and waits for the process to exit. The fixture is created before timing begins.

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
- [2026-11-17 · Linux x86-64 · checksums](bench-results/2026-11-17-linux-x86-64-checksum.md)
- [2026-11-17 · Linux x86-64 · streams and paths](bench-results/2026-11-17-linux-x86-64-streams.md)
- [2026-11-17 · Linux x86-64 · file names and symbolic links](bench-results/2026-11-17-linux-x86-64-paths.md)
- [2026-11-17 · Linux x86-64 · shell and system utilities](bench-results/2026-11-17-linux-x86-64-shell.md)
- [2026-11-17 · Linux x86-64 · file search](bench-results/2026-11-17-linux-x86-64-fd.md)
- [2026-11-17 · Linux x86-64 · mounted filesystems](bench-results/2026-11-17-linux-x86-64-disk.md)
- [2026-11-18 · Linux x86-64 · base encodings](bench-results/2026-11-18-linux-x86-64-encoding.md)
- [2026-11-18 · Linux x86-64 · filesystem mutations](bench-results/2026-11-18-linux-x86-64-mutate.md)

## Checksum throughput

From the repository root, use a directory containing the GNU `md5sum`,
`sha1sum`, `sha224sum`, `sha256sum`, `sha384sum`, and `sha512sum` binaries
to compare all checksum commands.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench checksum
```

Criterion measures three invocations per algorithm over the same four 16 MiB
files. The gutil runs use one and four workers. The GNU run uses the standalone
utility. Each timed iteration includes process startup and discards stdout.
The harness compares the emitted checksum records byte for byte before timing.
Omit `GNU_COREUTILS_DIR` to compare the gutil worker settings by themselves.

## Stream and path commands

Set `GNU_COREUTILS_DIR` to the directory containing GNU `head`, `tail`,
`wc`, `cat`, and `pwd` to compare all stream and path workloads.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench streams -- --noplot
```

The fixture is a 64 MiB file of fixed-length lines. The harness compares
stdout byte for byte before timing each GNU pair. Timed iterations include
process startup and discard stdout. The piped `tail` workload includes
copying the file into the child process's stdin. The `pwd` workload runs
from the fixture directory and reads no fixture data.

## Path commands

Set `GNU_COREUTILS_DIR` to the directory containing GNU `basename`,
`dirname`, `readlink`, and `realpath`.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench paths -- --noplot
```

Each release iteration passes 512 names to one process and discards stdout.
The lexical commands receive regular-file names, and `realpath` resolves
those paths on disk. On Unix, `readlink` reads 512 symbolic link targets.
The harness checks byte-for-byte output parity and measures one and four
gutil workers for filesystem lookups.

## Shell and system utilities

Set `GNU_COREUTILS_DIR` to the directory containing GNU `true`, `false`,
`echo`, `printenv`, `nproc`, `sleep`, and `yes`.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench shell -- --noplot
```

The harness checks output and exit status for finite commands and the first
8,192 output bytes from `yes`. Timed pipe runs drain 16 MiB before closing
the reader. The report records the exact operands and environment.

## File search

Set `REFERENCE_FD` to the upstream fd executable to compare ignore-aware
file search with one and four workers.

```sh
TMPDIR=/tmp REFERENCE_FD=/usr/bin/fd cargo bench --locked -p gutil --bench fd -- --noplot
```

Both implementations search the same tree and apply the same regular
expression, extension, file type, and ignore rules. The harness verifies
the matched path set before timing.

## Mounted filesystem reporting

Set `GNU_COREUTILS_DIR` to a directory containing GNU `df` and
`REFERENCE_DUF` to an upstream duf executable.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin REFERENCE_DUF=/usr/bin/duf cargo bench --locked -p gutil --bench disk -- --noplot
```

The harness compares one file operand and 512 repeated operands with
GNU `df`. A separate workload measures the full mount inventory as JSON.
It compares the stable capacity fields on mounts both implementations report.

## Base encoding

Set `GNU_COREUTILS_DIR` to a directory containing GNU `base32` and `base64`.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench encoding -- --noplot
```

The harness compares wrapped encoding, unwrapped encoding, and wrapped
decoding over an 8 MiB binary fixture. It checks output against GNU before
timing each pair and includes process startup in the measured time.

Run the generated encoding parity suite with GNU Coreutils 9.12 binaries.
The test uses a deterministic proptest generator and compares stdout and
exit status for 294 encoding and decoding cases, including malformed data.
It requires an explicit opt-in because the GNU version is part of the
compatibility baseline.

```sh
GNU_COREUTILS_DIR=/usr/bin cargo test --locked -p gutil --test encoding generated_cases_match_gnu_encoding_and_decoding -- --ignored
```

## Filesystem mutations

Set `GNU_COREUTILS_DIR` to the directory containing GNU `link`, `ln`,
`unlink`, `rmdir`, and `truncate`.

```sh
TMPDIR=/tmp GNU_COREUTILS_DIR=/usr/bin cargo bench --locked -p gutil --bench mutate -- --noplot
```

The harness measures hard-link creation, unlinking, 512 empty-directory
removals, and absolute resizing of one or 512 existing files. Each iteration
receives a fresh fixture. Fixture creation and cleanup are outside the
measured child-process interval. Verification checks the resulting content,
file lengths, directory removals, and hard-link identity before timing.

On Unix, run the reproducible state comparison against GNU Coreutils 9.12.
It compares exit codes, stdout, directory trees, link text, file contents,
and hard-link relationships across 91 cases.

```sh
GNU_COREUTILS_DIR=/usr/bin cargo test --locked -p gutil --test mutate filesystem_state_and_exit_codes_match_gnu -- --ignored
```
