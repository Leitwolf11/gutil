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

## Filesystem jobs

Set the limit on concurrent filesystem jobs with `-j N`, `--jobs N`, or `--threads N`. The option works before or after the subcommand. The following commands use the same limit.

```sh
gutil -j 4 ls -la .
gutil ls --jobs=4 -la .
gutil ls -la --threads 4 .
```

The default is the detected parallelism, capped at eight. Set the limit to one to run filesystem work serially. Positive job counts above 512 use 512. Concurrent jobs preserve the output order.

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

The last mode option takes precedence. Physical mode is the default. Setting `POSIXLY_CORRECT` selects logical mode by default. In physical mode, Windows uses the platform's canonical path syntax, which can include an extended-length prefix. Windows paths that cannot be represented as UTF-8 produce an error.

`pwd` uses one filesystem job. It exits with status `0` after printing the path and `1` for invalid options, an unresolvable working directory, or failed output. Extra operands are ignored with a warning.
