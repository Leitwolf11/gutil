use std::ffi::OsString;
use std::io::{self, BufWriter, Write};
use std::path::{Component, Path, PathBuf};

use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

mod mode;
mod options;
mod platform;

use mode::Evaluated;
use options::{Context, Options, Request};

const HELP: &str = "Usage: gutil mkdir [OPTIONS] DIRECTORY...\n\nCreate directories in operand order.\n\n  -m, --mode=MODE  Use an octal or symbolic permission mode\n  -p, --parents    Create missing parents and accept existing directories\n  -v, --verbose    Report every created directory\n  -Z               Request the default security context\n      --context[=CTX]  Request a default or explicit security context\n  -j, --jobs, --threads N  Limit runtime workers (operands stay ordered)\n      --help       Print help\n      --version    Print version\n\nParents unmask owner write/search access; inherited ACLs remain effective.\nExisting directories keep their permissions. Windows uses inherited ACLs.\nContext options are accepted on systems without active labeling; explicit\ncontexts emit a warning. Active SELinux/SMACK context assignment is unsupported.\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "gutil mkdir {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        Ok(Request::Create(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), "gutil mkdir", &error.to_string());
            return Ok(2);
        }
    };
    if !context_ready(
        &options.context,
        platform::active_labels,
        ui,
        &mut io::stderr(),
    )? {
        return Ok(1);
    }
    #[cfg(windows)]
    if options.mode.is_some() {
        let _ = ui.warning(
            &mut io::stderr(),
            "gutil mkdir",
            "POSIX permission modes are unavailable; directories use inherited Windows ACLs",
        );
    }
    let mask = platform::Mask::capture();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(&options, mask, ui))
            .await
            .map_err(io::Error::other)
    })
}

fn context_ready(
    context: &Context,
    probe: impl FnOnce() -> io::Result<bool>,
    ui: Presentation,
    errors: &mut impl Write,
) -> io::Result<bool> {
    if matches!(context, Context::Inherited) {
        return Ok(true);
    }
    if probe()? {
        let _ = ui.error(
            errors,
            "gutil mkdir",
            "SELinux/SMACK context assignment is not implemented; no directories were created",
        );
        return Ok(false);
    }
    if let Context::Explicit(context) = context {
        let _ = ui.warning(
            errors,
            "gutil mkdir",
            &format!("security labeling is unavailable; ignoring context {context:?}"),
        );
    }
    Ok(true)
}

struct Creator<'a, W, E> {
    options: &'a Options,
    mask: platform::Mask,
    mode: Option<Evaluated>,
    output: &'a mut W,
    errors: &'a mut E,
    progress: Option<ProgressReport>,
    prefer_existing: bool,
}

fn execute(options: &Options, mask: platform::Mask, ui: Presentation) -> u8 {
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = stderr.lock();
    let evaluated = options
        .mode
        .as_ref()
        .map(|mode| mode.evaluate(mask.original()));
    let mut creator = Creator {
        options,
        mask,
        mode: evaluated,
        output: &mut output,
        errors: &mut errors,
        progress: ui.progress("gutil mkdir", "directories", true),
        prefer_existing: false,
    };
    let mut failed = false;
    for path in &options.paths {
        if let Err(error) = creator.ensure(path) {
            let _ = ui.error(creator.errors, "gutil mkdir", &error.to_string());
            failed = true;
        }
    }
    if let Some(progress) = &mut creator.progress {
        progress.finish(creator.errors);
    }
    if let Err(error) = creator.output.flush() {
        let _ = ui.error(creator.errors, "gutil mkdir", &error.to_string());
        failed = true;
    }
    u8::from(failed)
}

impl<W: Write, E: Write> Creator<'_, W, E> {
    fn ensure(&mut self, path: &Path) -> io::Result<()> {
        // Remember the cheaper syscall order for a run of existing operands,
        // not their existence: every operand still gets a fresh filesystem check.
        if self.options.parents && self.prefer_existing && path.is_dir() {
            return Ok(());
        }
        match self.create_one(path, !ends_in_dot(path)) {
            Ok(created) => {
                self.prefer_existing = !created;
                return Ok(());
            }
            Err(error) if self.options.parents && error.kind() == io::ErrorKind::NotFound => {
                self.prefer_existing = false;
            }
            Err(error) => return Err(error),
        }
        let mut prefix = PathBuf::new();
        let mut components = path.components().peekable();
        let trailing_dot = ends_in_dot(path);
        while let Some(component) = components.next() {
            prefix.push(component.as_os_str());
            if matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::CurDir
            ) {
                continue;
            }
            self.create_one(&prefix, components.peek().is_none() && !trailing_dot)?;
        }
        if prefix.as_os_str().is_empty() {
            return Err(path_error(path, &io::Error::from(io::ErrorKind::NotFound)));
        }
        if trailing_dot {
            self.create_one(path, false)?;
        }
        Ok(())
    }

    fn create_one(&mut self, path: &Path, leaf: bool) -> io::Result<bool> {
        let requested = leaf.then_some(self.mode).flatten();
        let permissions = requested.map_or(0o777, |mode| {
            if mode.affected & 0o6000 != 0 || mode.bits & 0o1000 != 0 {
                mode.bits & !0o022
            } else {
                mode.bits
            }
        });
        let mask = if requested.is_some() {
            0
        } else if leaf {
            self.mask.original()
        } else {
            self.mask.original() & !0o300
        };
        let entry = platform::Entry::new(path).map_err(|error| path_error(path, &error))?;
        match self.mask.create(&entry, permissions, mask) {
            Ok(()) => {}
            Err(error)
                if self.options.parents
                    && error.kind() == io::ErrorKind::AlreadyExists
                    && entry.is_dir() =>
            {
                return Ok(false)
            }
            Err(error) => return Err(path_error(path, &error)),
        }
        #[cfg(unix)]
        Self::adjust(&entry, requested).map_err(|error| path_error(path, &error))?;
        if self.options.verbose {
            writeln!(
                self.output,
                "gutil mkdir: created directory {}",
                quote(path)
            )?;
        }
        if let Some(progress) = &mut self.progress {
            progress.advance(1, self.errors);
        }
        Ok(true)
    }

    #[cfg(unix)]
    fn adjust(entry: &platform::Entry<'_>, requested: Option<Evaluated>) -> io::Result<()> {
        if let Some(mode) = requested {
            let actual = entry.permissions()?;
            let target = mode.bits | (actual & 0o6000 & !mode.affected);
            let affected = mode.affected;
            if (actual ^ target) & affected != 0 {
                entry.set_permissions(target)?;
            }
        }
        Ok(())
    }
}

fn ends_in_dot(path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    let separator = |byte: &u8| *byte == b'/' || (cfg!(windows) && *byte == b'\\');
    let end = bytes
        .iter()
        .rposition(|byte| !separator(byte))
        .map_or(0, |index| index + 1);
    let bytes = &bytes[..end];
    bytes == b"."
        || bytes
            .strip_suffix(b".")
            .is_some_and(|prefix| prefix.last().is_some_and(separator))
}

fn path_error(path: &Path, error: &io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("cannot create directory {}: {error}", quote(path)),
    )
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.as_os_str().to_string_lossy().escape_debug())
}

#[cfg(test)]
mod tests;
