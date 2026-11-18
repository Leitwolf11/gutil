use std::ffi::OsStr;
use std::process::Command;

use super::ParseError;

pub(super) struct GitHubSource {
    owner: String,
    repository: String,
    revision: Option<Revision>,
}

enum Revision {
    Tag(String),
    Commit(String),
}

impl Revision {
    fn parse(value: &str) -> Result<Self, ParseError> {
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

impl GitHubSource {
    pub(super) fn parse(argument: &OsStr) -> Result<Self, ParseError> {
        let (owner, path) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix("github.com:"))
            .and_then(|path| path.split_once('/'))
            .ok_or(ParseError::Source)?;
        let (repository, revision) = match path.split_once('@') {
            Some((repository, revision)) => (repository, Some(Revision::parse(revision)?)),
            None => (path, None),
        };
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
            revision,
        })
    }

    pub(super) fn configure_cargo(&self, command: &mut Command) {
        command.arg("--git").arg(format!(
            "https://github.com/{}/{}",
            self.owner, self.repository
        ));
        match &self.revision {
            Some(Revision::Tag(tag)) => command.arg(format!("--tag={tag}")),
            Some(Revision::Commit(commit)) => command.arg(format!("--rev={commit}")),
            None => command,
        };
    }
}

#[cfg(test)]
mod tests;
