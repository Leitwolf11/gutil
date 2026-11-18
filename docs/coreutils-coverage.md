# Coreutils coverage

The baseline is GNU Coreutils 9.12. Its upstream program list contains 108
standalone command names after excluding the multicall wrapper `coreutils`
and the shared library `libstdbuf.so`. All 108 names are implementation targets.
The baseline includes standalone aliases and platform-dependent commands.
gutil's `cache` and `plug-in` commands are outside this total.
The commands implemented below account for 31 of the 108 target names.
[GNU Coreutils 9.12 program list](https://github.com/coreutils/coreutils/blob/v9.12/build-aux/gen-lists-of-programs.sh)

| Group | Commands | Status |
| --- | --- | --- |
| Existing utilities | `cat`, `ls`, `pwd` | Implemented and benchmarked against GNU. |
| Checksums | `md5sum`, `sha1sum`, `sha224sum`, `sha256sum`, `sha384sum`, `sha512sum` | Implemented and compared against GNU for multi-file checksum generation. |
| Stream selection and counts | `head`, `tail`, `wc` | Implemented and compared against GNU for regular files and a pipe. |
| Additional text and streams | `comm`, `csplit`, `cut`, `expand`, `fold`, `nl`, `od`, `paste`, `sort`, `split`, `tac`, `tee`, `tr`, `unexpand`, `uniq` | Planned. |
| File names and symbolic links | `basename`, `dirname`, `readlink`, `realpath` | Implemented and benchmarked against GNU. |
| Filesystem capacity | `df` | Implemented and benchmarked against GNU. |
| Links and filesystem changes | `link`, `ln`, `rmdir`, `truncate`, `unlink` | Implemented and benchmarked against GNU for hard links, removal, and file sizes. |
| Files and directories | `cp`, `du`, `mkdir`, `mktemp`, `mv`, `rm`, `stat`, `touch` | Planned. |
| Shell and system utilities | `echo`, `false`, `nproc`, `printenv`, `sleep`, `true`, `yes` | Implemented and benchmarked against GNU. |
| Base encoding | `base32`, `base64` | Implemented and benchmarked against GNU for encoding and wrapped decoding. |
| Additional shell utilities | `env`, `printf`, `seq` | Planned. |

The command-line reference documents supported behavior and intentional
differences. The benchmark reports identify the machine and workload for each
comparison. [Command-line reference](cli.md) · [Benchmark reports](benchmarking.md)
