# Command-line reference

gutil runs utilities as subcommands of a single executable.

```sh
gutil --help
gutil --version
gutil ls --help
gutil cat --help
gutil pwd --help
gutil ls -la .
```

`gutil --help` lists the available subcommands and global options. Use `gutil <COMMAND> --help` for command-specific syntax and options.

Global settings can be supplied by a TOML file, a named profile, or command-line options. [Configuration and profiles](configuration.md)

## Filesystem jobs

Set the limit on concurrent filesystem jobs with `-j N`, `--jobs N`, or `--threads N`. The option works before or after the subcommand. The following commands use the same limit.

```sh
gutil -j 4 ls -la .
gutil ls --jobs=4 -la .
gutil ls -la --threads 4 .
```

With the built-in settings, the default is the detected parallelism, capped at eight. Set the limit to one to run filesystem work serially. Positive job counts above 512 use 512. Concurrent jobs preserve the output order.

## ls

`gutil ls [OPTION]... [FILE]...` lists the current directory when no path is supplied. It accepts multiple file and directory operands. Output uses one entry per line, sorted by the native filename representation. With mixed operands, files appear before directory sections. Each directory is listed separately.

| Option | Behavior |
| --- | --- |
| `-a`, `--all` | Include entries beginning with a dot, including `.` and `..`. |
| `-A`, `--almost-all` | Include entries beginning with a dot, except `.` and `..`. |
| `-d`, `--directory` | List directory operands themselves. |
| `-l` | Show metadata and symbolic link targets in long format. |
| `-1` | Use one entry per line, the default layout. |
| `--` | Treat subsequent arguments as paths, including names beginning with a hyphen. |

Long format uses UTC timestamps. On Unix, it reports numeric owner and group IDs. On Windows, it reports basic read-only information and uses `-` for unavailable ownership fields. On all platforms, a leading dot identifies a hidden entry.

`ls` exits with status `0` on success, `1` for minor listing errors, and `2` for invalid options or inaccessible operands. When one operand fails, `ls` continues processing the remaining operands.

## cat

`gutil cat [OPTION]... [FILE]...` copies operands to standard output in the order supplied. With no operands, or for each `-` operand, it reads standard input. Numbering and formatting carry across operand boundaries.

```sh
gutil cat first.txt - last.txt
gutil cat --number -- report.txt
```

| Option | Behavior |
| --- | --- |
| `-n`, `--number` | Number all output lines. |
| `-b`, `--number-nonblank` | Number nonempty lines. Overrides `-n`. |
| `-s`, `--squeeze-blank` | Collapse adjacent empty lines into one. |
| `-E`, `--show-ends` | Mark line endings with `$` and show CRLF as `^M$`. |
| `-T`, `--show-tabs` | Show tabs as `^I`. |
| `-v`, `--show-nonprinting` | Show control bytes with caret notation and high-bit bytes with an `M-` prefix. Tabs and line feeds retain their usual meaning. |
| `-A`, `--show-all` | Combine `-v`, `-E`, and `-T`. |
| `-e` | Combine `-v` and `-E`. |
| `-t` | Combine `-v` and `-T`. |
| `-u` | Accepted for POSIX compatibility. |
| `--` | End option parsing. A subsequent `-` still denotes standard input. |

`cat` uses one filesystem job to preserve stream order. Without formatting options, files and redirected streams are copied byte for byte. Windows console streams use the platform's UTF-8 handling.

`cat` exits with status `0` on success and `1` on command or I/O errors. A closed output pipe is reported as an I/O error. It reports input errors and continues with subsequent operands. It rejects a regular input file that is also the output file.

## pwd

`gutil pwd [OPTION]...` prints the absolute path of the current working directory.

| Option | Behavior |
| --- | --- |
| `-L`, `--logical` | Use `PWD` when it is an absolute path without `.` or `..` components and identifies the current directory. Otherwise use the physical path. |
| `-P`, `--physical` | Resolve symbolic links in the working directory path. |

The last mode option takes precedence. Physical mode is the built-in default. Setting `POSIXLY_CORRECT` selects logical mode by default. In physical mode, Windows uses the platform's canonical path syntax, which can include an extended-length prefix. Windows paths that cannot be represented as UTF-8 produce an error.

`pwd` uses one filesystem job. It exits with status `0` after printing the path and `1` for invalid options, an unresolvable working directory, or failed output. Extra operands are ignored with a warning.

## cache

`gutil cache [--offline] put [--] FILE` stores a regular file and prints its lowercase SHA-256 digest. `gutil cache [--offline] get SHA256` looks up a 64-digit hexadecimal digest and writes the verified bytes to stdout.

```sh
digest=$(gutil cache put artifact.tar)
gutil cache get "$digest" > restored.tar
gutil cache --offline get "$digest" > offline.tar
```

The local cache is enabled by default. An enabled remote cache can serve local misses, and read-write mode also receives uploads. `--offline` skips remote access for the invocation. Use `--` before a file name beginning with a hyphen.

