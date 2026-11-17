use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::{canonical, long_value, output_path, Command, Operands, ParseError};
use crate::{jobs, presentation::Presentation};
use canonical::{Existence, Mode, Order};

pub(super) struct Options {
    pub(super) paths: Operands,
    mode: Mode,
    link_target: bool,
    quiet: bool,
    delimiter: u8,
    omit_delimiter: bool,
    relative_to: Option<PathBuf>,
    relative_base: Option<PathBuf>,
}

impl Options {
    pub(super) fn new(command: Command) -> Self {
        let link = command == Command::Readlink;
        Self {
            paths: Operands::new(),
            mode: Mode::default(),
            link_target: link,
            quiet: link && env::var_os("POSIXLY_CORRECT").is_none(),
            delimiter: b'\n',
            omit_delimiter: false,
            relative_to: None,
            relative_base: None,
        }
    }

    pub(super) fn option(
        &mut self,
        command: Command,
        arg: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
        workers: &mut jobs::Count,
    ) -> Result<(), ParseError> {
        let bytes = arg.as_encoded_bytes();
        match bytes {
            [b'-', flags @ ..] if !flags.starts_with(b"-") => {
                for (index, &flag) in flags.iter().enumerate() {
                    if flag == b'j' {
                        *workers = jobs::short_value(&flags[index + 1..], args)?;
                        break;
                    }
                    self.flag(command, flag)
                        .ok_or_else(|| ParseError::unknown(arg))?;
                }
            }
            b"--canonicalize" => self.select(Existence::FinalMayBeMissing),
            b"--canonicalize-existing" => self.select(Existence::Existing),
            b"--canonicalize-missing" => self.select(Existence::AnyMayBeMissing),
            b"--quiet" => self.quiet = true,
            b"--silent" if command == Command::Readlink => self.quiet = true,
            b"--verbose" if command == Command::Readlink => self.quiet = false,
            b"--no-newline" if command == Command::Readlink => self.omit_delimiter = true,
            b"--zero" => self.delimiter = 0,
            _ if command == Command::Realpath => self.realpath_only(arg, args)?,
            _ => return Err(ParseError::unknown(arg)),
        }
        Ok(())
    }

    fn realpath_only(
        &mut self,
        arg: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        let bytes = arg.as_encoded_bytes();
        match bytes {
            b"--logical" => self.mode.order = Order::Logical,
            b"--physical" => self.mode.order = Order::Physical,
            b"--strip" | b"--no-symlinks" => self.mode.strip = true,
            _ if bytes == b"--relative-to" || bytes.starts_with(b"--relative-to=") => {
                self.relative_to = Some(long_value(arg, "--relative-to", args)?.into());
            }
            _ if bytes == b"--relative-base" || bytes.starts_with(b"--relative-base=") => {
                self.relative_base = Some(long_value(arg, "--relative-base", args)?.into());
            }
            _ => return Err(ParseError::unknown(arg)),
        }
        Ok(())
    }

    fn flag(&mut self, command: Command, flag: u8) -> Option<()> {
        let realpath = command == Command::Realpath;
        match flag {
            b'E' if realpath => self.select(Existence::FinalMayBeMissing),
            b'f' if !realpath => self.select(Existence::FinalMayBeMissing),
            b'e' => self.select(Existence::Existing),
            b'm' => self.select(Existence::AnyMayBeMissing),
            b'L' if realpath => self.mode.order = Order::Logical,
            b'P' if realpath => self.mode.order = Order::Physical,
            b's' if realpath => self.mode.strip = true,
            b'q' | b's' => self.quiet = true,
            b'v' if !realpath => self.quiet = false,
            b'n' if !realpath => self.omit_delimiter = true,
            b'z' => self.delimiter = 0,
            _ => return None,
        }
        Some(())
    }

    fn select(&mut self, existence: Existence) {
        self.link_target = false;
        self.mode.existence = existence;
    }
}

#[derive(Clone)]
struct Resolver {
    cwd: PathBuf,
    mode: Mode,
    link_target: bool,
    relative_to: Option<PathBuf>,
    relative_base: Option<PathBuf>,
}

impl Resolver {
    fn new(options: &Options) -> io::Result<Self> {
        let cwd = if options.link_target {
            PathBuf::new()
        } else {
            canonical::current_directory()?
        };
        let base = |path: Option<&Path>| {
            path.map(|path| canonical::resolve(path, &cwd, options.mode))
                .transpose()
        };
        let relative_to = base(options.relative_to.as_deref())?;
        let relative_base = base(options.relative_base.as_deref())?;
        Ok(Self {
            cwd,
            mode: options.mode,
            link_target: options.link_target,
            relative_to,
            relative_base,
        })
    }

    fn resolve(&self, operand: &Path) -> io::Result<PathBuf> {
        if self.link_target {
            return fs::read_link(operand);
        }
        let path = canonical::resolve(operand, &self.cwd, self.mode)?;
        if self.relative_base.as_ref().is_some_and(|base| {
            !path.starts_with(base)
                || self
                    .relative_to
                    .as_ref()
                    .is_some_and(|relative| !relative.starts_with(base))
        }) {
            return Ok(path);
        }
        // A relative route is possible only within the same filesystem root.
        let base = self.relative_to.as_ref().or(self.relative_base.as_ref());
        Ok(base
            .and_then(|base| canonical::relative(&path, base))
            .unwrap_or(path))
    }
}

#[allow(clippy::unnecessary_debug_formatting)] // Keep control characters in operands escaped.
pub(super) fn write(
    command: Command,
    mut options: Options,
    workers: jobs::Count,
    ui: Presentation,
    output: &mut impl Write,
) -> io::Result<u8> {
    let mut errors = io::stderr().lock();
    let delimiter = if options.omit_delimiter && options.paths.len() == 1 {
        None
    } else {
        if options.omit_delimiter {
            ui.warning(
                &mut errors,
                command.label(),
                "--no-newline requires a single operand",
            )?;
        }
        Some(options.delimiter)
    };
    let resolver = match Resolver::new(&options) {
        Ok(resolver) => resolver,
        Err(error) => {
            if !options.quiet {
                ui.error(&mut errors, command.label(), &error.to_string())?;
            }
            return Ok(1);
        }
    };
    let mut failed = false;
    let mut print = |operand: &OsStr, result: io::Result<PathBuf>| -> io::Result<()> {
        match result {
            Ok(path) => output_path(output, path.as_os_str(), delimiter),
            Err(error) => {
                failed = true;
                if !options.quiet {
                    ui.error(
                        &mut errors,
                        command.label(),
                        &format!("{operand:?}: {error}"),
                    )?;
                }
                Ok(())
            }
        }
    };
    let operands = std::mem::take(&mut options.paths);
    // Small requests avoid runtime startup. Larger batches dispatch independent
    // path lookups to bounded workers and retain operand order.
    if workers.get() == 1 || operands.len() < 8 {
        for operand in &operands {
            print(operand, resolver.resolve(Path::new(operand)))?;
        }
    } else {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(workers.get())
            .build()?;
        for (operand, result) in
            runtime.block_on(jobs::map(operands.into_vec(), workers, move |operand| {
                let result = resolver.resolve(Path::new(&operand));
                (operand, result)
            }))?
        {
            print(&operand, result)?;
        }
    }
    Ok(u8::from(failed))
}
