use std::env;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use globset::{Glob, GlobSet, GlobSetBuilder};

use super::{Column, Command, Error, Kind};
use crate::jobs;

pub(super) enum Request {
    Help,
    Version,
    List(Box<Options>),
}

#[derive(Clone, Copy)]
pub(super) enum Size {
    Blocks(u64, bool),
    Binary,
    Si,
}

#[derive(Default)]
pub(super) struct Filter {
    types: Vec<String>,
    excluded_types: Vec<String>,
    groups: Vec<Kind>,
    excluded_groups: Vec<Kind>,
    targets: Option<GlobSet>,
    excluded_targets: Option<GlobSet>,
}

impl Filter {
    pub(super) fn matches(&self, kind: Kind, filesystem: &OsStr, path: &std::path::Path) -> bool {
        (self.types.is_empty() || self.types.iter().any(|item| filesystem == OsStr::new(item)))
            && !self
                .excluded_types
                .iter()
                .any(|item| filesystem == OsStr::new(item))
            && (self.groups.is_empty() || self.groups.contains(&kind))
            && !self.excluded_groups.contains(&kind)
            && self
                .targets
                .as_ref()
                .is_none_or(|matcher| matcher.is_match(path))
            && self
                .excluded_targets
                .as_ref()
                .is_none_or(|matcher| !matcher.is_match(path))
    }
}

#[derive(Default)]
pub(super) struct Layout {
    pub(super) inodes: bool,
    pub(super) show_type: bool,
    pub(super) portable: bool,
    pub(super) duf: DufLayout,
    pub(super) columns: Option<Vec<Column>>,
    pub(super) sort: Option<Column>,
    pub(super) width: Option<usize>,
}

#[derive(Default)]
pub(super) struct DufLayout {
    pub(super) json: bool,
    pub(super) ascii: bool,
    pub(super) warnings: bool,
}

pub(super) struct Options {
    pub(super) workers: jobs::Count,
    pub(super) paths: Vec<PathBuf>,
    pub(super) include_all: bool,
    pub(super) total: bool,
    pub(super) sync: bool,
    pub(super) layout: Layout,
    pub(super) filter: Filter,
    pub(super) size: Size,
}

impl Options {
    fn new(workers: jobs::Count) -> Self {
        Self {
            workers,
            paths: Vec::new(),
            include_all: false,
            total: false,
            sync: false,
            layout: Layout::default(),
            filter: Filter::default(),
            size: default_size(),
        }
    }

    fn switch(&mut self, command: Command, flag: &str) -> bool {
        match flag {
            "all" => self.include_all = true,
            "inodes" => self.layout.inodes = true,
            "human-readable" if command == Command::Df => self.size = Size::Binary,
            "si" if command == Command::Df => self.size = Size::Si,
            "local" if command == Command::Df => self.filter.excluded_groups.push(Kind::Network),
            "portability" if command == Command::Df => self.layout.portable = true,
            "print-type" if command == Command::Df => self.layout.show_type = true,
            "sync" if command == Command::Df => self.sync = true,
            "no-sync" if command == Command::Df => self.sync = false,
            "total" if command == Command::Df => self.total = true,
            "json" if command == Command::Duf => self.layout.duf.json = true,
            "warnings" if command == Command::Duf => self.layout.duf.warnings = true,
            _ => return false,
        }
        true
    }

    fn set(&mut self, command: Command, flag: &str, value: &str) -> Result<(), Error> {
        match flag {
            "output" => self.layout.columns = Some(columns(command, value)?),
            "block-size" => {
                self.size = parse_size(value).ok_or_else(|| Error::Value(flag.into()))?;
            }
            "type" => self.filter.types.push(value.into()),
            "exclude-type" => self.filter.excluded_types.push(value.into()),
            _ => self.set_duf(flag, value)?,
        }
        Ok(())
    }

