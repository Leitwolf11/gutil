# Linux x86-64 mkdir permission-hardening comparison

Measured on November 19, 2026, in Europe/Istanbul, against GNU Coreutils
`mkdir` 9.12. Compare saved gutil 0.16.0 baseline `3daf85f` with candidate
`9573915`. Both versions use an ordered asynchronous filesystem worker and
adaptive handling of existing `-p` operands. The candidate includes Wasmi
2.0.2 and the mkdir review fixes: descriptor-relative permission adjustment,
permission completion before verbose output, and early trailing-dot
classification. Merge commit `5942b56` has identical production sources,
manifest, and lockfile to the candidate.

Explicit modes keep an open parent descriptor from creation through metadata
and permission adjustment. Inherited-mode creation has no later chmod and
uses a single `mkdirat` without an unused parent open. An exploratory version
that opened parents even without permission adjustment increased nested-path
elapsed time; that version is not the candidate reported here.

## Machine and tools

| Item | Recorded value |
| --- | --- |
| CPU | Intel Core i5-3210M at 2.50 GHz; 2 physical cores, 4 logical CPUs |
| CPU budget | Logical CPU 3 for the Python harness and every child process |
| Frequency policy | `intel_cpufreq`, `schedutil` |
| RAM | 7.61 GiB visible to Linux |
| OS | Arch Linux, x86-64 |
| Kernel | `7.3.4-zen2-2-zen` |
| Filesystems | ext4 executable storage; tmpfs fixtures under `/tmp` |
| Rust | rustc 1.100.0, LLVM 23.1.5, `x86_64-unknown-linux-gnu` |
| Python | 3.14.10, standard library |
| GNU | Coreutils 9.12 |
| Build profile | Default optimized Cargo release profile; no custom LTO or linker flags |

## Compared workloads and method

Each process receives 512 or 4096 sibling operands. The depth-four workload
instead receives 128 or 1024 paths and creates the same total number of
directories. All children have `LC_ALL=C`, `TZ=UTC`, umask 022, and logical CPU
3 affinity. Both gutil executables use
`--no-config --progress=never --jobs=1 mkdir`; GNU uses `mkdir`.

| Workload | Arguments | Expected result |
| --- | --- | --- |
| `siblings` | `d0000 d0001 ...` | New sibling directories, mode 755 |
| `siblings-mode-750` | `-m750 d0000 d0001 ...` | New sibling directories, mode 750 |
| `parents-depth-four` | `-p p0000/a/b/leaf ...` | Four new directories per operand, mode 755 |
| `existing-parents` | `-p d0000 d0001 ...` | Preexisting directories retain mode 755 |

Each timed interval includes process startup, directory work, and waiting for
exit. Fixture creation, cleanup, and verification of the complete tree and
every directory mode occur outside the interval. Nonzero status or stderr
fails the harness. Four warmup rounds precede 30 measured rounds for each
workload. Seed 42 shuffles the three executables within each round. Immutable
saved release executables are identified by SHA-256 in the raw results.
No local compilation or test suite ran during measurement; ordinary desktop
background activity remained possible.

```sh
taskset -c 3 python3 benchmarks/mkdir.py /tmp/gutil-mkdir-inherited \
  --before /tmp/gutil-mkdir-3daf85f --gnu /usr/bin/mkdir \
  --count 512 --rounds 30 --warmup 4
taskset -c 3 python3 benchmarks/mkdir.py /tmp/gutil-mkdir-inherited \
  --before /tmp/gutil-mkdir-3daf85f --gnu /usr/bin/mkdir \
  --count 4096 --rounds 30 --warmup 4
```

## Elapsed-time results

Values are median milliseconds with inclusive interquartile ranges. Paired
reductions use `100 * (reference - candidate) / reference` within each round.
Positive reductions mean lower candidate elapsed time; negative reductions
mean higher elapsed time. IQRs describe spread rather than confidence bounds.

