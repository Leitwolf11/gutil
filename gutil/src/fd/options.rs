use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use globset::{GlobBuilder, GlobMatcher};
use ignore::{overrides::OverrideBuilder, DirEntry, WalkBuilder};
use regex::bytes::{Regex, RegexBuilder};

use crate::jobs;

pub(super) enum Request {
    Help,
    Version,
    Search(Box<Options>),
}

#[derive(Default, Clone, Copy)]
enum Case {
    #[default]
    Smart,
    Sensitive,
    Insensitive,
}

#[derive(Default)]
enum Pattern {
    #[default]
    Regex,
    Glob,
    Fixed,
    Exact,
}

struct Ignore {
    dotfiles: bool,
    files: bool,
    vcs: bool,
    scope: IgnoreScope,
}

impl Default for Ignore {
    fn default() -> Self {
        Self {
            dotfiles: true,
            files: true,
            vcs: true,
            scope: IgnoreScope {
                parent: true,
                require_git: true,
            },
        }
    }
}

struct IgnoreScope {
    parent: bool,
    require_git: bool,
}

#[derive(Default)]
struct PathOptions {
    full: bool,
    absolute: bool,
    follow: bool,
}

pub(super) struct Output {
    pub(super) delimiter: u8,
    pub(super) quiet: bool,
    pub(super) show_errors: bool,
    pub(super) strip_implicit_dot: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileType {
    File,
    Directory,
    Symlink,
}

pub(super) struct Options {
    pattern: Option<OsString>,
    pub(super) roots: Vec<PathBuf>,
    pub(super) workers: jobs::Count,
    pub(super) output: Output,
    path: PathOptions,
    case: Case,
    syntax: Pattern,
    ignore: Ignore,
    min_depth: usize,
    max_depth: Option<usize>,
    types: Vec<FileType>,
    extensions: Vec<OsString>,
    excludes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("unrecognized option {0:?}")]
    Unknown(OsString),
    #[error("{0} requires an argument")]
    Missing(&'static str),
    #[error("invalid value for {0}")]
    Value(&'static str),
    #[error("minimum depth exceeds maximum depth")]
    Depth,
    #[error(transparent)]
    Workers(#[from] jobs::InvalidCount),
    #[error(transparent)]
    Path(#[from] std::io::Error),
    #[error(transparent)]
    Regex(#[from] regex::Error),
    #[error(transparent)]
    Glob(#[from] globset::Error),
}

impl Options {
    fn new(workers: jobs::Count) -> Self {
        Self {
            pattern: None,
            roots: Vec::new(),
            workers,
            output: Output {
                delimiter: b'\n',
                quiet: false,
                show_errors: false,
                strip_implicit_dot: false,
            },
            path: PathOptions::default(),
            case: Case::Smart,
            syntax: Pattern::Regex,
            ignore: Ignore::default(),
            min_depth: 0,
            max_depth: None,
            types: Vec::new(),
            extensions: Vec::new(),
            excludes: Vec::new(),
        }
    }

    fn positional(&mut self, value: OsString) {
        if self.pattern.is_some() {
            self.roots.push(value.into());
        } else {
            self.pattern = Some(value);
        }
    }

    fn switch(&mut self, option: &[u8]) -> bool {
        match option {
            b"-H" | b"--hidden" => self.ignore.dotfiles = false,
            b"--no-hidden" => self.ignore.dotfiles = true,
            b"-I" | b"--no-ignore" => self.ignore.files = false,
            b"--ignore" => self.ignore.files = true,
            b"--no-ignore-vcs" => self.ignore.vcs = false,
            b"--ignore-vcs" => self.ignore.vcs = true,
            b"--no-ignore-parent" => self.ignore.scope.parent = false,
            b"--ignore-parent" => self.ignore.scope.parent = true,
            b"--no-require-git" => self.ignore.scope.require_git = false,
            b"--require-git" => self.ignore.scope.require_git = true,
            b"-u" | b"--unrestricted" => {
                self.ignore.files = false;
                self.ignore.dotfiles = false;
            }
            b"-s" | b"--case-sensitive" => self.case = Case::Sensitive,
            b"-i" | b"--ignore-case" => self.case = Case::Insensitive,
            b"-g" | b"--glob" => self.syntax = Pattern::Glob,
            b"--regex" => self.syntax = Pattern::Regex,
            b"-F" | b"--fixed-strings" => self.syntax = Pattern::Fixed,
            b"--exact" => self.syntax = Pattern::Exact,
            b"-p" | b"--full-path" => self.path.full = true,
            b"-L" | b"--follow" => self.path.follow = true,
            b"-a" | b"--absolute-path" => self.path.absolute = true,
            b"-0" | b"--print0" => self.output.delimiter = 0,
            b"-q" | b"--quiet" => self.output.quiet = true,
            b"--show-errors" => self.output.show_errors = true,
            _ => return false,
        }
        true
    }

    fn measure(&mut self, flag: &'static str, value: &OsStr) -> Result<(), Error> {
        match flag {
            "--extension" => self.extensions.push(value.into()),
            "--exclude" => self
                .excludes
                .push(value.to_str().ok_or(Error::Value(flag))?.to_owned()),
            "--type" => self.types.push(match value.as_encoded_bytes() {
                b"f" | b"file" => FileType::File,
                b"d" | b"directory" => FileType::Directory,
                b"l" | b"symlink" => FileType::Symlink,
                _ => return Err(Error::Value(flag)),
            }),
            _ => self.depth(flag, value)?,
        }
        Ok(())
    }

    fn depth(&mut self, flag: &'static str, value: &OsStr) -> Result<(), Error> {
        let value = value
            .to_str()
            .and_then(|value| value.parse().ok())
            .ok_or(Error::Value(flag))?;
        match flag {
            "--min-depth" => self.min_depth = value,
            "--max-depth" => self.max_depth = Some(value),
            _ => {
                self.min_depth = value;
                self.max_depth = Some(value);
            }
        }
        Ok(())
    }
}

pub(super) fn parse(
    mut args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
) -> Result<Request, Error> {
    let mut options = Options::new(workers);
    let mut flags = true;
    while let Some(arg) = args.next() {
        let bytes = arg.as_encoded_bytes();
        if !flags || bytes == b"-" || !bytes.starts_with(b"-") {
            options.positional(arg);
            continue;
        }
        if let Some(count) = jobs::option(bytes, &mut args) {
            options.workers = count?;
            continue;
        }
        match bytes {
            b"--" => flags = false,
            b"-h" | b"--help" => return Ok(Request::Help),
            b"-V" | b"--version" => return Ok(Request::Version),
            _ if options.switch(bytes) => {}
            [b'-', b'-', ..] => long(&mut options, &arg, &mut args)?,
            [b'-', short @ ..] => short_options(&mut options, short, &arg, &mut args)?,
            _ => return Err(Error::Unknown(arg)),
        }
    }
    if options.max_depth.is_some_and(|max| max < options.min_depth) {
        return Err(Error::Depth);
    }
    if options.roots.is_empty() {
        options.roots.push(PathBuf::from("."));
        options.output.strip_implicit_dot = options.output.delimiter == b'\n';
    }
    Ok(Request::Search(Box::new(options)))
}

fn long(
    options: &mut Options,
    argument: &OsStr,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<(), Error> {
    let text = argument
        .to_str()
        .ok_or_else(|| Error::Unknown(argument.into()))?;
    let (flag, attached) = text.split_once('=').map_or((text, None), |(flag, value)| {
        (flag, Some(OsString::from(value)))
    });
    let flag = match flag {
        "--extension" => "--extension",
        "--exclude" => "--exclude",
        "--type" => "--type",
        "--min-depth" => "--min-depth",
        "--max-depth" => "--max-depth",
        "--exact-depth" => "--exact-depth",
        _ => return Err(Error::Unknown(argument.into())),
    };
    options.measure(
        flag,
        &attached
            .or_else(|| args.next())
            .ok_or(Error::Missing(flag))?,
    )
}

fn short_options(
    options: &mut Options,
    bytes: &[u8],
    argument: &OsStr,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<(), Error> {
    for (index, &byte) in bytes.iter().enumerate() {
        let flag = match byte {
            b'd' => "--max-depth",
            b'e' => "--extension",
            b'E' => "--exclude",
            b't' => "--type",
            b'j' => {
                options.workers = jobs::short_value(&bytes[index + 1..], args)?;
                return Ok(());
            }
            _ if options.switch(&[b'-', byte]) => continue,
            _ => return Err(Error::Unknown(argument.into())),
        };
        let value = match &bytes[index + 1..] {
            [] => args.next().ok_or(Error::Missing(flag))?,
            attached => std::str::from_utf8(attached)
                .map(OsString::from)
                .map_err(|_| Error::Value(flag))?,
        };
        return options.measure(flag, &value);
    }
    Ok(())
}

enum Matcher {
    Regex(RegexMatcher),
    Glob(GlobMatcher),
}

struct RegexMatcher {
    unicode: Regex,
    bytes: OnceLock<Option<Regex>>,
    insensitive: bool,
}

impl RegexMatcher {
    fn is_match(&self, name: &[u8]) -> bool {
        if std::str::from_utf8(name).is_ok() {
            return self.unicode.is_match(name);
        }
        self.bytes
            .get_or_init(|| {
                RegexBuilder::new(self.unicode.as_str())
                    .case_insensitive(self.insensitive)
                    .unicode(false)
                    .build()
                    .ok()
            })
            .as_ref()
            .unwrap_or(&self.unicode)
            .is_match(name)
    }
}

pub(super) struct Search {
    pub(super) options: Options,
    matcher: Matcher,
    cwd: Option<PathBuf>,
}

impl Search {
    pub(super) fn new(mut options: Options) -> Result<Self, Error> {
        let pattern = options
            .pattern
            .as_deref()
            .unwrap_or_default()
            .to_str()
            .ok_or(Error::Value("pattern"))?;
        let insensitive = match options.case {
            Case::Smart => !pattern.chars().any(char::is_uppercase),
            Case::Insensitive => true,
            Case::Sensitive => false,
        };
        let matcher = if let Pattern::Glob = options.syntax {
            Matcher::Glob(
                GlobBuilder::new(if pattern.is_empty() { "*" } else { pattern })
                    .case_insensitive(insensitive)
                    .literal_separator(true)
                    .build()?
                    .compile_matcher(),
            )
        } else {
            let pattern = match options.syntax {
                Pattern::Fixed => regex::escape(pattern),
                Pattern::Exact => format!("\\A{}\\z", regex::escape(pattern)),
                _ => pattern.to_owned(),
            };
            Matcher::Regex(RegexMatcher {
                unicode: RegexBuilder::new(&pattern)
                    .case_insensitive(insensitive)
                    .build()?,
                bytes: OnceLock::new(),
                insensitive,
            })
        };
        if options.path.absolute {
            for root in &mut options.roots {
                *root = std::path::absolute(&*root)?;
            }
        }
        let cwd = options.path.full.then(std::env::current_dir).transpose()?;
        Ok(Self {
            options,
            matcher,
            cwd,
        })
    }

    pub(super) fn configure(
        &self,
        builder: &mut WalkBuilder,
        root: &Path,
    ) -> Result<(), ignore::Error> {
        let options = &self.options;
        builder
            .hidden(options.ignore.dotfiles)
            .ignore(options.ignore.files)
            .git_ignore(options.ignore.files && options.ignore.vcs)
            .git_global(options.ignore.files && options.ignore.vcs)
            .git_exclude(options.ignore.files && options.ignore.vcs)
            .require_git(options.ignore.scope.require_git)
            .parents(options.ignore.scope.parent)
            .follow_links(options.path.follow)
            .max_depth(options.max_depth)
            .threads(options.workers.get());
        if options.ignore.files {
            builder.add_custom_ignore_filename(".fdignore");
        }
        if !options.excludes.is_empty() {
            let mut exclusions = OverrideBuilder::new(root);
            for pattern in &options.excludes {
                exclusions.add(&format!("!{pattern}"))?;
            }
            builder.overrides(exclusions.build()?);
        }
        Ok(())
    }

    pub(super) fn inspect(
        &self,
        result: Result<DirEntry, ignore::Error>,
        errors: &mut Vec<ignore::Error>,
    ) -> Option<DirEntry> {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                if self.options.output.show_errors {
                    errors.push(error);
                }
                return None;
            }
        };
        if self.options.output.show_errors {
            errors.extend(entry.error().cloned());
        }
        if entry.depth() == 0 || entry.depth() < self.options.min_depth || !self.accepts(&entry) {
            return None;
        }
        let absolute;
        let name = if let Some(cwd) = &self.cwd {
            let path = entry.path();
            absolute = cwd.join(path.strip_prefix(".").unwrap_or(path));
            absolute.as_path()
        } else {
            Path::new(entry.file_name())
        };
        let matched = match &self.matcher {
            Matcher::Regex(regex) => regex.is_match(name.as_os_str().as_encoded_bytes()),
            Matcher::Glob(glob) => glob.is_match(name),
        };
        matched.then_some(entry)
    }

    fn accepts(&self, entry: &DirEntry) -> bool {
        self.accepts_type(entry)
            && (self.options.extensions.is_empty()
                || self.options.extensions.iter().any(|extension| {
                    let bytes = extension.as_encoded_bytes();
                    let bytes = bytes.strip_prefix(b".").unwrap_or(bytes);
                    let name = entry.file_name().as_encoded_bytes();
                    name.len() > bytes.len() + 1
                        && name[name.len() - bytes.len() - 1] == b'.'
                        && name[name.len() - bytes.len()..].eq_ignore_ascii_case(bytes)
                }))
    }

    fn accepts_type(&self, entry: &DirEntry) -> bool {
        self.options.types.is_empty()
            || entry.file_type().is_some_and(|kind| {
                self.options.types.iter().any(|required| match required {
                    FileType::File => kind.is_file(),
                    FileType::Directory => kind.is_dir(),
                    FileType::Symlink => kind.is_symlink(),
                })
            })
    }
}
