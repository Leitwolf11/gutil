use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use gutil_plugin_sdk::{PROTOCOL_ENV, PROTOCOL_VERSION};
use wasmi::{Engine, ExternType, Linker, Module, Store};
use wasmi_wasi::{ambient_authority, Dir, WasiCtxBuilder};

use crate::config;
use crate::presentation::Presentation;

mod source;

use source::InstallSource;

const LABEL: &str = "gutil plug-in";
const BINARY_PREFIX: &str = "gutil-";
const MODULE_SUFFIX: &str = ".wasm";
const WASI_TARGET: &str = "wasm32-wasip1";
const HELP: &str = "Usage: gutil plug-in install SOURCE [name:PACKAGE]\n       gutil plug-in use COMMAND [ARGUMENTS]...\n\nSources:\n  github.com:OWNER/REPO[@TAG|@COMMIT]\n  gitlab.com:GROUP/REPO[@TAG|@COMMIT]\n  bitbucket.org:WORKSPACE/REPO[@TAG|@COMMIT]\n  local:PATH\n  crates.io:PACKAGE[@VERSION]\n  sparse-index:HOST/PATH name:PACKAGE[@VERSION]\n  ALIAS:PACKAGE[@VERSION]  Read ALIAS from [registeries]\n\nUse @refs/tags/NAME for an explicit Git tag. The binary target must be\nnamed gutil-<command> and compile for wasm32-wasip1.\n";

enum Request {
    Help,
    Version,
    Install(InstallSource),
    Use(PluginCommand, Vec<OsString>),
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error(
        "expected 'install SOURCE [name:PACKAGE]' or 'use COMMAND' (see 'gutil plug-in --help')"
    )]
    Usage,
    #[error("invalid plug-in source (see 'gutil plug-in --help')")]
    Source,
    #[error("a sparse index requires name:PACKAGE after the index URL")]
    MissingPackage,
    #[error("expected a valid Git tag or commit hash after '@'")]
    Revision,
    #[error("a plug-in command must be a nonempty name that fits in one file name")]
    Command,
}

impl Request {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        registries: &config::Registries,
    ) -> Result<Self, ParseError> {
        let action = args.next().ok_or(ParseError::Usage)?;
        match action.to_str() {
            Some("-h" | "--help") => Ok(Self::Help),
            Some("--version") => Ok(Self::Version),
            Some("install") => {
                let source = args.next().ok_or(ParseError::Usage)?;
                let package = args.next();
                if args.next().is_some() {
                    return Err(ParseError::Usage);
                }
                InstallSource::parse(&source, package.as_deref(), registries).map(Self::Install)
            }
            Some(source) if source.starts_with("bitbucket.org:") && args.next().is_none() => {
                InstallSource::parse(&action, None, registries).map(Self::Install)
            }
            Some("use") => {
                let command = args.next().ok_or(ParseError::Usage)?;
                Ok(Self::Use(PluginCommand::parse(&command)?, args.collect()))
            }
            _ => Err(ParseError::Usage),
        }
    }
}

struct PluginCommand(String);

impl PluginCommand {
    fn parse(argument: &OsStr) -> Result<Self, ParseError> {
        let name = argument.to_str().ok_or(ParseError::Command)?;
        let filename = format!("{BINARY_PREFIX}{name}{MODULE_SUFFIX}");
        let path = Path::new(&filename);
        if name.is_empty()
            || name.contains('\0')
            || (cfg!(windows) && name.contains(':'))
            || path.file_name() != Some(path.as_os_str())
        {
            return Err(ParseError::Command);
        }
        Ok(Self(name.to_owned()))
    }

    fn module_name(&self) -> String {
        format!("{BINARY_PREFIX}{}{MODULE_SUFFIX}", self.0)
    }

    fn from_module_name(name: &OsStr) -> Result<Self, ParseError> {
        let command = name
            .to_str()
            .and_then(|name| name.strip_suffix(MODULE_SUFFIX))
            .and_then(|name| name.strip_prefix(BINARY_PREFIX))
            .ok_or(ParseError::Command)?;
        Self::parse(OsStr::new(command))
    }
}

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    registries: &config::Registries,
    ui: Presentation,
) -> io::Result<u8> {
    let result = match Request::parse(args, registries) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Install(source)) => install(&source),
        Ok(Request::Use(command, arguments)) => launch(&command, &arguments),
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

