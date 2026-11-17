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

`gutil plug-in install github.com:OWNER/EXTENSION` builds and installs a Rust
plug-in from GitHub. `gutil plug-in use COMMAND [ARGUMENTS]...` runs the
installed command and forwards every remaining argument to it.
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
