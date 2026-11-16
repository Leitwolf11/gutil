# Plug-ins

Install a Rust plug-in from a GitHub repository using Cargo:

```sh
gutil plug-in install github.com:owner/extension
gutil plug-in use plugin-command --example-flag example-argument
```

The installed Cargo package must provide exactly one binary target named
`gutil-<command>`. For example, the binary target `gutil-plugin-command`
registers the command `plugin-command`. A command starts with a lowercase
ASCII letter and can contain lowercase ASCII letters, digits, and hyphens.

Installation requires GitHub access, Cargo, and a Rust toolchain. Commit the
extension's `Cargo.lock` so Cargo can reproduce dependency resolution with
`--locked`. Cargo builds the GitHub repository's default branch for the current
platform. Builds and plug-ins run with the current user's permissions, so only
install repositories you trust.

gutil installs binaries to `~/.gutil/plugins` on Linux and macOS and
`%USERPROFILE%\.gutil\plugins` on Windows. Each command name can be installed
once. To replace a plug-in, remove its installed `gutil-<command>` executable
and run the installation command again. On Windows, the executable ends in
`.exe`.

The launcher forwards every argument after the command name, including
`--help` and `--`. The plug-in inherits standard streams, the working directory,
and the environment. It also receives `GUTIL_PLUGIN_PROTOCOL=1`. gutil
propagates normal exit statuses from 0 through 255. On Unix, a signal
termination returns 128 plus the signal number.

## Write a plug-in in Rust

Add this to the extension's `Cargo.toml`:

```toml
[dependencies]
gutil-plugin-sdk = { git = "https://github.com/Leitwolf11/gutil", tag = "v0.6.0" }

[[bin]]
name = "gutil-plugin-command"
path = "src/main.rs"
```

The SDK validates the launcher protocol and provides the forwarded arguments as
native strings. Handle your own flags and exit status in the plug-in.

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
