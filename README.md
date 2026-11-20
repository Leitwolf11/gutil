# gutil

gutil is a Rust 2021 project to reimplement GNU coreutils as a single executable for Linux, macOS, and Windows. GNU compatibility and lower runtime are project goals. gutil includes `ls` for directory listings, `cat` for concatenating files and standard input, `pwd` for printing the working directory, and `cache` for storing and retrieving files by SHA-256 digest.

gutil also includes `head` and `tail` for selecting parts of streams, `wc` for counts, `tee` for copying a stream to multiple outputs, and the MD5, SHA-1, and SHA-2 checksum utilities.

The name gutil has no expanded form.

## Install

```sh
cargo install gutil
```

## Usage

```sh
gutil ls -la .
gutil --jobs 4 ls -la .
gutil cat -n README.md
gutil pwd -P
gutil mkdir -p -m755 build/cache
gutil mktemp --suffix=.json artifact.XXXXXXXX
gutil tail -n 20 application.log
gutil wc -l README.md
gutil cat README.md | gutil tee saved-readme.md
```

`ls` supports `-a` (`--all`), `-A` (`--almost-all`), `-d` (`--directory`), `-l`, `-1`, and `--`. It accepts multiple file and directory operands and defaults to the current directory. It sorts by the native filename representation and emits one entry per line. Long output uses UTC timestamps and numeric Unix owner and group IDs. On Windows, long output reports basic read-only information and marks unavailable ownership fields with `-`. Hidden entries are identified by a leading dot on all platforms.

Filesystem work runs on a Tokio blocking pool. Use `-j N`, `--jobs N`, or `--threads N` before or after the command name to limit concurrent jobs. The default is the detected parallelism, capped at eight. A job count of one processes filesystem work serially. Requests above 512 use 512 as the limit. Concurrent execution preserves the listing order.

## Documentation

- [Command-line reference](docs/cli.md)
- [Configuration and profiles](docs/configuration.md)
- [Local and remote cache](docs/configuration.md#cache)
- [Plug-ins and the Rust SDK](docs/plug-ins.md)
- [Benchmark methodology and results](docs/benchmarking.md)
- [Coreutils coverage](docs/coreutils-coverage.md)
- [Additional commands](docs/additional-commands.md)

## Development

To install from a local checkout, run `cargo install --path gutil --locked`.

The plug-in integration tests compile a fixture for WASI. Install the target
with `rustup target add wasm32-wasip1` before running the test suite.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo audit --deny warnings
actionlint
zizmor .
reuse lint
```

The Criterion benchmark runs short and long listings against a 1,000-file fixture. Set `GNU_LS` to the absolute path of a GNU `ls` executable to include baseline measurements.

```sh
GNU_LS=/usr/bin/ls cargo bench --locked --bench ls
```

GitHub Actions runs the Rust checks on Linux, macOS, and Windows and saves an LCOV report. Add `CODECOV_TOKEN` and `CODACY_PROJECT_TOKEN` as Actions secrets after connecting the repository to those services. Qodo reviews pull requests using the configuration in `.pr_agent.toml`.

## License

Copyright 2026 Gutil's Contributors. Licensed under the GNU Lesser General Public License, version 3 or later. See [LGPL-3.0-or-later](LICENSES/LGPL-3.0-or-later.txt) and the accompanying [GPL text](COPYING.GPL).