fn install(source: &InstallSource) -> io::Result<u8> {
    let directory = directory()?;
    let staging = tempfile::tempdir()?;
    let mut cargo = Command::new("cargo");
    cargo
        .current_dir(staging.path())
        .args(["install", "--locked", "--target", WASI_TARGET])
        .arg("--root")
        .arg(staging.path());
    source.configure_cargo(&mut cargo)?;
    let status = cargo
        .status()
        .map_err(|error| io::Error::new(error.kind(), format!("cannot run cargo: {error}")))?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "cargo install failed with {status}"
        )));
    }

    let (command, module_path) = installed_module(&staging.path().join("bin"))?;
    let bytes = fs::read(module_path)?;
    load_module(&Engine::default(), &bytes)?;
    let destination = directory.join(command.module_name());
    fs::create_dir_all(&directory)?;
    let mut output = tempfile::NamedTempFile::new_in(directory)?;
    output.write_all(&bytes)?;
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

fn installed_module(directory: &Path) -> io::Result<(PluginCommand, PathBuf)> {
    let invalid = || {
        io::Error::other(
            "the package must install exactly one WebAssembly module named gutil-<command>.wasm",
        )
    };
    let mut entries = fs::read_dir(directory)?;
    let entry = entries.next().ok_or_else(invalid)??;
    if entries.next().transpose()?.is_some() || !entry.file_type()?.is_file() {
        return Err(invalid());
    }
    let command = PluginCommand::from_module_name(&entry.file_name()).map_err(|_| invalid())?;
    Ok((command, entry.path()))
}

fn load_module(engine: &Engine, bytes: &[u8]) -> io::Result<Module> {
    let module = Module::new(engine, bytes).map_err(io::Error::other)?;
    if !matches!(module.get_export("_start"), Some(ExternType::Func(signature))
        if signature.params().is_empty() && signature.results().is_empty())
    {
        return Err(io::Error::other(
            "plug-in must export a WASI _start function",
        ));
    }
    Ok(module)
}

fn launch(command: &PluginCommand, arguments: &[OsString]) -> io::Result<u8> {
    let path = directory()?.join(command.module_name());
    if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("plug-in command '{}' is not installed", command.0),
        ));
    }
    let engine = Engine::default();
    let module = load_module(&engine, &fs::read(path)?)?;
    let mut context = WasiCtxBuilder::new();
    context.inherit_stdio();
    context
        .env(PROTOCOL_ENV, PROTOCOL_VERSION)
        .map_err(io::Error::other)?;
    context.arg(&command.0).map_err(io::Error::other)?;
    for argument in arguments {
        let argument = argument.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "WebAssembly plug-in arguments must be valid Unicode",
            )
        })?;
        context.arg(argument).map_err(io::Error::other)?;
    }
    context
        .preopened_dir(Dir::open_ambient_dir(".", ambient_authority())?, ".")
        .map_err(io::Error::other)?;

    let mut store = Store::new(&engine, context.build());
    let mut linker = Linker::new(&engine);
    wasmi_wasi::add_to_linker(&mut linker, |context| context).map_err(io::Error::other)?;
    let result = linker
        .instantiate_and_start(&mut store, &module)
        .and_then(|instance| {
            instance
                .get_typed_func::<(), ()>(&store, "_start")?
                .call(&mut store, ())
        });
    match result {
        Ok(()) => Ok(0),
        Err(error) => match error.i32_exit_status() {
            Some(status) => Ok(u8::try_from(status).unwrap_or(1)),
            None => Err(io::Error::other(error)),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn validates_the_cargo_install_output() {
        let staging = tempfile::tempdir().unwrap();
        assert!(installed_module(staging.path()).is_err());
        let invalid = staging.path().join("other");
        fs::write(&invalid, b"").unwrap();
        assert!(installed_module(staging.path()).is_err());
        let command = PluginCommand::parse(OsStr::new("example")).unwrap();
        let binary = staging.path().join(command.module_name());
        fs::write(&binary, b"").unwrap();
        assert!(installed_module(staging.path()).is_err());
        fs::remove_file(invalid).unwrap();
        let (_, selected) = installed_module(staging.path()).unwrap();
        assert_eq!(selected, binary);
    }
    #[test]
    fn validates_the_wasi_entry_point() {
        let engine = Engine::default();
        assert!(load_module(&engine, b"invalid").is_err());
        let empty_module = b"\0asm\x01\0\0\0";
        let error = load_module(&engine, empty_module).err().unwrap();
        assert_eq!(
            error.to_string(),
            "plug-in must export a WASI _start function"
        );
    }
}
