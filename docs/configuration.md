# Configuration

gutil reads the first configuration file it finds.

| Priority | Linux and macOS | Windows |
| --- | --- | --- |
| 1 | `./gutil.toml` | `.\gutil.toml` |
| 2 | `~/.gutil/config.toml` | `%USERPROFILE%\.gutil\config.toml` |

The first file supplies all configuration values. Missing settings use the built-in defaults. An unreadable or invalid file produces an error with its path. Direct help and version requests remain available. Use `--no-config` for scripts that require built-in defaults regardless of the working directory. Setting keys and enumerated values use kebab-case, and unknown keys are errors.

Copy the example to one of the paths above, or pass it explicitly. [Example configuration](examples/gutil.toml)

```sh
gutil --config ./team.toml --profile interactive ls
gutil --profile ci --jobs 4 cat report.txt
gutil --progress=always cat large.bin > copy.bin
gutil --no-config pwd -P
```

Place global options before the command. The existing job flags also work after the command. `--config PATH` selects a specific file and `--no-config` skips file lookup. `--profile NAME` selects one profile from the chosen file. A profile can override fields in any table, including nested tables such as `ls.long`.

The effective order is command-line flags, the selected profile, the file's top-level settings, and built-in defaults. For `pwd`, `POSIXLY_CORRECT` takes precedence over the profile and file. An explicit `-L` or `-P` takes precedence over it.

## Global settings

| Key | Values and behavior |
| --- | --- |
| `global.jobs` | A job limit from 1 through 512, or `"auto"` for detected parallelism. The default is `"auto"`. |
| `global.max-auto-jobs` | Upper limit for automatic parallelism, from 1 through 512. The default is 8. |
| `global.progress` | `"auto"`, `"never"`, or `"always"`. The default is `"auto"`. Override it with `--progress=MODE`. |
| `output.color` | `"auto"`, `"never"`, or `"always"`. Automatic color is enabled on a terminal unless `NO_COLOR` is set. Override it with `--color=MODE`. |
| `output.hyperlinks` | `"auto"`, `"never"`, or `"always"`. The default is `"never"`. Override it with `--hyperlinks=MODE`. |
| `diagnostics.format` | `"text"` or `"json"`. JSON diagnostics are one object per line on stderr. |
| `diagnostics.verbosity` | `"quiet"` suppresses warnings. `"normal"` shows warnings and errors. `"verbose"` also reports the selected configuration, profile, and global job limit. |

### Progress output

Progress is written to stderr. `auto` reports operations lasting at least 250 milliseconds when stderr is a terminal. `always` emits progress even when stderr is redirected. Updates are limited to one per 250 milliseconds, plus a final count. `never` disables progress. `ls` counts output entries, `cat` counts input bytes, and `pwd` has no progress display.

With text diagnostics, rusty-rich renders progress and adds a spinner or a progress bar on terminals with color enabled. A single regular file copied by `cat` provides a known total. With JSON diagnostics, each progress record includes `type`, `command`, `unit`, `completed`, `total`, and `finished`. JSON records and redirected progress contain no terminal control sequences. Automatic progress emits only the final count for listings and for copies that read from or write to a terminal.

## Command settings

### ls

| Key | Values and behavior |
| --- | --- |
| `layout` | `"one-per-line"`, `"columns"`, or `"long"`. Columns use rusty-rich and fit the terminal width, with an 80-column fallback. Entries are read left to right across each row. |
| `hidden` | `"omit"`, `"almost-all"`, or `"all"`. |
| `sort` | `"name"`, `"size"`, `"modified"`, `"extension"`, or `"none"`. Size is largest first and modified is newest first. |
| `reverse` | Reverse the order within each group. The default is false. |
| `directories-first` | Group directories before other entries within a listing. The default is false. |
| `quote` | `"auto"` shows safe display text on a terminal, `"literal"` writes unquoted names, and `"shell"` uses shell quoting. Terminal and styled listings sanitize control characters, including in literal mode. Plain one-per-line Unix listings can preserve native filename bytes. |

Within `[ls.long]`, `size-style` accepts `"bytes"`, `"human"` (base 1024), and `"si"` (base 1000). `time-style` accepts `"iso"`, `"rfc3339"`, and `"unix"`. `timezone` accepts `"utc"` and `"local"`. `owner-format` accepts `"numeric"` or `"none"`.

The options `--layout=MODE`, `--hidden=MODE`, `--sort=MODE`, `--quote=MODE`, `--size-style=MODE`, `--time-style=MODE`, `--timezone=MODE`, and `--owner-format=MODE` override their respective settings. Use `--reverse` or `--no-reverse` and `--directories-first` or `--no-directories-first` to override the booleans.

### cat

`number` accepts `"none"`, `"all"`, or `"nonblank"`. The switches `squeeze-blank`, `show-ends`, `show-tabs`, and `show-nonprinting` default to false. Use `--number=none` to disable configured numbering and the corresponding `--no-*` options to turn off individual switches.

### pwd

`mode` accepts `"physical"` or `"logical"`. Physical is the built-in default.

## Cache

The `cache` command stores immutable blobs by SHA-256. Cache settings apply to explicit `cache put` and `cache get` calls. Profiles can override any of these settings.

| Key | Values and behavior |
| --- | --- |
| `cache.max-entry-bytes` | Maximum bytes per entry, from 1 to 268435456. The default is 16777216 (16 MiB). The limit applies to local files and remote responses. |
| `cache.local.enabled` | Whether to use the local cache. Defaults to true. |
| `cache.local.directory` | Cache root. Relative paths are resolved against the configuration file's directory. Defaults to `~/.gutil/cache` on Linux and macOS and `%USERPROFILE%\.gutil\cache` on Windows. |
| `cache.remote.mode` | `"off"`, `"read-only"`, or `"read-write"`. Defaults to `"off"`. |
| `cache.remote.endpoint` | Base URL for the remote cache, required in read-only and read-write modes. Use HTTPS, or HTTP on loopback for local development. |
| `cache.remote.timeout-ms` | Total timeout per HTTP request, including the response body, from 1 to 60000 milliseconds. Defaults to 3000. |

A discovered home configuration can enable the remote cache. To enable remote access from a project configuration, select it explicitly with `gutil --config ./gutil.toml cache …`. Use `cache --offline` to disable network access for a single call. The optional `GUTIL_CACHE_TOKEN` environment variable is sent as a Bearer token over HTTPS. The client sends neither tokens over HTTP nor requests to redirect targets.

A local hit avoids a network request. A verified remote hit fills or repairs the local entry when the local cache is enabled. A put succeeds if at least one writable cache stores the entry. Backend failures appear as warnings on stderr. Local entries are written atomically and can be removed from the cache directory to reclaim space.

### Remote HTTP protocol

Configure a service that handles these requests at the base URL, such as `https://cache.example.com/team/builds`. Use separate base paths for separate cache namespaces.

| Request | Service response |
| --- | --- |
| `GET /team/builds/sha256/<digest>` | Return `200 OK` with the raw bytes or `404 Not Found` for a miss. |
| `PUT /team/builds/sha256/<digest>` | Store the `application/octet-stream` body and return a `2xx` response. Treat repeated puts of the same digest as idempotent. |

Digests in requests are lowercase hexadecimal. gutil checks the size and SHA-256 digest before returning any fetched bytes.
