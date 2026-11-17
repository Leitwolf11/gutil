# Coreutils coverage

The baseline is GNU Coreutils 9.12. Its upstream program list contains 108
standalone command names after excluding the multicall wrapper `coreutils`
and the shared library `libstdbuf.so`. The halfway target is 54 commands.
The baseline includes standalone aliases and platform-dependent commands.
gutil's `cache` and `plug-in` commands are outside this total.
[GNU Coreutils 9.12 program list](https://github.com/coreutils/coreutils/blob/v9.12/build-aux/gen-lists-of-programs.sh)

| Group | Commands | Status |
| --- | --- | --- |
| Existing utilities | `cat`, `ls`, `pwd` | Implemented. `ls` has a recorded comparison. |
| Checksums | `md5sum`, `sha1sum`, `sha224sum`, `sha256sum`, `sha384sum`, `sha512sum` | Implemented and compared against GNU for multi-file checksum generation. |
| Text and streams | `comm`, `csplit`, `cut`, `expand`, `fold`, `head`, `nl`, `od`, `paste`, `sort`, `split`, `tac`, `tail`, `tee`, `tr`, `unexpand`, `uniq`, `wc` | Planned. |
| Files and paths | `basename`, `cp`, `dirname`, `du`, `link`, `ln`, `mkdir`, `mktemp`, `mv`, `readlink`, `realpath`, `rm`, `rmdir`, `stat`, `touch`, `truncate`, `unlink` | Planned. |
| Encoding and shell utilities | `base32`, `base64`, `echo`, `env`, `nproc`, `printenv`, `printf`, `seq`, `sleep`, `yes` | Planned. |

The command-line reference documents supported behavior and intentional
differences. The benchmark reports identify the machine and workload for each
comparison. [Command-line reference](cli.md) · [Benchmark reports](benchmarking.md)
