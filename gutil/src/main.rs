mod cache;
mod cat;
mod checksum;
mod config;
mod disk;
mod edges;
mod encoding;
mod fd;
mod file_size;
mod fold;
mod jobs;
mod ls;
mod mkdir;
mod mktemp;
mod mutate;
mod paste;
mod paths;
mod plugin;
mod presentation;
mod pwd;
mod shell;
mod tee;
mod tty;
mod uniq;
mod wc;

use std::cell::LazyCell;
use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use config::{Source, When};
use presentation::Presentation;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

const HELP: &str = "Usage: gutil [OPTIONS] <COMMAND> [ARGUMENTS]\n\nCommands:\n  arch, uname  Report machine and system information\n  base32, base64  Encode or decode data\n  df, duf  Report mounted filesystem usage\n  fd       Find files and directories\n  fold     Wrap input lines\n  link, ln, unlink  Create links or remove file names\n  mkdir    Create directories\n  mktemp   Create temporary files or directories\n  rmdir    Remove empty directories\n  truncate  Adjust file sizes\n  basename  Print the final component of a path\n  dirname   Print the directory portion of a path\n  readlink  Print symbolic link targets\n  realpath  Print resolved paths\n  cache    Store and retrieve content by digest\n  cat      Concatenate files\n  paste    Merge corresponding records from files\n  head     Print the beginning of files\n  tail     Print the end of files\n  tee      Copy standard input to files and standard output\n  uniq     Filter adjacent repeated records\n  wc       Count lines, words, and bytes\n  md5sum   Print or verify MD5 checksums\n  sha1sum  Print or verify SHA-1 checksums\n  sha224sum, sha256sum, sha384sum, sha512sum  Print or verify SHA-2 checksums\n  ls       List directory contents\n  plug-in  Install or run external plug-ins\n  pwd      Print the working directory\n  echo, yes          Write text\n  false, true        Return a fixed exit status\n  nproc, printenv    Report processor or environment information\n  sleep              Pause for a duration\n  tty                Report the standard input terminal\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n  -j, --jobs, --threads N        Limit concurrent filesystem jobs\n      --config PATH              Read a specific TOML file\n      --no-config                Skip configuration lookup\n      --profile NAME             Apply a named configuration profile\n      --progress=MODE            auto, never, or always\n      --color=MODE               auto, never, or always\n      --hyperlinks=MODE          auto, never, or always\n";

