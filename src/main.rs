mod cat;
mod config;
mod jobs;
mod ls;
mod presentation;
mod pwd;

use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use config::{Source, When};
use presentation::Presentation;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::process::ExitCode;

const HELP: &str = "Usage: gutil [OPTIONS] <COMMAND> [ARGUMENTS]\n\nCommands:\n  cat  Concatenate files\n  ls   List directory contents\n  pwd  Print the working directory\n\nOptions:\n  -h, --help     Print help\n  -V, --version  Print version\n  -j, --jobs, --threads N        Limit concurrent filesystem jobs\n      --config PATH              Read a specific TOML file\n      --no-config                Skip configuration lookup\n      --profile NAME             Apply a named configuration profile\n      --progress=MODE            auto, never, or always\n      --color=MODE               auto, never, or always\n      --hyperlinks=MODE          auto, never, or always\n";

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
    jobs: Option<usize>,
    progress: Option<When>,
    color: Option<When>,
    hyperlinks: Option<When>,
}

fn run(mut args: impl Iterator<Item = OsString>) -> io::Result<u8> {
    let mut options = GlobalOptions::default();
    let command = loop {
        let Some(argument) = args.next() else {
            break None;
        };
        match options.consume(&argument, &mut args) {
            Ok(true) => {}
            Ok(false) => break Some(argument),
            Err(error) => return writeln!(io::stderr(), "gutil: {error}").map(|()| 2),
        }
    };
    let command = command.as_deref().and_then(OsStr::to_str);
    match command {
        Some("-h" | "--help") => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Some("-V" | "--version") => {
            return writeln!(io::stdout(), "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0)
        }
        _ => {}
    }

    let loaded = match config::load(options.source, options.profile.as_deref()) {
        Ok(loaded) => loaded,
        Err(error) => return writeln!(io::stderr(), "gutil: {error}").map(|()| 2),
    };
    let mut settings = loaded.settings;
    if let Some(mode) = options.progress {
        settings.global.progress = mode;
    }
    if let Some(mode) = options.color {
        settings.output.color = mode;
    }
    if let Some(mode) = options.hyperlinks {
        settings.output.hyperlinks = mode;
    }
    let job_count = options.jobs.unwrap_or_else(|| settings.global.job_count());
    let ui = Presentation::new(settings);
    if settings.diagnostics.verbosity == config::Verbosity::Verbose {
        let source = loaded.path.as_ref().map_or_else(
            || "built-in defaults".into(),
            |path| path.display().to_string(),
        );
        let profile = options.profile.as_deref().unwrap_or("none");
        ui.context(
            &mut io::stderr(),
            &format!("settings: {source}; profile: {profile}; jobs: {job_count}"),
        )?;
    }

    if command == Some("cat") {
        return match cat::run(args, job_count, settings.cat, ui) {
            Err(error) => {
                ui.error(&mut io::stderr(), "gutil cat", &error.to_string())?;
                Ok(1)
            }
            result => result,
        };
    }

    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    let mut errors = io::stderr().lock();
    let result = match command {
        Some("ls") => ls::run(
            args,
            job_count,
            &settings,
            stdout.is_terminal(),
            &mut output,
            &mut errors,
        ),
        Some("pwd") => pwd::run(args, job_count, settings.pwd, ui, &mut output, &mut errors),
        _ => ui
            .error(
                &mut errors,
                "gutil",
                "expected a command (see 'gutil --help')",
            )
            .map(|()| 2),
    };
    match result.and_then(|status| output.flush().map(|()| status)) {
        Err(error) if command == Some("pwd") => {
            ui.error(&mut errors, "gutil pwd", &error.to_string())?;
            Ok(1)
        }
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            ui.error(&mut errors, "gutil ls", &error.to_string())?;
            Ok(1)
        }
        result => result,
    }
}

impl GlobalOptions {
    fn consume(
        &mut self,
        argument: &OsStr,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<bool, String> {
        if let Some(count) = jobs::option(argument.as_encoded_bytes(), args) {
            self.jobs = Some(count.map_err(|error| error.to_string())?);
            return Ok(true);
        }
        if argument == "--config" {
            self.source = Source::Explicit(PathBuf::from(
                args.next().ok_or("--config requires a path")?,
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
                self.source =
                    Source::Explicit(PathBuf::from(attached.ok_or("--config requires a path")?));
            }
            "--no-config" if attached.is_none() => self.source = Source::Disabled,
            "--profile" => {
                self.profile = Some(match attached {
                    Some(value) => value.to_owned(),
                    None => args
                        .next()
                        .and_then(|value| value.into_string().ok())
                        .ok_or("--profile requires a name")?,
                });
            }
            "--progress" => self.progress = Some(parse_when(option, attached)?),
            "--color" => self.color = Some(parse_when(option, attached)?),
            "--hyperlinks" => self.hyperlinks = Some(parse_when(option, attached)?),
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn parse_when(option: &str, value: Option<&str>) -> Result<When, String> {
    match value {
        Some("auto") => Ok(When::Auto),
        Some("never") => Ok(When::Never),
        Some("always") => Ok(When::Always),
        _ => Err(format!("{option} expects auto, never, or always")),
    }
}
