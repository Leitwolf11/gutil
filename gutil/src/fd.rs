use std::ffi::OsString;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{PathBuf, MAIN_SEPARATOR_STR};

use ignore::{DirEntry, WalkBuilder, WalkState};
use parking_lot::Mutex;

use crate::{jobs, presentation::Presentation};

mod options;
use options::{Options, Request, Search};

const HELP: &str = "Usage: gutil fd [OPTIONS] [PATTERN] [PATH]...\n\nSearch directory trees for matching entries. Hidden entries and ignored paths are skipped by default.\n\n  -H, --hidden           Include hidden entries\n  -I, --no-ignore        Include entries excluded by ignore files\n  -u, --unrestricted     Include both hidden and ignored entries\n      --no-hidden        Skip hidden entries again\n      --ignore           Honor ignore files again\n      --no-ignore-vcs    Skip version-control ignore rules\n      --ignore-vcs       Honor version-control ignore rules again\n      --no-ignore-parent Skip ignore files above the search root\n      --ignore-parent    Honor ignore files above the root again\n      --no-require-git   Honor Git rules outside Git repositories\n      --require-git      Use Git rules only in repositories\n  -s, --case-sensitive   Match case exactly\n  -i, --ignore-case      Match without regard to case\n  -g, --glob             Match a glob instead of a regular expression\n      --regex            Match a regular expression again\n  -F, --fixed-strings    Match a literal substring\n      --exact            Match the entire name literally\n  -p, --full-path        Match the path including its directory portion\n  -L, --follow           Follow symbolic links to directories\n  -a, --absolute-path    Print absolute paths\n  -0, --print0           End each result with NUL\n  -d, --max-depth N      Limit traversal depth\n      --min-depth N      Skip entries above this depth\n      --exact-depth N    Match at a single depth\n  -t, --type TYPE        Filter by file, directory, or symlink\n  -e, --extension EXT    Filter by extension\n  -E, --exclude GLOB     Exclude matching paths\n  -q, --quiet            Exit successfully if any match is found\n      --show-errors      Report traversal and ignore-file errors\n  -j, --jobs, --threads N  Limit concurrent search workers\n      --help             Print help\n      --version          Print version\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    match options::parse(args, workers) {
        Ok(Request::Help) => io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            writeln!(io::stdout(), "gutil fd {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        Ok(Request::Search(options)) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .max_blocking_threads(1)
                .build()?;
            runtime.block_on(async move {
                tokio::task::spawn_blocking(move || execute(*options, ui))
                    .await
                    .map_err(io::Error::other)?
            })
        }
        Err(error) => {
            ui.error(&mut io::stderr(), "gutil fd", &error.to_string())?;
            Ok(1)
        }
    }
}

#[derive(Default)]
struct Matches {
    paths: Vec<(PathBuf, bool)>,
    errors: Vec<ignore::Error>,
}

fn execute(options: Options, ui: Presentation) -> io::Result<u8> {
    let search = match Search::new(options) {
        Ok(search) => search,
        Err(error) => {
            ui.error(&mut io::stderr(), "gutil fd", &error.to_string())?;
            return Ok(1);
        }
    };
    let mut matches = Matches::default();
    let mut errors = io::stderr().lock();
    let mut found_root = false;
    for root in &search.options.roots {
        let result = fs::metadata(root).and_then(|metadata| {
            if metadata.is_dir() {
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotADirectory,
                    "search root is not a directory",
                ))
            }
        });
        if let Err(error) = result {
            ui.error(
                &mut errors,
                "gutil fd",
                &format!("{}: {error}", root.display()),
            )?;
            continue;
        }
        found_root = true;
        let mut builder = WalkBuilder::new(root);
        if let Err(error) = search.configure(&mut builder, root) {
            ui.error(&mut errors, "gutil fd", &error.to_string())?;
            return Ok(1);
        }
        walk(&builder, &search, &mut matches);
        if search.options.output.quiet && !matches.paths.is_empty() {
            break;
        }
    }
    if search.options.output.show_errors {
        for error in matches.errors {
            ui.warning(&mut errors, "gutil fd", &error.to_string())?;
        }
    }
    if search.options.output.quiet {
        return Ok(u8::from(matches.paths.is_empty()));
    }
    if !found_root {
        return Ok(1);
    }
    matches.paths.sort_unstable();
    let mut output = BufWriter::new(io::stdout().lock());
    for (path, directory) in matches.paths {
        let path = if search.options.output.strip_implicit_dot {
            path.strip_prefix(".").unwrap_or(&path)
        } else {
            &path
        };
        output.write_all(path.as_os_str().as_encoded_bytes())?;
        if directory {
            output.write_all(MAIN_SEPARATOR_STR.as_bytes())?;
        }
        output.write_all(&[search.options.output.delimiter])?;
    }
    output.flush()?;
    Ok(0)
}

fn walk(builder: &WalkBuilder, search: &Search, matches: &mut Matches) {
    if search.options.workers.get() == 1 {
        for result in builder.build() {
            if let Some(entry) = search.inspect(result, &mut matches.errors) {
                matches.paths.push(path_and_type(entry));
                if search.options.output.quiet {
                    break;
                }
            }
        }
    } else {
        let shared = Mutex::new(Matches::default());
        builder.build_parallel().run(|| {
            Box::new(|result| {
                let mut errors = Vec::new();
                let entry = search.inspect(result, &mut errors);
                if !errors.is_empty() || entry.is_some() {
                    let mut matches = shared.lock();
                    matches.errors.extend(errors);
                    if let Some(entry) = entry {
                        matches.paths.push(path_and_type(entry));
                        if search.options.output.quiet {
                            return WalkState::Quit;
                        }
                    }
                }
                WalkState::Continue
            })
        });
        let shared = shared.into_inner();
        matches.paths.extend(shared.paths);
        matches.errors.extend(shared.errors);
    }
}

fn path_and_type(entry: DirEntry) -> (PathBuf, bool) {
    let directory = entry.file_type().is_some_and(|kind| kind.is_dir());
    (entry.into_path(), directory)
}
