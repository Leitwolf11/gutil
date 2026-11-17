use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Existence {
    Existing,
    FinalMayBeMissing,
    AnyMayBeMissing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Order {
    Physical,
    Logical,
}

#[derive(Clone, Copy)]
pub(super) struct Mode {
    pub(super) existence: Existence,
    pub(super) order: Order,
    pub(super) strip: bool,
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            existence: Existence::FinalMayBeMissing,
            order: Order::Physical,
            strip: false,
        }
    }
}

pub(super) fn resolve(path: &Path, cwd: &Path, mode: Mode) -> io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "empty path"));
    }

    // The platform canonicalizer does the physical walk without allocating a
    // queue. A failed lookup still needs our configurable missing-path rules.
    if mode.order == Order::Physical && !mode.strip {
        if let Ok(canonical) = fs::canonicalize(path) {
            return Ok(native_path(canonical));
        }
    }

    let (root, parts) = split(&cwd.join(path));
    if mode.strip {
        return walk(root, parts, mode.existence, Expansion::Strip);
    }
    match mode.order {
        Order::Physical => walk(root, parts, mode.existence, Expansion::Physical),
        Order::Logical => {
            let normalized = walk(root, parts, mode.existence, Expansion::Logical)?;
            let (root, parts) = split(&normalized);
            walk(root, parts, mode.existence, Expansion::Physical)
        }
    }
}

#[cfg(not(windows))]
fn native_path(path: PathBuf) -> PathBuf {
    path
}

#[cfg(windows)]
fn native_path(path: PathBuf) -> PathBuf {
    use std::path::Prefix;

    let mut components = path.components();
    match components.next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(drive) => {
                let mut normal = PathBuf::from(format!("{}:", char::from(drive)));
                normal.push(components.as_path());
                normal
            }
            Prefix::VerbatimUNC(host, share) => {
                let mut normal = OsString::from(r"\\");
                normal.push(host);
                normal.push("\\");
                normal.push(share);
                let mut normal = PathBuf::from(normal);
                normal.push(components.as_path());
                normal
            }
            _ => path,
        },
        _ => path,
    }
}

enum Part {
    Parent,
    Name(OsString),
    // A trailing slash or explicit dot makes the preceding component a
    // directory. A plain slash allows a missing leaf in the default mode.
    Directory { allow_missing_leaf: bool },
}

fn split(path: &Path) -> (PathBuf, VecDeque<Part>) {
    let mut root = PathBuf::new();
    let mut parts = VecDeque::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => root.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => parts.push_back(Part::Parent),
            Component::Normal(name) => parts.push_back(Part::Name(name.to_os_string())),
        }
    }
    let bytes = path.as_os_str().as_encoded_bytes();
    let is_separator = |byte: u8| byte == b'/' || (cfg!(windows) && byte == b'\\');
    let end = bytes
        .iter()
        .rposition(|byte| !is_separator(*byte))
        .map_or(0, |i| i + 1);
    let trimmed = &bytes[..end];
    if matches!(trimmed, [.., separator, b'.'] if is_separator(*separator)) {
        parts.push_back(Part::Directory {
            allow_missing_leaf: false,
        });
        return (root, parts);
    }
    if bytes.len() != trimmed.len() {
        parts.push_back(Part::Directory {
            allow_missing_leaf: true,
        });
    }
    (root, parts)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expansion {
    Physical,
    Logical,
    Strip,
}

fn walk(
    mut result: PathBuf,
    mut parts: VecDeque<Part>,
    existence: Existence,
    expansion: Expansion,
) -> io::Result<PathBuf> {
    let mut followed = 0;
    let expand = expansion == Expansion::Physical;
    while let Some(part) = parts.pop_front() {
        match part {
            Part::Parent => {
                result.pop();
            }
            Part::Directory { .. } => {}
            Part::Name(name) => {
                result.push(name);
                if !expand && existence == Existence::AnyMayBeMissing {
                    continue;
                }
                let metadata = if expand {
                    fs::symlink_metadata(&result)
                } else {
                    fs::metadata(&result)
                };
                let metadata = match metadata {
                    Ok(metadata) => metadata,
                    Err(error) if allow_failure(&error, existence, &parts, expansion) => continue,
                    Err(error) => return Err(error),
                };
                if expand && metadata.file_type().is_symlink() {
                    followed += 1;
                    if followed > 40 {
                        if existence == Existence::AnyMayBeMissing {
                            continue;
                        }
                        return Err(io::Error::other("too many symbolic link expansions"));
                    }
                    let target = fs::read_link(&result)?;
                    let (root, mut target) = split(&target);
                    result.pop();
                    if !root.as_os_str().is_empty() {
                        result.push(native_path(root));
                    }
                    target.append(&mut parts);
                    parts = target;
                } else if !parts.is_empty()
                    && !metadata.is_dir()
                    && existence != Existence::AnyMayBeMissing
                {
                    return Err(io::Error::new(
                        io::ErrorKind::NotADirectory,
                        "path component is not a directory",
                    ));
                }
            }
        }
    }
    Ok(result)
}

fn allow_failure(
    error: &io::Error,
    existence: Existence,
    remaining: &VecDeque<Part>,
    expansion: Expansion,
) -> bool {
    match existence {
        Existence::Existing => false,
        Existence::AnyMayBeMissing => true,
        Existence::FinalMayBeMissing => {
            error.kind() == io::ErrorKind::NotFound
                && remaining.iter().all(|part| {
                    matches!(
                        part,
                        Part::Directory {
                            allow_missing_leaf: true
                        }
                    ) || (expansion == Expansion::Strip && matches!(part, Part::Name(_)))
                })
        }
    }
}

pub(super) fn relative(path: &Path, base: &Path) -> Option<PathBuf> {
    let mut path = path.components().peekable();
    let mut base = base.components().peekable();
    while path.peek().is_some_and(|left| Some(left) == base.peek()) {
        path.next();
        base.next();
    }
    if path
        .peek()
        .into_iter()
        .chain(base.peek())
        .any(|part| matches!(part, Component::RootDir | Component::Prefix(_)))
    {
        return None;
    }
    let mut result = PathBuf::new();
    for _ in base {
        result.push("..");
    }
    result.extend(path);
    if result.as_os_str().is_empty() {
        result.push(".");
    }
    Some(result)
}
