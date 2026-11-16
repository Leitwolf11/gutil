use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::config;
use crate::presentation::Presentation;

const LABEL: &str = "gutil plug-in";
const EXECUTABLE_PREFIX: &str = "gutil-";
const PROTOCOL_ENV: &str = "GUTIL_PLUGIN_PROTOCOL";
const PROTOCOL_VERSION: &str = "1";
const HELP: &str = "Usage: gutil plug-in install github.com:OWNER/EXTENSION\n       gutil plug-in use COMMAND [ARGUMENTS]...\n\nInstall a Rust plug-in from GitHub or run an installed plug-in.\nThe binary target must be named gutil-<command>.\n";

enum Request {
    Help,
    Version,
    Install(GitHubSource),
    Use(PluginCommand, Vec<OsString>),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("expected 'install github.com:OWNER/EXTENSION' or 'use COMMAND' (see 'gutil plug-in --help')")]
    Usage,
    #[error("expected a GitHub source in the form github.com:OWNER/EXTENSION")]
    Source,
    #[error("plug-in command names must start with a lowercase ASCII letter and contain only lowercase ASCII letters, digits, and hyphens")]
    Command,
}

impl Request {
    fn parse(mut args: impl Iterator<Item = OsString>) -> Result<Self, ParseError> {
        match args.next().as_deref().and_then(OsStr::to_str) {
            Some("-h" | "--help") => Ok(Self::Help),
            Some("--version") => Ok(Self::Version),
            Some("install") => match (args.next(), args.next()) {
                (Some(source), None) => GitHubSource::parse(&source).map(Self::Install),
                _ => Err(ParseError::Usage),
            },
            Some("use") => {
                let command = args.next().ok_or(ParseError::Usage)?;
                Ok(Self::Use(PluginCommand::parse(&command)?, args.collect()))
            }
            _ => Err(ParseError::Usage),
        }
    }
}

struct GitHubSource {
    owner: String,
    repository: String,
}

impl GitHubSource {
    fn parse(argument: &OsStr) -> Result<Self, ParseError> {
        let (owner, repository) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("github.com:"))
            .and_then(|path| path.split_once('/'))
            .ok_or(ParseError::Source)?;
        if owner.is_empty()
            || !owner
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || owner.starts_with('-')
            || owner.ends_with('-')
            || repository.is_empty()
            || matches!(repository, "." | "..")
            || !repository
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ParseError::Source);
        }
        Ok(Self {
            owner: owner.to_owned(),
            repository: repository.to_owned(),
        })
    }

    fn url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repository)
    }
}

struct PluginCommand(String);

impl PluginCommand {
    fn parse(argument: &OsStr) -> Result<Self, ParseError> {
        let name = argument.to_str().ok_or(ParseError::Command)?;
        if name.starts_with(|character: char| character.is_ascii_lowercase())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            Ok(Self(name.to_owned()))
        } else {
            Err(ParseError::Command)
        }
    }

    fn executable(&self) -> String {
        format!("{EXECUTABLE_PREFIX}{}{}", self.0, env::consts::EXE_SUFFIX)
    }

    fn from_executable(name: &OsStr) -> Result<Self, ParseError> {
        let command = name
            .to_str()
            .and_then(|name| name.strip_suffix(env::consts::EXE_SUFFIX))
            .and_then(|name| name.strip_prefix(EXECUTABLE_PREFIX))
            .ok_or(ParseError::Command)?;
        Self::parse(OsStr::new(command))
    }
}

pub(crate) fn run(args: impl Iterator<Item = OsString>, ui: Presentation) -> io::Result<u8> {
    let result = match Request::parse(args) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Install(source)) => install(&source),
        Ok(Request::Use(command, arguments)) => launch(&command, arguments),
        Err(error) => {
            return ui
                .error(&mut io::stderr(), LABEL, &error.to_string())
                .map(|()| 2)
        }
    };
    match result {
        Ok(status) => Ok(status),
        Err(error) => ui
            .error(&mut io::stderr(), LABEL, &error.to_string())
            .map(|()| 1),
    }
}

fn directory() -> io::Result<PathBuf> {
    let home = config::home_dir()
        .filter(|path| path.is_absolute())
        .ok_or_else(|| io::Error::other("plug-ins require an absolute home directory"))?;
    Ok(home.join(".gutil").join("plugins"))
}

fn install(source: &GitHubSource) -> io::Result<u8> {
    let directory = directory()?;
    let staging = tempfile::tempdir()?;
    let status = Command::new("cargo")
        .current_dir(staging.path())
        .args(["install", "--locked", "--git", &source.url(), "--root"])
        .arg(staging.path())
        .status()
        .map_err(|error| io::Error::new(error.kind(), format!("cannot run cargo: {error}")))?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo install failed with {status}"
        )));
    }

    let (command, executable) = installed_binary(&staging.path().join("bin"))?;
    let destination = directory.join(command.executable());
    fs::create_dir_all(&directory)?;
    let mut input = File::open(executable)?;
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    io::copy(&mut input, &mut output)?;
    output
        .as_file()
        .set_permissions(input.metadata()?.permissions())?;
    output.persist_noclobber(destination).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("plug-in command '{}' is already installed", command.0),
            )
        } else {
            error.error
        }
    })?;
    writeln!(io::stdout(), "Installed {}", command.0)?;
    Ok(0)
}

fn installed_binary(directory: &Path) -> io::Result<(PluginCommand, PathBuf)> {
    let invalid =
        || io::Error::other("the package must install exactly one binary named gutil-<command>");
    let mut entries = fs::read_dir(directory)?;
    let entry = entries.next().ok_or_else(invalid)??;
    if entries.next().transpose()?.is_some() || !entry.file_type()?.is_file() {
        return Err(invalid());
    }
    let command = PluginCommand::from_executable(&entry.file_name()).map_err(|_| invalid())?;
    Ok((command, entry.path()))
}

fn launch(command: &PluginCommand, arguments: Vec<OsString>) -> io::Result<u8> {
    let executable = directory()?.join(command.executable());
    if !fs::symlink_metadata(&executable).is_ok_and(|metadata| metadata.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plug-in command '{}' is not installed", command.0),
        ));
    }
    let status = Command::new(executable)
        .args(arguments)
        .env(PROTOCOL_ENV, PROTOCOL_VERSION)
        .status()?;
    Ok(exit_code(status))
}

fn exit_code(status: ExitStatus) -> u8 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return u8::try_from(128 + signal).unwrap_or(1);
        }
    }
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_cargo_install_output() {
        let staging = tempfile::tempdir().unwrap();
        assert!(installed_binary(staging.path()).is_err());
        let invalid = staging.path().join("other");
        fs::write(&invalid, b"").unwrap();
        assert!(installed_binary(staging.path()).is_err());
        let command = PluginCommand::parse(OsStr::new("example")).unwrap();
        let binary = staging.path().join(command.executable());
        fs::write(&binary, b"").unwrap();
        assert!(installed_binary(staging.path()).is_err());
        fs::remove_file(invalid).unwrap();
        let (_, selected) = installed_binary(staging.path()).unwrap();
        assert_eq!(selected, binary);
    }

    #[cfg(unix)]
    #[test]
    fn maps_signal_exit_status() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(exit_code(ExitStatus::from_raw(15)), 143);
    }
}
