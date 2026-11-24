use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use std::process::Command;

use reqwest::Url;

use super::ParseError;
use crate::config::Registries;

pub(super) enum InstallSource {
    Git(GitSource),
    Local(PathBuf),
    Registry { source: Registry, package: OsString },
}

pub(super) enum Registry {
    CratesIo,
    Sparse(String),
}

pub(super) struct GitSource {
    url: Url,
    revision: Option<Revision>,
}

enum Revision {
    Tag(String),
    Commit(String),
}

impl Revision {
    fn parse(value: &str) -> Result<Self, ParseError> {
        if let Some(tag) = value.strip_prefix("refs/tags/") {
            return if valid_tag(tag) {
                Ok(Self::Tag(tag.to_owned()))
            } else {
                Err(ParseError::Revision)
            };
        }
        if (7..=40).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(Self::Commit(value.to_ascii_lowercase()));
        }
        if valid_tag(value) {
            return Ok(Self::Tag(value.to_owned()));
        }
        Err(ParseError::Revision)
    }
}

#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "Git refname rules reserve the case-sensitive .lock suffix."
)]
fn valid_tag(value: &str) -> bool {
    !value.is_empty()
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains("@{")
        && !value.bytes().any(|byte| {
            byte <= b' '
                || byte == 127
                || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
        })
        && value.split('/').all(|component| {
            !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
        })
}

impl InstallSource {
    pub(super) fn parse(
        argument: &OsStr,
        package: Option<&OsStr>,
        registries: &Registries,
    ) -> Result<Self, ParseError> {
        const LOCAL: &str = "local:";
        const CRATES: &str = "crates.io:";
        let bytes = argument.as_encoded_bytes();
        if bytes.starts_with(LOCAL.as_bytes()) && package.is_none() {
            return Ok(Self::Local(PathBuf::from(required_suffix(
                argument, LOCAL,
            )?)));
        }
        if bytes.starts_with(CRATES.as_bytes()) && package.is_none() {
            return Ok(Self::Registry {
                source: Registry::CratesIo,
                package: required_suffix(argument, CRATES)?,
            });
        }
        let (kind, source) = argument
            .to_str()
            .and_then(|value| value.split_once(':'))
            .ok_or(ParseError::Source)?;
        match kind {
            "sparse-index" => {
                let package = package
                    .filter(|name| name.as_encoded_bytes().starts_with(b"name:"))
                    .ok_or(ParseError::MissingPackage)?;
                let package =
                    required_suffix(package, "name:").map_err(|_| ParseError::MissingPackage)?;
                Ok(Self::Registry {
                    source: Registry::Sparse(sparse_index(source)?),
                    package,
                })
            }
            "github.com" | "gitlab.com" | "bitbucket.org" if package.is_none() => {
                GitSource::parse(kind, source).map(Self::Git)
            }
            alias if package.is_none() => {
                let index = registries.get(alias).ok_or(ParseError::Source)?;
                Ok(Self::Registry {
                    source: Registry::Sparse(sparse_index(index)?),
                    package: required_suffix(argument, &format!("{alias}:"))?,
                })
            }
            _ => Err(ParseError::Source),
        }
    }

    pub(super) fn configure_cargo(&self, command: &mut Command) -> io::Result<()> {
        match self {
            Self::Git(source) => source.configure_cargo(command),
            Self::Local(path) => {
                command.arg("--path").arg(std::path::absolute(path)?);
            }
            Self::Registry { source, package } => {
                match source {
                    Registry::CratesIo => command.args(["--registry", "crates-io"]),
                    Registry::Sparse(index) => command.arg("--index").arg(index),
                };
                command.arg("--").arg(package);
            }
        }
        Ok(())
    }
}

fn required_suffix(argument: &OsStr, prefix: &str) -> Result<OsString, ParseError> {
    let suffix = native_suffix(argument, prefix.len())?;
    if suffix.is_empty() {
        return Err(ParseError::Source);
    }
    Ok(suffix)
}

#[cfg(unix)]
fn native_suffix(argument: &OsStr, start: usize) -> Result<OsString, ParseError> {
    use std::os::unix::ffi::OsStrExt;

    argument
        .as_bytes()
        .get(start..)
        .map(OsStr::from_bytes)
        .map(OsStr::to_owned)
        .ok_or(ParseError::Source)
}

#[cfg(windows)]
fn native_suffix(argument: &OsStr, start: usize) -> Result<OsString, ParseError> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    let units: Vec<_> = argument.encode_wide().collect();
    units
        .get(start..)
        .map(OsString::from_wide)
        .ok_or(ParseError::Source)
}

#[cfg(not(any(unix, windows)))]
fn native_suffix(argument: &OsStr, start: usize) -> Result<OsString, ParseError> {
    argument
        .to_str()
        .and_then(|value| value.get(start..))
        .map(OsString::from)
        .ok_or(ParseError::Source)
}

fn sparse_index(value: &str) -> Result<String, ParseError> {
    let value = value.strip_prefix("sparse+").unwrap_or(value);
    let url = if value.contains("://") {
        Url::parse(value)
    } else {
        Url::parse(&format!("https://{value}"))
    };
    let mut url = url.map_err(|_| ParseError::Source)?;
    if value.is_empty()
        || !matches!(url.scheme(), "http" | "https")
        || !url.has_host()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ParseError::Source);
    }
    if !url.path().ends_with('/') {
        url.path_segments_mut()
            .map_err(|()| ParseError::Source)?
            .push("");
    }
    Ok(format!("sparse+{url}"))
}

impl GitSource {
    fn parse(host: &str, path: &str) -> Result<Self, ParseError> {
        let (repository, revision) = match path.split_once('@') {
            Some((repository, revision)) => (repository, Some(Revision::parse(revision)?)),
            None => (path, None),
        };
        let segments: Vec<_> = repository.split('/').collect();
        if segments.len() < 2
            || (host != "gitlab.com" && segments.len() != 2)
            || segments
                .iter()
                .any(|segment| matches!(*segment, "" | "." | ".."))
        {
            return Err(ParseError::Source);
        }
        let mut url = Url::parse(&format!("https://{host}/")).map_err(|_| ParseError::Source)?;
        url.path_segments_mut()
            .map_err(|()| ParseError::Source)?
            .pop_if_empty()
            .extend(segments);
        Ok(Self { url, revision })
    }

    fn configure_cargo(&self, command: &mut Command) {
        command.arg("--git").arg(self.url.as_str());
        match &self.revision {
            Some(Revision::Tag(tag)) => command.arg(format!("--tag={tag}")),
            Some(Revision::Commit(commit)) => command.arg(format!("--rev={commit}")),
            None => command,
        };
    }
}

#[cfg(test)]
mod tests;