    fn set_duf(&mut self, flag: &str, value: &str) -> Result<(), Error> {
        let invalid = || Error::Value(flag.into());
        match flag {
            "only" => self.filter.groups = groups(value)?,
            "hide" => self.filter.excluded_groups = groups(value)?,
            "only-fs" => self.filter.types = values(value),
            "hide-fs" => self.filter.excluded_types = values(value),
            "only-mp" => self.filter.targets = Some(globs(value)?),
            "hide-mp" => self.filter.excluded_targets = Some(globs(value)?),
            "sort" => {
                self.layout.sort = Some(Column::parse(Command::Duf, value).ok_or_else(invalid)?);
            }
            "width" => {
                self.layout.width = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|&width| width != 0)
                        .ok_or_else(invalid)?,
                );
            }
            "style" => {
                self.layout.duf.ascii = match value {
                    "ascii" => true,
                    "unicode" => false,
                    _ => return Err(invalid()),
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }

    fn short(
        &mut self,
        bytes: &[u8],
        remaining: &mut impl Iterator<Item = OsString>,
        original: &OsStr,
    ) -> Result<(), Error> {
        for (index, &flag) in bytes.iter().enumerate() {
            let option = match flag {
                b'a' => "all",
                b'h' => "human-readable",
                b'H' => "si",
                b'i' => "inodes",
                b'l' => "local",
                b'P' => "portability",
                b'T' => "print-type",
                b'v' => continue,
                b'k' => {
                    self.size = Size::Blocks(1024, true);
                    continue;
                }
                b'j' => {
                    self.workers = jobs::short_value(&bytes[index + 1..], remaining)?;
                    break;
                }
                b'B' | b't' | b'x' => {
                    let name = match flag {
                        b'B' => "block-size",
                        b't' => "type",
                        _ => "exclude-type",
                    };
                    return self.set(
                        Command::Df,
                        name,
                        &value(name, &bytes[index + 1..], remaining)?,
                    );
                }
                _ => return Err(Error::Option(original.into())),
            };
            self.switch(Command::Df, option);
        }
        Ok(())
    }
}

pub(super) fn parse(
    command: Command,
    mut args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
) -> Result<Request, Error> {
    let mut options = Options::new(workers);
    let mut flags = true;
    while let Some(arg) = args.next() {
        let bytes = arg.as_encoded_bytes();
        if !flags || !bytes.starts_with(b"-") || bytes == b"-" {
            options.paths.push(arg.into());
            continue;
        }
        // The upstream duf spelling -json shares the -j worker prefix.
        if command == Command::Duf && bytes == b"-json" {
            options.layout.duf.json = true;
            continue;
        }
        if let Some(count) = jobs::option(bytes, &mut args) {
            options.workers = count?;
            continue;
        }
        match bytes {
            b"--" => flags = false,
            b"--help" | b"-help" => return Ok(Request::Help),
            b"--version" | b"-version" => return Ok(Request::Version),
            [b'-', short @ ..] if command == Command::Df && !short.starts_with(b"-") => {
                options.short(short, &mut args, &arg)?;
            }
            _ => long(command, &arg, &mut options, &mut args)?,
        }
    }
    if command == Command::Df
        && options.layout.columns.is_some()
        && (options.layout.inodes || options.layout.portable || options.layout.show_type)
    {
        return Err(Error::ConflictingColumns);
    }
    Ok(Request::List(Box::new(options)))
}

fn long(
    command: Command,
    arg: &OsStr,
    options: &mut Options,
    remaining: &mut impl Iterator<Item = OsString>,
) -> Result<(), Error> {
    let text = arg.to_str().ok_or_else(|| Error::Option(arg.into()))?;
    let name = text
        .strip_prefix("--")
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    let (flag, attached) = name
        .split_once('=')
        .map_or((name, None), |(flag, value)| (flag, Some(value)));
    if attached.is_none() && options.switch(command, flag) {
        return Ok(());
    }
    if command == Command::Df && flag == "output" && attached.is_none() {
        options.layout.columns = Some(Column::DF_ALL.to_vec());
        return Ok(());
    }
    let accepts_value = match command {
        Command::Df => matches!(flag, "output" | "block-size" | "type" | "exclude-type"),
        Command::Duf => matches!(
            flag,
            "output"
                | "only"
                | "hide"
                | "only-fs"
                | "hide-fs"
                | "only-mp"
                | "hide-mp"
                | "sort"
                | "width"
                | "style"
        ),
    };
    if !accepts_value {
        return Err(Error::Option(arg.into()));
    }
    match attached {
        Some(input) => options.set(command, flag, input),
        None => options.set(command, flag, &value(flag, b"", remaining)?),
    }
}

fn value(
    flag: &str,
    attached: &[u8],
    remaining: &mut impl Iterator<Item = OsString>,
) -> Result<String, Error> {
    let invalid = || Error::Value(flag.into());
    if attached.is_empty() {
        return remaining
            .next()
            .ok_or_else(|| Error::Missing(flag.into()))?
            .into_string()
            .map_err(|_| invalid());
    }
    String::from_utf8(attached.to_vec()).map_err(|_| invalid())
}

fn columns(command: Command, list: &str) -> Result<Vec<Column>, Error> {
    let mut columns = Vec::new();
    for name in list.split(',') {
        let column = Column::parse(command, name).ok_or_else(|| Error::Column(name.into()))?;
        if columns.contains(&column) {
            return Err(Error::Column(name.into()));
        }
        columns.push(column);
    }
    Ok(columns)
}

fn groups(list: &str) -> Result<Vec<Kind>, Error> {
    list.split(',')
        .map(|name| Kind::parse(name).ok_or_else(|| Error::Group(name.into())))
        .collect()
}

fn values(list: &str) -> Vec<String> {
    list.split(',').map(str::to_owned).collect()
}

fn globs(list: &str) -> Result<GlobSet, Error> {
    let mut builder = GlobSetBuilder::new();
    for value in list.split(',') {
        builder.add(Glob::new(value)?);
    }
    Ok(builder.build()?)
}

fn default_size() -> Size {
    ["DF_BLOCK_SIZE", "BLOCK_SIZE", "BLOCKSIZE"]
        .into_iter()
        .find_map(|name| env::var(name).ok().and_then(|value| parse_size(&value)))
        .unwrap_or_else(|| {
            Size::Blocks(
                if env::var_os("POSIXLY_CORRECT").is_some() {
                    512
                } else {
                    1024
                },
                true,
            )
        })
}

fn parse_size(value: &str) -> Option<Size> {
    match value {
        "human-readable" => return Some(Size::Binary),
        "si" => return Some(Size::Si),
        _ => {}
    }
    let split = value.bytes().take_while(u8::is_ascii_digit).count();
    let (digits, suffix) = value.split_at(split);
    let amount = if digits.is_empty() {
        1u64
    } else {
        digits.parse().ok()?
    };
    let multiplier = match suffix.as_bytes() {
        b"" if split == 0 => return None,
        b"" | b"B" => 1,
        [prefix, ending @ ..] => {
            let power = b"KMGTPEZYRQ".iter().position(|unit| unit == prefix)? + 1;
            let radix = match ending {
                b"" | b"iB" => 1024u64,
                b"B" => 1000u64,
                _ => return None,
            };
            radix.checked_pow(u32::try_from(power).ok()?)?
        }
    };
    let bytes = amount.checked_mul(multiplier)?;
    (bytes != 0).then_some(Size::Blocks(bytes, suffix.is_empty() || suffix == "K"))
}