`cache` exits with status `0` on success, `1` on an unavailable entry or failed operation, and `2` on invalid arguments. [Cache configuration and HTTP protocol](configuration.md#cache)

## plug-in

`gutil plug-in install SOURCE [name:PACKAGE]` builds a Rust plug-in from
GitHub, GitLab, Bitbucket, a local path, crates.io, or a sparse registry.
Use `@v4` for a Git tag or append a commit hash to pin a revision.
Configured registry aliases use `ALIAS:PACKAGE`.
`gutil plug-in use COMMAND [ARGUMENTS]...` runs the installed plug-in
and forwards every remaining argument to it.
[Plug-in installation and SDK](plug-ins.md)

## Checksums

`md5sum`, `sha1sum`, `sha224sum`, `sha256sum`, `sha384sum`, and
`sha512sum` print or verify checksums. With no operands or a `-` operand,
the command reads stdin.

```sh
gutil sha256sum first.bin second.bin > SHA256SUMS
gutil sha256sum --check SHA256SUMS
gutil --jobs=4 --progress=never sha512sum first.bin second.bin
```

| Option | Behavior |
| --- | --- |
| `-b`, `--binary` | Mark generated records as binary. |
| `-t`, `--text` | Mark generated records as text. This is the default. |
| `--tag` | Write BSD-style records, for example `SHA256 (file) = digest`. |
| `-z`, `--zero` | Terminate generated records with NUL and write raw file names. |
| `-c`, `--check` | Verify GNU-style or BSD-style newline-delimited records. |
| `--ignore-missing` | Skip missing files. At least one listed file must verify successfully. |
| `--quiet` | Suppress successful verification records. |
| `--status` | Suppress verification records and summaries. Read errors still appear on stderr. |
| `--strict` | Fail when a checksum list contains malformed records. |
| `-w`, `--warn` | Report each malformed record with its line number. |
| `-j N`, `--jobs N`, `--threads N` | Limit concurrent filesystem jobs before or after the command name. |
| `--` | Treat subsequent arguments as file operands. |

Files are hashed as bytes on every platform, including in text mode. GNU-style
records escape backslashes, carriage returns, and newlines in file names.
Unix records preserve native filename bytes. On Windows, filename encoding in
records is UTF-8. Verification status lines use checksum record escaping for file
names that contain backslashes or line breaks. MD5 and SHA-1 are available for
compatibility with existing manifests.

Multiple regular-file operands totaling at least 1 MiB can be hashed
concurrently when progress output is disabled. Output follows the original
operand order. Verification and stream input are processed sequentially.
If the checksum list comes from stdin, a record naming `-` is malformed.
Checksum commands return `1` on input, verification, or option errors and
`0` on success.

## head and tail

`head` prints the first 10 lines of each input. `tail` prints the last 10.
With no file operands, or for a `-` operand, both read stdin. Input bytes
are preserved, including a final record with no delimiter.

```sh
gutil head -n 20 report.txt
gutil head --bytes=-512 archive.bin
gutil tail -n +3 report.txt
gutil tail -F --sleep-interval=0.25 application.log
```

| Option | Behavior |
| --- | --- |
| `-n N`, `--lines=N` | Select lines. With `-z`, select NUL-delimited records. |
| `-c N`, `--bytes=N` | Select bytes. Units include `b` (512), `K` or `KiB` (1024), and `KB` (1000). |
| `-q`, `--quiet`, `--silent` | Suppress file headers. |
| `-v`, `--verbose` | Show file headers even for one input. The default shows headers for multiple operands. |
| `-z`, `--zero-terminated` | Use NUL as the record delimiter. |
| `-f`, `--follow[=descriptor]` | Poll an open regular file for new data. |
| `--follow=name` | Poll a regular file by path and reopen it after replacement. |
| `-F` | Follow by name and retry opening the file. |
| `--retry` | Keep trying to open an unavailable file while following. |
| `-s N`, `--sleep-interval=N` | Set the interval between follow polls in seconds. The default is one second. |
| `--` | Treat following arguments as file operands. |

A negative count tells `head` to omit the last N bytes or records. A leading
`+` tells `tail` to begin at the given one-based position. Ordinary `tail`
counts select from the end. Named regular files support follow mode. Stdin is
read once. Operands are processed in order. Exit status is `0` on success
and `1` on argument or I/O errors.

## wc

`wc` reports newline, word, and byte counts by default. Explicit count
options select the requested columns. With no file operands, or for a `-`
operand, it reads stdin.

```sh
gutil wc -lwm report.txt
gutil wc -l --files0-from=paths.nul --total=only
```

| Option | Behavior |
| --- | --- |
| `-l`, `--lines` | Count newline bytes. |
| `-w`, `--words` | Count whitespace-delimited words. |
| `-m`, `--chars` | Count characters. |
| `-c`, `--bytes` | Count bytes. Regular files can use their reported size when its boundary is readable. |
| `-L`, `--max-line-length` | Measure the widest line in display columns, with eight-column tab stops. |
| `--files0-from=FILE` | Read NUL-delimited file names. Use `-` to read the list from stdin. |
| `--total=MODE` | Use `auto` for totals with multiple inputs, `always` to always show them, `only` to print the aggregate values, or `never` to suppress totals. |
| `--` | Treat following arguments as file operands. |

An unset locale and the C and POSIX locales count characters as bytes.
Other locale settings use UTF-8 decoding and Unicode column widths.
`POSIXLY_CORRECT` excludes GNU-specific nonbreaking word separators. A stdin file list reserves
stdin for the list itself. The maximum line length total is the largest
width across the inputs. Failed operands produce diagnostics while the
remaining operands are counted. Exit status is `0` on success and `1`
on argument or I/O errors.

## basename and dirname

`basename NAME [SUFFIX]` prints the last component of a path and optionally
removes a matching suffix. A suffix matching the entire component is
preserved. `-a` or `--multiple` accepts several names. `-s SUFFIX` or
`--suffix=SUFFIX` supplies the suffix in that mode.
`dirname NAME...` prints the directory portion of each name, using `.`
for names without a directory component. Both commands support `-z` or
`--zero` for NUL delimiters. Both work directly on operands and preserve
native Unix filename bytes. Windows also recognizes the backslash separator,
drive prefixes, and UNC share roots.

```sh
gutil basename -s .tar backup/first.tar backup/second.tar
gutil dirname ./source/file.txt
```

## readlink and realpath

`readlink` prints a symbolic link's stored target by default. `realpath`
prints a canonical absolute path by default. Both accept multiple file
operands and retain their order.

| Option | Behavior |
| --- | --- |
| `readlink -f`, `realpath -E`, `--canonicalize` | Expand symbolic links and allow a missing final component. |
| `-e`, `--canonicalize-existing` | Require all components to exist. |
| `-m`, `--canonicalize-missing` | Allow missing or unavailable components. |
| `realpath -L`, `--logical` | Process parent components before expanding symbolic links. |
| `realpath -P`, `--physical` | Expand symbolic links in path order. This is the default. |
| `realpath -s`, `--strip`, `--no-symlinks` | Normalize path components without expanding symbolic links. |
| `realpath --relative-to=DIR` | Print paths relative to DIR when they share a filesystem root. |
| `realpath --relative-base=DIR` | Use relative output for paths at or below DIR. |
| `readlink -n`, `--no-newline` | Omit the output delimiter for a single operand. |
| `-z`, `--zero` | Delimit the results with NUL bytes. |
| `-q`, `--quiet` | Suppress path resolution errors. This is the default for `readlink` unless `POSIXLY_CORRECT` is set. |
| `readlink -v`, `--verbose` | Report path resolution errors. |
| `-j N`, `--jobs N`, `--threads N` | Limit concurrent filesystem workers for larger operand batches. |

With both relative options, paths qualify for relative output when
the operand and `--relative-to` are both within `--relative-base`. The
relative path starts at `--relative-to`. The
commands exit with status `1` if one or more operands fail and keep
processing the remaining operands. On Windows, resolved paths use native
separators.

```sh
gutil readlink -n current
gutil realpath -e --relative-to=. ./src/../README.md
```

## Shell and system utilities

`true` returns status `0` and `false` returns status `1`. Both ignore
operands and recognize `--help` or `--version` when passed alone.

`echo [-n] [-e|-E] [STRING]...` writes strings separated by spaces. Use
`-n` to omit the final newline, `-e` to interpret escapes, and `-E` to
preserve backslashes. These options may be grouped and precede the operands.
Escapes include control characters, octal and hexadecimal bytes, and
`\c` to stop output immediately. With `POSIXLY_CORRECT` set, escapes are
always enabled and options are recognized when the first argument is `-n`.

`yes [--] [STRING]...` repeats a line containing all operands or `y`
when none are given. Use `--` before operands that look like options.
A closed pipe ends the command with status `1`.

`printenv [-0|--null] [VARIABLE]...` writes selected values in operand order
or the complete environment as name-value pairs when invoked without
operands. The exit status is `1` when a requested variable is missing and
`2` on option or output errors. Use `-0` for NUL delimiters.

`nproc [--all] [--ignore=N]` reports the number of processing units
available to the process. Explicit values in `OMP_NUM_THREADS` and
`OMP_THREAD_LIMIT` override and cap the count, respectively. `--all`
ignores these variables and reads the installed CPU count on Linux. On
other platforms or when Linux CPU inventory is unavailable, it reports
the platform's available processor count. `--ignore` subtracts from the
result without reducing it below one.

`sleep NUMBER[SUFFIX]...` waits for the sum of nonnegative durations.
Use `s`, `m`, `h`, or `d` for seconds, minutes, hours, or days. Unsuffixed
values are seconds. Fractional decimal and hexadecimal values are
supported. A positive infinite duration waits until interrupted.

```sh
gutil echo -e 'name\tvalue'
gutil printenv -0 HOME PATH
gutil nproc --ignore=1
gutil sleep 0.25s 0.5s
```

## fd

`gutil fd [OPTIONS] [PATTERN] [PATH]...` recursively searches directories.
With no path operands, it searches the current directory. The default pattern
is an empty regular expression, which matches every visible entry. Smart
case uses case-insensitive matching unless the pattern contains an uppercase
character. Results are written in native path order.

```sh
gutil fd -t f -e rs '^test' gutil
gutil fd --glob '*.toml'
gutil --jobs=4 fd -0 --exact Cargo.toml
```

| Option | Behavior |
| --- | --- |
| `-H`, `--hidden` | Include hidden names. |
| `-I`, `--no-ignore` | Use every visible name regardless of ignore rules. |
| `-u`, `--unrestricted` | Include hidden and ignored entries. |
| `--no-ignore-vcs` | Ignore version-control rules while keeping `.ignore` and `.fdignore` rules active. |
| `--no-require-git` | Honor `.gitignore` rules outside Git repositories. |
| `--no-ignore-parent` | Skip ignore files outside the search roots. |
| `-s`, `-i` | Force case-sensitive or case-insensitive matching. |
| `-g`, `--glob` | Interpret the search pattern as a glob. |
| `-F`, `--fixed-strings` | Match a literal substring. |
| `--exact` | Match the complete name literally. |
| `-p`, `--full-path` | Match against the absolute path instead of the final component. |
| `-a`, `--absolute-path` | Write absolute paths. |
| `-0`, `--print0` | End names with NUL bytes. An implicit relative search includes the current-directory prefix. |
| `-L`, `--follow` | Traverse directory symbolic links. |
| `-d N`, `--max-depth N` | Limit search depth relative to each root. |
| `--min-depth N`, `--exact-depth N` | Filter results by depth. |
| `-t TYPE`, `--type TYPE` | Match files, directories, or symbolic links. Repeat to select several types. |
| `-e EXT`, `--extension EXT` | Match extensions without regard to ASCII case. Repeat to select several extensions. |
| `-E GLOB`, `--exclude GLOB` | Skip entries matching a glob. |
| `-q`, `--quiet` | Write no results and exit with status `1` when no entries match. |
| `--show-errors` | Report traversal errors to stderr. |
| `-j N`, `--jobs N`, `--threads N` | Limit search workers. |

Directory results end with a native path separator. Normal searches return
`0` for a valid search even when there are no matches. Invalid options and
searches with no usable root return `1`.

## df and duf

`gutil df [OPTIONS] [FILE]...` reports mounted filesystem space. Without
operands, it reads the visible mount inventory. With operands, it reports
the filesystem containing each file in operand order. Use `duf` for a
rusty-rich table or a JSON inventory from the same mount data.

### df output and filters

| Option | Behavior |
| --- | --- |
| `-a`, `--all` | Include mounts with zero capacity, duplicate mounts, and rows with unavailable statistics. |
| `-B SIZE`, `--block-size=SIZE` | Scale block counts, rounding up to the selected unit. The environment variables `DF_BLOCK_SIZE`, `BLOCK_SIZE`, and `BLOCKSIZE` supply the default in that order. |
| `-h`, `--human-readable` | Use binary size prefixes. |
| `-H`, `--si` | Use decimal size prefixes. |
| `-i`, `--inodes` | Display inode counts when the platform supplies them. |
| `-k` | Use 1024-byte blocks. |
| `-l`, `--local` | Exclude network mounts. |
| `-P`, `--portability` | Use POSIX column headings. |
| `-T`, `--print-type` | Display filesystem types. |
| `-t TYPE`, `--type=TYPE` | Include only the filesystem types specified. Repeat the option for additional types. |
| `-x TYPE`, `--exclude-type=TYPE` | Exclude the filesystem type specified. |
| `--output[=FIELDS]` | Select comma-separated columns from `source,fstype,itotal,iused,iavail,ipcent,size,used,avail,pcent,file,target`. Omitting FIELDS selects all columns. |
| `--total` | Append the total across the printed rows. |
| `--sync`, `--no-sync` | Control whether to sync filesystems first on Linux. |
| `-j N`, `--jobs=N`, `--threads=N` | Limit workers used for large mount selections. |

With no size override, `df` uses 1024-byte blocks or 512-byte blocks when
`POSIXLY_CORRECT` is set. POSIX layout uses these defaults unless a size
option is provided. gutil reports an unavailable inode value as `-`
in the table. Filesystem visibility comes from the current mount
namespace on Linux and the system disk inventory on macOS and Windows.

### duf output and filters

`gutil duf [OPTIONS] [PATH]...` lists filesystems once by default.
Use `--all` to include duplicate and special mounts, `--inodes` for inode
usage, or `--json` for machine-readable results. JSON includes byte totals,
usage, inode counts, and mount details. Unavailable numeric fields are
zero in JSON. A mount measurement failure is reported and makes the exit
status nonzero. Available bytes can be negative on exhausted filesystems.
The `df` table preserves native filename bytes, and duf uses UTF-8 text.
The single-dash spellings used by upstream duf, such as
`-json`, are also accepted.

| Option | Behavior |
| --- | --- |
| `--only GROUPS`, `--hide GROUPS` | Include or exclude device groups from `local,network,fuse,special,loops,binds`. |
| `--only-fs TYPES`, `--hide-fs TYPES` | Include or exclude comma-separated filesystem types. |
| `--only-mp GLOBS`, `--hide-mp GLOBS` | Include or exclude mount points by comma-separated globs. |
| `--output COLUMNS` | Select table fields from `mountpoint,size,used,avail,usage,inodes,inodes_used,inodes_avail,inodes_usage,type,filesystem`. |
| `--sort COLUMN` | Sort by a table field. Text sorts ascending, and numeric values sort descending. |
| `--style unicode`, `--style ascii` | Select table border characters. |
| `--width WIDTH` | Set the maximum width in display columns. |
| `--warnings` | Report mount measurement errors. |

```sh
gutil df -hT /tmp
gutil df --output=source,size,target --total .
gutil duf --only local --sort size
gutil duf --json .
```

## Base32 and Base64

`gutil base32` and `gutil base64` encode bytes using the RFC 4648 alphabets.
Each command reads one file operand, or stdin when the operand is omitted
or `-`. Encoding wraps at 76 columns by default and terminates nonempty
wrapped output with a newline. Unwrapped encoding adds no newline.

| Option | Behavior |
| --- | --- |
| `-d`, `--decode` | Decode the selected alphabet. Line feeds are accepted. |
| `-i`, `--ignore-garbage` | Skip bytes outside the selected alphabet while decoding. |
| `-w N`, `--wrap=N` | Wrap encoded output at N columns. Zero disables wrapping. |
| `-j N`, `--jobs=N`, `--threads=N` | Select the shared runtime worker count. A stream is processed in input order. |
| `--help`, `--version` | Print command-specific help or version. |

Malformed encoded input exits with status 1 and can emit the successfully
decoded prefix, following the supported GNU behavior. Canonical unpadded
tails and concatenated padded groups are accepted. Progress uses stderr
and respects the global `--progress=auto|never|always` setting.

```sh
gutil base64 -w0 file.bin
gutil base64 --decode encoded.txt
gutil base32 --wrap=32 file.bin
```

## Links, empty directories, and file sizes

These commands execute filesystem operations inside gutil's asynchronous
runtime. Operands run in input order, including repeated names and hard-link
aliases. The shared `-j`, `--jobs`, and `--threads` flags bound runtime workers.
`--progress=auto|never|always` controls progress on stderr. Processing continues
with later operands after an individual failure and returns status 1 if any
operation failed.

### link and unlink

`gutil link FILE1 FILE2` creates one hard link. `gutil unlink FILE` removes
one name. Both accept `--` before names beginning with `-` and expose
command-specific `--help` and `--version` options. Existing destination
names are preserved when `link` fails.

### ln

`gutil ln` creates hard links by default. It accepts a source and destination,
a single source for the current directory, multiple sources followed by a
directory, or sources with `--target-directory`.

| Option | Behavior |
| --- | --- |
| `-s`, `--symbolic` | Create symbolic links, including links to missing targets. |
| `-f`, `--force` | Replace existing destination files. Prepare the new link before replacing the old name. |
| `-i`, `--interactive` | Ask on stderr before replacement and read an answer from stdin. |
| `-L`, `--logical` | Follow source symbolic links when making hard links. |
| `-P`, `--physical` | Make hard links to source symlinks themselves where supported. This is the default. |
| `-n`, `--no-dereference` | Treat a destination symlink to a directory as a file name. |
| `-r`, `--relative` | With `--symbolic`, compute link text relative to the destination directory. |
| `-t DIR`, `--target-directory=DIR` | Create links in DIR. |
| `-T`, `--no-target-directory` | Treat the destination as a file name. |
| `-b`, `--backup[=CONTROL]` | Back up existing names. Controls include `none`, `simple`, `numbered`, and `existing`, with GNU aliases. |
| `-S SUFFIX`, `--suffix=SUFFIX` | Set the simple backup suffix and enable backups. |
| `-v`, `--verbose` | Print each created link. |
| `-d`, `-F`, `--directory` | Allow the native hard-link operation to attempt a directory link, subject to platform permissions and filesystem support. |

`VERSION_CONTROL` selects the default backup policy when backups are requested.
`SIMPLE_BACKUP_SUFFIX` defaults to `~`. Replacement links are prepared in a
temporary directory beside the destination. Numbered backups use an atomic no-replace rename and retry with the next
number on collision. If installation fails after a backup is made, gutil
restores it with an atomic no-replace rename. A concurrently created
destination is preserved and the retained backup path is reported on conflict.
Force mode protects hard-link names created earlier in the same invocation,
except when numbered backups are explicitly selected.

On Windows, a symbolic-link privilege failure asks the user to enable
Developer Mode in Windows Settings and retry. Existing directory targets use
directory symlinks. Missing targets use file symlinks. Windows requires the
link type to be selected at creation time. Changing between file and
directory links uses a prepared replacement and a rollback copy of the old
name. A failed rollback retains that copy and reports its path. [Microsoft symbolic-link API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createsymboliclinkw)

### rmdir

`gutil rmdir DIRECTORY...` removes empty directories. `-p`, `--parents` also
attempts each ancestor, stopping on the first failure. `--ignore-fail-on-non-empty`
ignores nonempty-directory errors. Other errors retain a failing exit status.
`-v`, `--verbose` reports each attempted removal. Verbose records use gutil's
command label and escaped path names.

### truncate

`gutil truncate --size=SIZE FILE...` creates missing files and shrinks or
extends existing files. Extension preserves the existing prefix and makes
the added range read as zero bytes. `-c`, `--no-create` skips missing files.

| Option | Behavior |
| --- | --- |
| `-s SIZE`, `--size=SIZE` | Set the size, or apply a modifier below. |
| `-r FILE`, `--reference=FILE` | Use the reference file's size as the base. Combining a reference with a size requires a relative modifier. |
| `-o`, `--io-blocks` | Express the amount in the target filesystem's reported I/O block size on Unix or allocation cluster size on Windows. |

A size accepts binary prefixes such as `K` or `MiB`, and decimal prefixes
such as `KB` or `MB`. Modifiers `+` and `-` extend and reduce the base size,
`<` and `>` impose a maximum or minimum, `/` rounds down to a multiple, and
`%` rounds up. Reductions stop at zero. Invalid sizes and division by zero
are rejected before processing files. Arithmetic overflow is reported for
the affected operand. Unix file operands are opened nonblockingly, so a FIFO
error does not prevent later operands from being processed.

```sh
gutil ln --symbolic --relative source.txt links/source.txt
gutil ln --backup=numbered --force source.txt destination.txt
gutil rmdir --parents empty/tree/leaf
gutil truncate --size=64KiB data.bin
gutil truncate --size=+1MiB first.bin second.bin
gutil truncate --reference=template.bin copy.bin
```

## tee

`gutil tee [OPTIONS] [FILE]...` copies standard input to standard output and
all named files. Existing files are overwritten unless `-a` / `--append` is
used. A literal `-` names a file; `--` ends option parsing. Native paths are
preserved. The stream stays ordered inside one asynchronous blocking task;
`-j`, `--jobs`, and `--threads` set the runtime worker limit.

`-i` / `--ignore-interrupts` ignores Ctrl-C interrupts. Without an output-error
option, a broken output pipe terminates the command (SIGPIPE on Unix); other
output failures are diagnosed while remaining outputs continue.

| Option | Behavior |
| --- | --- |
| `-p`, `--output-error`, `--output-error=warn-nopipe` | Ignore broken pipes, diagnose other failures, and continue with remaining outputs. |
| `--output-error=warn` | Diagnose all output failures and continue with remaining outputs. |
| `--output-error=exit` | Stop at the first output failure. |
| `--output-error=exit-nopipe` | Ignore broken pipes and stop at the first other output failure. |

Output files and owned copies of stdout file/pipe handles are closed explicitly;
close-time failures follow the same error policy as write failures. Windows
console output uses the standard console writer to preserve Unicode handling.
Closing an output does not request an additional `fsync` durability guarantee.

Diagnosed I/O failures return status 1; invalid options return status 2.
An optional output-error mode must follow `=`; `--output-error warn` writes
to a file named `warn`. If no live outputs remain, copying stops without
waiting for input EOF. Rich progress and diagnostics use stderr and preserve
stdout bytes. `gutil tee --help` and `gutil tee --version` work independently
of configuration loading.

## mkdir

`gutil mkdir [OPTIONS] DIRECTORY...` creates directories in operand order.
It continues independent operands after errors and returns status 1 when any
filesystem operation fails. Invalid requests return status 2. `--` ends option
parsing, and native paths are preserved. A single Tokio blocking task preserves
ordering; `-j`, `--jobs`, and `--threads` set the runtime worker limit.

| Option | Behavior |
| --- | --- |
| `-p`, `--parents` | Create missing parents and accept existing directories, including symlinks to directories. |
| `-m MODE`, `--mode=MODE` | Set permissions of newly created operand directories; existing directories retain their modes. |
| `-v`, `--verbose` | Report each newly created directory to stdout. |
| `-Z`, `--context[=CTX]` | Request a security context, subject to the platform limitations below. |
| `--help`, `--version` | Print command-specific information without loading configuration. |

Unix permission modes accept octal numbers, `+`, `-`, or `=` numeric operators,
and symbolic clauses using `u`, `g`, `o`, `a`, `r`, `w`, `x`, `X`, `s`, and `t`.
Clauses can copy permissions, as in `u=rw,g=u,o=g`, or combine operations, as
in `u+r-w+x`. Omitted classes in symbolic clauses use the original umask. Ordinary numeric modes
preserve inherited directory setuid/setgid bits unless they explicitly set
those bits; numeric modes of five or more digits and operator assignment
modes can explicitly clear them.

Parent creation uses the original umask with owner write/search bits unmasked.
`-m` does not change parent permissions. Inherited default ACLs still apply;
gutil does not widen a parent's ACL when it restricts the owner's write access.
Trailing `/.`, `..`, repeated operands, and existing directory aliases retain
filesystem traversal and ordering semantics. Special-mode creation follows
GNU's conservative initial permissions and affected-bit checks. On Linux,
`-m +t` can leave ordinary permissions at 755, whereas `-m 1777` explicitly
requests all ordinary permissions and produces 1777.

Windows directories use inherited ACLs; a supplied POSIX mode is validated and
produces a warning because it cannot set Unix permission bits. On systems
without active SELinux or SMACK labeling, `-Z` and bare `--context` are no-ops,
and an explicit context produces a warning. Assignment on active labeling
systems is not yet implemented and is rejected before creating directories.
No active label assignment is silently claimed.

Rich progress and diagnostics use stderr. Verbose output quotes and escapes
paths; its text intentionally differs from GNU's localized verbose messages.


## mktemp

`gutil mktemp [OPTIONS] [TEMPLATE]` creates a temporary file or directory and
prints its name. The final template component needs a run of at least three
`X` characters. The final run is replaced by uniformly sampled alphanumeric
characters from the operating system's random source. Earlier literal `X`
characters remain unchanged. With no template, use `tmp.XXXXXXXXXX` under the
temporary directory.

| Option | Behavior |
| --- | --- |
| `-d`, `--directory` | Create a directory instead of a file. |
| `-u`, `--dry-run` | Print an unused name without creating or reserving it. |
| `-q`, `--quiet` | Suppress creation-failure diagnostics; failure status remains nonzero. |
| `--suffix=TEXT` | Append a suffix; the supplied template must end in `X`. Otherwise infer the suffix after the final run of `X` characters. |
| `-p DIR`, `--tmpdir[=DIR]` | Resolve a relative template under the directory. An omitted or empty directory selects `TMPDIR`, then the native temporary directory. |
| `-t` | Legacy single-component templates; prefer `TMPDIR` over `-p DIR`. |
| `-j`, `--jobs`, `--threads` | Limit runtime workers; each invocation reserves one name. |

Creation runs in one asynchronous filesystem worker. Existing files,
directories, and symbolic links are treated as collisions and remain intact.
Only the final component is created; intermediate directories must exist.
Without a temporary-directory option, an explicit template is used as written
and may be absolute. Unix files use mode 600 and directories use mode 700,
restricted by the process umask. Windows objects retain native inherited ACLs. Both `-t` and `--tmpdir`
reject Windows drive-prefixed or rooted templates. Windows templates and
suffixes cannot select alternate data streams, and a device handle cannot
be reported as a reserved regular file.
Unix output preserves non-UTF-8 path bytes; Windows output uses its Unicode
text representation, replacing unpaired surrogates when necessary.

Dry-run names are not reserved and can be claimed by another process before
use. Dry runs do not create or delete filesystem objects. A missing parent can
still yield an unused dry-run name. Prefer ordinary atomic creation when the
name will be used for a file or directory.

Exit status is 0 on success, 1 for creation, randomness, close, or output
failure, and 2 for invalid arguments. Usage status 2 is gutil's convention;
GNU uses 1. Quiet mode does not suppress invalid-argument or output errors.
A reserved object remains present when closing or printing its name fails.
On Unix, an unset or empty `TMPDIR` falls back to `/tmp`; Windows uses its
native temporary-directory lookup. Rich progress goes to stderr.

```sh
gutil mktemp
gutil mktemp -d -p ./build cache.XXXXXX
gutil mktemp --suffix=.json artifact.XXXXXXXX
```

## tty

`gutil tty` prints the name of the terminal connected to standard input.
It exits with status 0 when a terminal is available and prints `not a tty`
with status 1 for redirected input. Use `-s`, `--silent`, or `--quiet` to
report the status alone. Usage errors exit with status 2. Output failures
exit with status 3.

Unix reports the native terminal device path. Windows uses the console label
`CONIN$` when Rust recognizes standard input as a terminal.

## arch and uname

`gutil arch` prints the machine hardware name, equivalent to `gutil uname -m`.
`gutil uname` defaults to the kernel name. Use `-s`, `-n`, `-r`, `-v`, `-m`,
`-p`, `-i`, and `-o` for kernel name, node name, kernel release, kernel
version, machine, processor, hardware platform, and operating system. Selected
fields appear in this order even when the options are repeated or reordered.

`-a` and `--all` print the known fields on one line. `-A` and
`--all-labeled` print one labeled field per line. The nonportable processor
and hardware platform fields are currently reported as `unknown` when
requested explicitly and omitted in the combined modes.

Unix uses the running kernel's system identification fields. On Windows,
the kernel name is `Windows_NT`, the release is the OS version, and the
kernel version field is the build number. The OS label is `Windows`.

## paste

`gutil paste [OPTIONS] [FILE]...` merges corresponding records with tab
separators. Shorter inputs contribute empty columns. `-` reads the shared
standard input stream, and no files defaults to standard input.

`-s` or `--serial` merges records within each file, producing one result
record per input. `-z` or `--zero-terminated` uses NUL instead of newline
for input and output records. `-d LIST` or `--delimiters=LIST` cycles through
the list for each row or serial file. Recognized escapes are `\0` (no
delimiter), `\b`, `\f`, `\n`, `\r`, `\t`, `\v`, and `\\`.
Other escaped characters are copied literally. GNU-compatible delimiter
units are bytes in the C locale and characters in UTF-8 locales. Windows
uses Unicode delimiter characters.

An input open failure in parallel mode prevents output. Serial mode
reports a failed file and continues. Both modes retain record order and
send optional progress to standard error.

## uniq

`gutil uniq [OPTIONS] [INPUT [OUTPUT]]` keeps the first record in each run of
adjacent matches. Omitted paths and `-` select the standard streams.
`-c` prints a count, `-d` keeps repeated groups, and `-u` keeps groups
of one. `-D` or `--all-repeated[=METHOD]` streams every repeated record.
The optional method is `none`, `prepend`, or `separate`.
`--group[=METHOD]` prints every record with `separate`, `prepend`,
`append`, or `both` placement of empty separators.

Comparison options are `-i` for ASCII case folding, `-f N` to skip fields,
`-s N` to skip characters after the fields, and `-w N` to compare at most
N characters. UTF-8 locales recognize multibyte characters for skipping
and limiting. The C locale uses bytes. `-z` uses NUL record boundaries.
The command preserves the original bytes and terminates the final record.

Unlike GNU uniq, gutil rejects an input and output referring to the same
regular file, including hard-link aliases and redirected standard streams.
The optional progress stream reports records on stderr.

## expand

`gutil expand [OPTIONS] [FILE]...` replaces tabs with spaces. The default
tab width is eight columns. Use `-t N`, `--tabs=N`, or the historical
`-N` form for a different width. A comma- or blank-separated list specifies
absolute tab columns. A final `/N` resumes at multiples of N from column
zero, and a final `+N` repeats at intervals of N after the last explicit
stop. A finite list replaces tabs beyond the final stop with one space.

`-i` converts only tabs before the first nonblank character on each line.
Backspaces are preserved and move the current column back one position.
Newlines reset the column. Operands form one continuous stream, including
when a preceding file lacks its final newline. Missing file operands and
`-` select standard input.

UTF-8 locales use Unicode display widths. Malformed units and other locale
encodings follow the single-byte behavior. Input errors are reported per
operand and subsequent files are still read. Progress goes to stderr.

## fold

`gutil fold [OPTIONS] [FILE]...` wraps input lines at 80 columns by default.
Use `-w WIDTH`, `--width=WIDTH`, or a historical width option such as
`-72` to select a positive width. `-s` prefers to break after horizontal
blanks. Missing file operands and `-` select standard input.

By default, wrapping tracks display columns. Tabs advance to eight-column
stops, carriage returns reset the column, and backspaces move back by the
last character's width. The backspace width starts at one on each line
and resets to one after a tab or carriage return. `-c` uses a width of one
for ordinary characters.
`-b` counts bytes and ignores those cursor controls. UTF-8 locales keep
complete scalars together, including in byte mode. Malformed bytes are
preserved. Other locale encodings use the single-byte path. Unicode column
widths use the library's Unicode tables.

Operands are processed in order. Each operand starts a new wrapping
position, and a missing final newline stays missing. Errors in one input
do not prevent later inputs from being read. Progress, when enabled, is
written to stderr.

## unexpand

`gutil unexpand [OPTIONS] [FILE]...` replaces eligible blanks with tabs.
The default converts leading blanks at eight-column stops. Use `-a` or
`--all` to convert across the line. `-t N` and `--tabs=LIST` also enable
whole-line conversion. `--first-only` overrides both options regardless
of order. The historical `-N` form changes the stops without enabling
whole-line conversion.

Tab positions use the same list, `/N`, and `+N` syntax as `expand`.
With a finite list, later blanks are copied as written. A single interior
blank before a stop is preserved when it is the only blank in the run.
Backspaces are preserved and reduce the current column by one. Operands
are read in order as one continuous stream. Use `-` for standard input,
which is also the default when no file is given.

UTF-8 locales use Unicode display widths. A replacement involving a
multibyte blank consumes the complete character, keeping valid UTF-8
well-formed. A blank that spans a tab stop is copied to preserve its
display width. Other locale encodings use single-byte columns. File errors
are reported per operand and progress reports bytes on stderr.

## tsort

`gutil tsort [FILE]` reads pairs of names and writes a topological order.
Each pair places its first name before its second. A pair of identical
names declares one node. Spaces, tabs, and newlines separate names.
Omit the file or pass `-` to read standard input. An odd number of names
produces an error before any ordered output.

The initial set of available names uses byte order. When new names become
available through the same node, the command processes its most recent
relations first. As in GNU tsort, an embedded NUL ends a name. Other name
bytes pass through unchanged, even in a UTF-8 locale. The historical
`-w` option is accepted for compatibility.

For cyclic input, gutil reports cycles on stderr, removes enough
relations to finish the ordering, and exits with status 1. Its selection
of relations to remove uses depth-first traversal, so cyclic inputs can
produce a different order and diagnostics from GNU tsort. Progress
reports bytes read and goes to stderr.
