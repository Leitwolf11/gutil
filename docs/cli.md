# Command-line reference

gutil runs utilities as subcommands of a single executable.

```sh
gutil --help
gutil --version
gutil ls --help
gutil ls -la .
```

`gutil --help` lists the available subcommands and global options. `gutil ls --help` lists the syntax and options for `ls`.

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
