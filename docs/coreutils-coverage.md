# Coreutils coverage

The target is every standalone command in GNU Coreutils 9.12. Its upstream
program list contains 108 command names after excluding the multicall
wrapper `coreutils` and the shared library `libstdbuf.so`. The upstream build
name `ginstall` corresponds to the installed command `install`. Standalone
aliases and platform-dependent commands remain part of the target.
[Upstream program list](https://github.com/coreutils/coreutils/blob/v9.12/build-aux/gen-lists-of-programs.sh)
· [Installed-name transformation](https://github.com/coreutils/coreutils/blob/v9.12/src/local.mk#L374-L384)

The default branch currently provides 35 of these 108 command names. The
count follows merged code. The table records available commands and the
remaining implementation work. Available commands implement the behavior
listed in the command reference, and their presence does not imply complete
GNU option coverage or a demonstrated performance advantage.

## Available commands

| Group | Commands | Evidence |
| --- | --- | --- |
| Directory listings | `ls` | Name-only and long-format benchmarks against GNU. |
| Streams and counts | `cat`, `head`, `tail`, `tee`, `wc` | Regular-file and pipe comparisons against GNU. |
| Adjacent duplicates | `uniq` | GNU output comparisons for adjacent runs, group separators, and binary input. |
| Paths | `basename`, `dirname`, `pwd`, `readlink`, `realpath` | Documented path behavior and GNU comparisons. |
| Checksums | `md5sum`, `sha1sum`, `sha224sum`, `sha256sum`, `sha384sum`, `sha512sum` | GNU output checks and multi-file benchmarks. |
| Base encoding | `base32`, `base64` | Encoding and wrapped-decoding comparisons against GNU. |
| Filesystem capacity | `df` | Mounted-filesystem comparisons against GNU. |
| Directory creation | `mkdir` | GNU mode, umask, inherited setgid, default ACL, filesystem-state, and status comparisons. |
| Temporary objects | `mktemp` | GNU template, suffix, temporary-directory precedence, dry-run, mode, and filesystem-state comparisons. |
| Links and filesystem changes | `link`, `ln`, `rmdir`, `truncate`, `unlink` | GNU filesystem-state and exit-status comparisons, failure recovery, and release benchmarks. |
| Shell and system utilities | `echo`, `false`, `nproc`, `printenv`, `sleep`, `true`, `yes` | Shell output, exit-status, and timing comparisons against GNU. |

## Remaining commands

These groups contain the remaining 73 names. A pull request may implement
one command or a small coherent subset of a group. The grouping does not
require a single pull request for every command in a row.

| Group | Commands |
| --- | --- |
| Text comparison and ordering | `comm`, `join`, `sort`, `tsort` |
| Text layout and fields | `cut`, `expand`, `fmt`, `fold`, `nl`, `paste`, `pr`, `ptx`, `tr`, `unexpand` |
| Stream transformation and splitting | `csplit`, `dd`, `od`, `split`, `tac` |
| File copying and removal | `cp`, `install`, `mv`, `rm`, `shred` |
| File and directory creation | `mkfifo`, `mknod`, `touch` |
| Metadata, ownership, and storage | `chgrp`, `chmod`, `chown`, `du`, `stat`, `sync` |
| Additional checksums and encodings | `b2sum`, `basenc`, `cksum`, `sum` |
| Arithmetic and formatted output | `expr`, `factor`, `numfmt`, `printf`, `seq`, `shuf` |
| Process and environment control | `[`, `chroot`, `env`, `kill`, `nice`, `nohup`, `pathchk`, `stdbuf`, `test`, `timeout` |
| Identity and login information | `groups`, `hostid`, `id`, `logname`, `pinky`, `users`, `who`, `whoami` |
| System, time, and terminal information | `arch`, `date`, `hostname`, `stty`, `tty`, `uname`, `uptime` |
| Security contexts | `chcon`, `runcon` |
| Listing aliases and color definitions | `dir`, `dircolors`, `vdir` |

## Delivery and performance

Continue through small, independently reviewable pull requests. Each command
needs typed argument handling, documented behavior, representative GNU
comparisons under a controlled locale, and checks on its supported platforms.
Platform-dependent operations need native behavior wherever the underlying
facility exists and tested diagnostics where it is unavailable. A placeholder
command does not establish implementation coverage.

The performance target is to surpass GNU on comparable workloads. Record
one-worker and parallel results where parallel execution applies, including
the resource budget used by each variant. Measure small inputs as well as
larger workloads so startup costs remain visible. A multi-worker result alone
does not establish a one-worker advantage. Claims must identify the measured
machine, filesystems, input, cache state, flags, and uncertainty. Keep
rusty-rich presentation and asynchronous execution part of the implementation
when measuring the applicable user-facing modes.

The CLI reference documents supported behavior and intentional differences.
Benchmark reports record the evidence and remaining performance gaps.
[Command-line reference](cli.md) · [Benchmark reports](benchmarking.md)

The additional `eza`, `duf`, and `fd` commands and gutil's `cache` and `plug-in`
commands are tracked separately from the 108-name Coreutils total.
[Additional commands](additional-commands.md) · [Plug-ins](plug-ins.md)