| Directories / workload | Before ms [IQR] | Candidate ms [IQR] | GNU ms [IQR] | Before reduction % [IQR] | GNU reduction % [IQR] |
| --- | ---: | ---: | ---: | ---: | ---: |
| 512 / siblings | 4.68 [4.44–5.20] | 4.94 [4.57–5.64] | 3.44 [3.12–3.85] | -8.64 [-19.21–7.37] | -49.54 [-60.87–-29.01] |
| 512 / siblings-mode-750 | 5.42 [4.98–5.94] | 5.51 [5.08–5.85] | 5.11 [4.72–5.53] | -0.48 [-13.50–6.94] | -5.50 [-19.21–5.86] |
| 512 / parents-depth-four | 5.34 [4.81–5.93] | 5.10 [4.72–5.49] | 4.20 [3.93–4.81] | 6.37 [-9.34–16.87] | -18.46 [-25.73–-9.37] |
| 512 / existing-parents | 3.39 [3.22–3.53] | 3.71 [3.34–3.90] | 2.25 [2.10–2.53] | -9.90 [-17.29–3.04] | -66.13 [-82.34–-37.82] |
| 4096 / siblings | 18.94 [18.33–19.54] | 19.05 [18.20–19.88] | 16.64 [16.21–17.12] | -0.83 [-5.24–4.88] | -16.13 [-19.70–-8.19] |
| 4096 / siblings-mode-750 | 37.01 [33.36–39.78] | 34.20 [31.29–38.89] | 42.31 [36.77–46.80] | 2.98 [-7.53–18.83] | 14.97 [2.59–30.38] |
| 4096 / parents-depth-four | 35.89 [32.27–43.32] | 33.46 [31.79–36.11] | 36.64 [33.71–38.49] | 3.90 [-16.74–22.21] | 7.77 [-3.31–17.62] |
| 4096 / existing-parents | 12.91 [9.36–16.66] | 15.40 [8.72–17.07] | 14.72 [9.67–17.65] | -1.87 [-31.76–15.77] | 0.26 [-18.27–15.03] |

GNU has lower median elapsed time for every 512-directory workload and for
ordinary sibling creation at 4096 directories. At 4096 directories, explicit
mode creation has a median paired reduction against GNU of 15.0%; nested
parent creation has a 7.8% reduction, with an IQR that crosses zero. The
existing-directory paired reduction is 0.3% with a broad IQR and does not
establish an advantage. A median paired reduction is computed from individual
rounds, so it can differ from a ratio of the aggregate elapsed-time medians.

Before/after IQRs cross zero in every workload. Some candidate medians regress
against the pre-hardening baseline, including small sibling and existing-path
cases. These samples do not establish a general performance improvement from
the correctness fixes. Sample spread is substantially wider than in the
initial report; the measurements do not isolate scheduling, background load,
or frequency effects. Preserve those limits when interpreting the larger
positive reductions. Startup and ordinary-creation gaps remain open work.

## Scope and evidence

The fixtures are empty, warm directories on tmpfs without default ACLs. These
results do not establish physical-storage throughput or performance on Windows
or macOS. No additional durability synchronization is requested. Separate
regression tests cover default ACLs, inherited setgid, symbolic modes, creation
order, ancestor substitution, final symlink replacement, trailing-dot paths,
and verbose output failures after creation. Native CI covers Linux, Windows,
Intel macOS, and Apple Silicon macOS.

This is a follow-up to the [initial mkdir comparison](2026-11-19-linux-x86-64-mkdir.md).
The baseline here already includes that report's adaptive existing-directory
optimization, so before/after reductions refer to different revisions. The
complete Coreutils implementation and project-wide performance target remain
unfinished.

[512-directory raw samples and binary identities](2026-11-19-linux-x86-64-mkdir-permissions-512.json)
· [4096-directory raw samples and binary identities](2026-11-19-linux-x86-64-mkdir-permissions-4096.json)
