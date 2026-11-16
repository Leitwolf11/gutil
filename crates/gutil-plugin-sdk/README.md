# gutil-plugin-sdk

Use this Rust library to write an executable for `gutil plug-in use`. Name the
binary target `gutil-<command>` and read the host invocation with
`gutil_plugin_sdk::Invocation::from_env()`.

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

The host forwards native argument strings and inherits the working directory,
standard streams, and environment. It sets `GUTIL_PLUGIN_PROTOCOL=1` to identify
the command-line interface understood by this crate.

See the [gutil plug-in guide](https://github.com/Leitwolf11/gutil/blob/main/docs/plug-ins.md)
for packaging and installation.
