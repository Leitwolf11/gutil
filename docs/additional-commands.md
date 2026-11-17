# Additional commands

Gutil plans to host the following tools in the same executable as its Coreutils commands.

| Command | Planned capabilities | Reference |
| --- | --- | --- |
| `gutil eza` | Directory listings with tree and grid views, file type icons, and Git status. | [eza](https://github.com/eza-community/eza) |
| `gutil duf` | Mounted filesystem discovery, capacity and free-space reporting, filters, and JSON output. | [duf](https://github.com/muesli/duf) |
| `gutil fd` | Recursive file search with smart-case patterns, ignore rules, extension and type filters, and NUL-delimited output. | [fd](https://github.com/sharkdp/fd) |

Interactive output uses gutil's terminal presentation settings. Machine-readable
output is suitable for pipes and other programs. These three commands are
outside the [GNU Coreutils coverage target](coreutils-coverage.md).
