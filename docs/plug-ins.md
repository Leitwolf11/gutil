# Plug-ins

Install a Rust plug-in from a GitHub repository using Cargo:

```sh
gutil plug-in install github.com:owner/extension
gutil plug-in install github.com:owner/extension@v4
gutil plug-in install github.com:owner/extension@abcdef0123456789abcdef0123456789abcdef0123
gutil plug-in use plugin-command --example-flag example-argument
```

The installed Cargo package must provide exactly one binary target named
`gutil-<command>`. For example, the binary target `gutil-plugin-command`
registers the command `plugin-command`. A command starts with a lowercase
ASCII letter and can contain lowercase ASCII letters, digits, and hyphens.

Installation requires GitHub access, Cargo, and the `wasm32-wasip1` target.

```sh
rustup target add wasm32-wasip1
```

Without a selector, Cargo builds the GitHub repository's default branch into
a WASI module. Add `@v4` to build the tag `v4`, or append a commit hash to
build that revision. Hexadecimal selectors of 7 through 40 characters select
commits, including unambiguous abbreviated hashes. Other selectors are Git
tag names, which can include paths such as `@releases/v4`. A tag selector
selects the tag even when a branch has the same name. A missing tag or commit
fails installation, and gutil does not fall back to the default branch.
Tags can be moved by repository maintainers. Use a full commit hash when
the selected revision must stay fixed.

Commit the extension's `Cargo.lock` so Cargo can resolve dependencies with
`--locked`. Cargo runs build scripts with your permissions. Install from
repositories you trust.

gutil stores WebAssembly modules in `~/.gutil/plugins` on Linux and macOS
and `%USERPROFILE%\.gutil\plugins` on Windows. To replace a plug-in,
remove its installed `gutil-<command>.wasm` file and install again.

`gutil plug-in use` runs the module inside the gutil process with the embedded
Wasmi interpreter. The module receives standard streams, access to the current
directory through WASI, and `GUTIL_PLUGIN_PROTOCOL=1`. Pass filesystem paths
relative to that directory. The environment available to the module contains
only the protocol variable.

gutil forwards UTF-8 arguments after the command name, including `--help` and
`--`. It propagates WASI exit statuses from 0 through 125 and reports traps as
command failures.

## Write a plug-in in Rust

Add this to the extension's `Cargo.toml`:

```toml
[dependencies]
gutil-plugin-sdk = "0.1.0"

[[bin]]
name = "gutil-plugin-command"
path = "src/main.rs"
```

The SDK validates the launcher protocol and provides the forwarded arguments as
`OsString` values. Handle your own flags and exit status in the plug-in.

```rust
use gutil_plugin_sdk::Invocation;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let invocation = Invocation::from_env()?;
    for argument in invocation.into_args() {
        println!("{argument:?}");
    }
    Ok(())
}
```