fn main() -> ExitCode {
    // Discard argv[0] and preserve non-UTF-8 filesystem operands.
    let args = env::args_os().skip(1); // nosemgrep: args-os, rust.lang.security.args-os.args-os
    match run(args) {
        Ok(status) => ExitCode::from(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr(), "gutil: {error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Default)]
struct GlobalOptions {
    source: Source,
    profile: Option<String>,
    jobs: Option<jobs::Count>,
    progress: Option<When>,
    color: Option<When>,
    hyperlinks: Option<When>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Cache,
    Cat,
    Checksum(checksum::Algorithm),
    Edge(edges::Edge),
    Encoding(encoding::Base),
    Disk(disk::Command),
    Fd,
    Fold,
    Ls,
    Mkdir,
    Mktemp,
    Mutate(mutate::Command),
    Paste,
    Path(paths::Command),
    Plugin,
    Pwd,
    Shell(shell::Command),
    Tee,
    Tty,
    Uniq,
    Wc,
    Help,
    Version,
    Unknown,
}

impl Command {
    fn parse(argument: &OsStr) -> Self {
        match argument.to_str() {
            Some("base32") => Self::Encoding(encoding::Base::Base32),
            Some("base64") => Self::Encoding(encoding::Base::Base64),
            Some("basename") => Self::Path(paths::Command::Basename),
            Some("dirname") => Self::Path(paths::Command::Dirname),
            Some("readlink") => Self::Path(paths::Command::Readlink),
            Some("realpath") => Self::Path(paths::Command::Realpath),
            Some("cache") => Self::Cache,
            Some("cat") => Self::Cat,
            Some("df") => Self::Disk(disk::Command::Df),
            Some("duf") => Self::Disk(disk::Command::Duf),
            Some("fd") => Self::Fd,
            Some("fold") => Self::Fold,
            Some("head") => Self::Edge(edges::Edge::Head),
            Some("tail") => Self::Edge(edges::Edge::Tail),
            Some("ls") => Self::Ls,
            Some("mkdir") => Self::Mkdir,
            Some("mktemp") => Self::Mktemp,
            Some("ln") => Self::Mutate(mutate::Command::Ln),
            Some("link") => Self::Mutate(mutate::Command::Link),
            Some("unlink") => Self::Mutate(mutate::Command::Unlink),
            Some("rmdir") => Self::Mutate(mutate::Command::Rmdir),
            Some("truncate") => Self::Mutate(mutate::Command::Truncate),
            Some("paste") => Self::Paste,
            Some("plug-in") => Self::Plugin,
            Some("pwd") => Self::Pwd,
            Some("tee") => Self::Tee,
            Some("tty") => Self::Tty,
            Some("uniq") => Self::Uniq,
            Some("wc") => Self::Wc,
            Some("-h" | "--help") => Self::Help,
            Some("-V" | "--version") => Self::Version,
            _ => checksum::Algorithm::parse(argument)
                .map(Self::Checksum)
                .or_else(|| shell::Command::parse(argument).map(Self::Shell))
                .unwrap_or(Self::Unknown),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum GlobalParseError {
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
    #[error("--config requires a path")]
    MissingConfig,
    #[error("--profile requires a name")]
    MissingProfile,
    #[error("{0} expects auto, never, or always")]
    InvalidMode(&'static str),
}

fn run(args: impl Iterator<Item = OsString>) -> io::Result<u8> {
    let mut args = args.peekable();
    let (mut options, command) = match GlobalOptions::parse(&mut args) {
        Ok(parsed) => parsed,
        Err(error) => return startup_error(&error.to_string()),
    };
    match command {
        Command::Help => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Command::Version => {
            return writeln!(io::stdout(), "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        _ => {}
    }

    // Direct help and version requests remain available when a config cannot load.
    let offline = command == Command::Cache && args.next_if(|arg| arg == "--offline").is_some();
    if direct_help_or_version(command, args.peek().and_then(|arg| arg.to_str())) {
        options.source = Source::Disabled;
        options.profile = None;
    }
    let mut loaded = match config::load(options.source, options.profile.as_deref()) {
        Ok(loaded) => loaded,
        Err(error) => return startup_error(&error),
    };
    let settings = &mut loaded.settings;
    if let Some(mode) = options.progress {
        settings.global.progress = mode;
    }
    if let Some(mode) = options.color {
        settings.output.color = mode;
    }
    if let Some(mode) = options.hyperlinks {
        settings.output.hyperlinks = mode;
    }
    let job_count = LazyCell::new(|| options.jobs.unwrap_or_else(|| settings.global.job_count()));
    let ui = Presentation::new(settings);
    if settings.diagnostics.verbosity == config::Verbosity::Verbose {
        report_settings(
            loaded.path.as_ref(),
            options.profile.as_deref(),
            *job_count,
            ui,
        )?;
    }

    match command {
        Command::Cat => {
            return match cat::run(args, *job_count, settings.cat, ui) {
                Err(error) => {
                    ui.error(&mut io::stderr(), "gutil cat", &error.to_string())?;
                    Ok(1)
                }
                result => result,
            }
        }
        Command::Plugin => return plugin::run(args, ui),
        Command::Mkdir => return mkdir::run(args, *job_count, ui),
        Command::Mktemp => return mktemp::run(args, *job_count, ui),
        Command::Mutate(command) => return mutate::run(command, args, *job_count, ui),
        Command::Paste => return paste::run(args, *job_count, ui),
        Command::Path(command) => return paths::run(command, args, *job_count, ui),
        Command::Checksum(algorithm) => return checksum::run(algorithm, args, *job_count, ui),
        Command::Edge(edge) => return edges::run(edge, args, *job_count, ui),
        Command::Encoding(base) => return encoding::run(base, args, *job_count, ui),
        Command::Tee => return tee::run(args, *job_count, ui),
        Command::Uniq => return uniq::run(args, *job_count, ui),
        Command::Wc => return wc::run(args, *job_count, ui),
        Command::Shell(command) => return shell::run(command, args, ui),
        Command::Tty => return Ok(tty::run(args, ui)),
        Command::Disk(command) => {
            return disk::run(command, args, *job_count, settings.output.color, ui)
        }
        Command::Fd => return fd::run(args, *job_count, ui),
        Command::Fold => return fold::run(args, *job_count, ui),
        _ => {}
    }

    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();
    let result = match command {
        Command::Cache => cache::run(args, offline, &loaded, ui, &mut output, &mut errors),
        Command::Ls => ls::run(
            args,
            *job_count,
            settings,
            stdout.is_terminal(),
            &mut output,
            &mut errors,
        ),
        Command::Pwd => pwd::run(args, *job_count, settings.pwd, ui, &mut output, &mut errors),
        _ => ui
            .error(
                &mut errors,
                "gutil",
                "expected a command (see 'gutil --help')",
            )
            .map(|()| 2),
    };
    finish_output(
        command,
        result.and_then(|status| output.flush().map(|()| status)),
        ui,
        &mut errors,
    )
}

fn finish_output(
    command: Command,
    result: io::Result<u8>,
    ui: Presentation,
    errors: &mut impl Write,
) -> io::Result<u8> {
    match result {
        Err(error)
            if matches!(command, Command::Cache | Command::Pwd)
                || error.kind() != io::ErrorKind::BrokenPipe =>
        {
            let label = match command {
                Command::Cache => "gutil cache",
                Command::Pwd => "gutil pwd",
                _ => "gutil ls",
            };
            ui.error(errors, label, &error.to_string())?;
            Ok(1)
        }
        result => result,
    }
}

fn report_settings(
    path: Option<&PathBuf>,
    profile: Option<&str>,
    job_count: jobs::Count,
    ui: Presentation,
) -> io::Result<()> {
    let source = path.map_or_else(
        || "built-in defaults".into(),
        |path| path.display().to_string(),
    );
    let profile = profile.unwrap_or("none");
    ui.context(
        &mut io::stderr(),
        &format!(
            "settings: {source}; profile: {profile}; jobs: {}",
            job_count.get()
        ),
    )
}

fn direct_help_or_version(command: Command, option: Option<&str>) -> bool {
    matches!(
        command,
        Command::Cache
            | Command::Cat
            | Command::Checksum(_)
            | Command::Disk(_)
            | Command::Edge(_)
            | Command::Encoding(_)
            | Command::Fd
            | Command::Fold
            | Command::Ls
            | Command::Mkdir
            | Command::Mktemp
            | Command::Mutate(_)
            | Command::Paste
            | Command::Path(_)
            | Command::Plugin
            | Command::Pwd
            | Command::Shell(_)
            | Command::Tee
            | Command::Tty
            | Command::Uniq
            | Command::Wc
    ) && (matches!(option, Some("--help" | "--version"))
        || (matches!(command, Command::Cache | Command::Plugin | Command::Fd)
            && option == Some("-h")
            || (command == Command::Fd && option == Some("-V"))))
}

impl GlobalOptions {
    fn parse(
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(Self, Command), GlobalParseError> {
        let mut options = Self::default();
        while let Some(argument) = args.next() {
            if !options.consume(&argument, args)? {
                return Ok((options, Command::parse(&argument)));
            }
        }
        Ok((options, Command::Unknown))
    }

    fn consume(
        &mut self,
        argument: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<bool, GlobalParseError> {
        if let Some(count) = jobs::option(argument.as_encoded_bytes(), args) {
            self.jobs = Some(count?);
            return Ok(true);
        }
        if argument == "--config" {
            self.source = Source::Explicit(PathBuf::from(
                args.next().ok_or(GlobalParseError::MissingConfig)?,
            ));
            return Ok(true);
        }
        let Some(argument) = argument.to_str() else {
            return Ok(false);
        };
        let (option, attached) = argument
            .split_once('=')
            .map_or((argument, None), |(key, value)| (key, Some(value)));
        match option {
            "--config" => {
                self.source = Source::Explicit(PathBuf::from(
                    attached.ok_or(GlobalParseError::MissingConfig)?,
                ));
            }
            "--no-config" if attached.is_none() => self.source = Source::Disabled,
            "--profile" => {
                self.profile = Some(match attached {
                    Some(value) => value.to_owned(),
                    None => args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or(GlobalParseError::MissingProfile)?,
                });
            }
            "--progress" => self.progress = Some(parse_when("--progress", attached)?),
            "--color" => self.color = Some(parse_when("--color", attached)?),
            "--hyperlinks" => self.hyperlinks = Some(parse_when("--hyperlinks", attached)?),
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn parse_when(option: &'static str, value: Option<&str>) -> Result<When, GlobalParseError> {
    value
        .and_then(|value| config::option_value(value).ok())
        .ok_or(GlobalParseError::InvalidMode(option))
}

fn startup_error(message: &str) -> io::Result<u8> {
    Presentation::new(&config::Settings::default())
        .error(&mut io::stderr(), "gutil", message)
        .map(|()| 2)
}
