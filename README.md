# gutil

gutil is a Rust 2021 project to reimplement GNU coreutils as a single executable for Linux, macOS, and Windows. GNU compatibility and lower runtime are project goals. The current release implements a subset of `ls`.

## Build and run

```sh
cargo install --path . --locked
gutil ls -la .
gutil --jobs 4 ls -la .
```

`ls` supports `-a` (`--all`), `-A` (`--almost-all`), `-d` (`--directory`), `-l`, `-1`, and `--`. It accepts multiple file and directory operands and defaults to the current directory. It sorts by the native filename representation and emits one entry per line. Long output uses UTC timestamps and numeric Unix owner and group IDs. On Windows, long output reports basic read-only information and marks unavailable ownership fields with `-`. Hidden entries are identified by a leading dot on all platforms.

Filesystem work runs on a Tokio blocking pool. Use `-j N`, `--jobs N`, or `--threads N` before or after the command name to limit concurrent jobs. The default is the detected parallelism, capped at eight. A job count of one processes filesystem work serially. Concurrent execution preserves the listing order.

## Development

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
