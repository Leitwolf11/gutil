# gutil-plugin-sdk

Use this Rust library to write a WebAssembly module for `gutil plug-in use`. Name the
binary target `gutil-<command>` and read the host invocation with
`gutil_plugin_sdk::Invocation::from_env()`.

```sh
cargo add gutil-plugin-sdk
```

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

Compile the binary target for `wasm32-wasip1`. gutil runs the module in its
own process and forwards UTF-8 arguments and standard streams. It provides
the current working directory through WASI and sets
`GUTIL_PLUGIN_PROTOCOL=1`.

See the [gutil plug-in guide](https://github.com/Leitwolf11/gutil/blob/main/docs/plug-ins.md)
for packaging and installation.
