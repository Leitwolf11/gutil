use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::{Table, Value};

macro_rules! choice {
    ($name:ident, $default:ident $(, $other:ident)*) => {
        #[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
        #[serde(rename_all = "kebab-case")]
        pub(crate) enum $name {
            #[default]
            $default,
            $($other),*
        }
    };
}

choice!(When, Auto, Never, Always);
choice!(DiagnosticFormat, Text, Json);
choice!(Verbosity, Normal, Quiet, Verbose);
choice!(Layout, OnePerLine, Columns, Long);
choice!(Hidden, Omit, AlmostAll, All);
choice!(Sort, Name, Size, Modified, Extension, None);
choice!(Quote, Auto, Literal, Shell);
choice!(SizeStyle, Bytes, Human, Si);
choice!(TimeStyle, Iso, Rfc3339, Unix);
choice!(Timezone, Utc, Local);
choice!(OwnerFormat, Numeric, None);
choice!(Number, None, All, Nonblank);
choice!(PwdMode, Physical, Logical);

impl When {
    pub(crate) fn enabled(self, terminal: bool) -> bool {
        self == Self::Always || (self == Self::Auto && terminal)
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Auto {
    Auto,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(untagged)]
enum JobCount {
    Number(usize),
    Automatic(Auto),
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Global {
    jobs: JobCount,
    max_auto_jobs: usize,
    pub(crate) progress: When,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            jobs: JobCount::Automatic(Auto::Auto),
            max_auto_jobs: 8,
            progress: When::Auto,
        }
    }
}

impl Global {
    pub(crate) fn job_count(self) -> usize {
        match self.jobs {
            JobCount::Number(count) => count,
            JobCount::Automatic(_) => crate::jobs::default_count(self.max_auto_jobs),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Output {
    pub(crate) color: When,
    pub(crate) hyperlinks: When,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            color: When::Auto,
            hyperlinks: When::Never,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Diagnostics {
    pub(crate) format: DiagnosticFormat,
    pub(crate) verbosity: Verbosity,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Ls {
    pub(crate) layout: Layout,
    pub(crate) hidden: Hidden,
    pub(crate) sort: Sort,
    pub(crate) reverse: bool,
    pub(crate) directories_first: bool,
    pub(crate) quote: Quote,
    pub(crate) long: Long,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Long {
    pub(crate) size_style: SizeStyle,
    pub(crate) time_style: TimeStyle,
    pub(crate) timezone: Timezone,
    pub(crate) owner_format: OwnerFormat,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
#[allow(clippy::struct_excessive_bools)] // These output switches are independent.
pub(crate) struct Cat {
    pub(crate) number: Number,
    pub(crate) squeeze_blank: bool,
    pub(crate) show_ends: bool,
    pub(crate) show_tabs: bool,
    pub(crate) show_nonprinting: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Pwd {
    pub(crate) mode: PwdMode,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Settings {
    pub(crate) global: Global,
    pub(crate) output: Output,
    pub(crate) diagnostics: Diagnostics,
    pub(crate) ls: Ls,
    pub(crate) cat: Cat,
    pub(crate) pwd: Pwd,
}

#[derive(Default)]
pub(crate) enum Source {
    #[default]
    Discover,
    Explicit(PathBuf),
    Disabled,
}

pub(crate) struct Loaded {
    pub(crate) settings: Settings,
    pub(crate) path: Option<PathBuf>,
}

pub(crate) fn load(source: Source, profile: Option<&str>) -> Result<Loaded, String> {
    let file = match source {
        Source::Discover => discover()?,
        Source::Explicit(path) => {
            Some((read(&path).map_err(|error| describe(&path, error))?, path))
        }
        Source::Disabled => None,
    };

    match file {
        Some((contents, path)) => {
            let settings = parse(&contents, profile).map_err(|error| describe(&path, error))?;
            Ok(Loaded {
                settings,
                path: Some(path),
            })
        }
        None if profile.is_some() => Err("a profile requires a configuration file".to_owned()),
        None => Ok(Loaded {
            settings: Settings::default(),
            path: None,
        }),
    }
}

fn discover() -> Result<Option<(String, PathBuf)>, String> {
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = env::var_os(home_key)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(path).join(".gutil").join("config.toml"));
    for path in [Some(PathBuf::from("gutil.toml")), home]
        .into_iter()
        .flatten()
    {
        match read(&path) {
            Ok(contents) => return Ok(Some((contents, path))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(describe(&path, error)),
        }
    }
    Ok(None)
}

fn read(path: &Path) -> io::Result<String> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration must be a regular file",
        ));
    }
    fs::read_to_string(path)
}

fn describe(path: &Path, error: impl std::fmt::Display) -> String {
    format!("configuration {}: {error}", path.display())
}

fn parse(contents: &str, profile: Option<&str>) -> Result<Settings, String> {
    let mut base: Table = toml::from_str(contents).map_err(|error| error.to_string())?;
    let profiles = match base.remove("profiles") {
        Some(Value::Table(profiles)) => profiles,
        Some(_) => return Err("profiles must be a table".into()),
        None => Table::new(),
    };
    let mut selected = validate(base.clone())?;
    let mut found = profile.is_none();
    for (name, profile_table) in profiles {
        let Value::Table(profile_table) = profile_table else {
            return Err(format!("profiles.{name} must be a table"));
        };
        let mut merged = base.clone();
        merge(&mut merged, profile_table);
        let settings = validate(merged).map_err(|error| format!("profiles.{name}: {error}"))?;
        if profile == Some(&name) {
            selected = settings;
            found = true;
        }
    }
    if !found {
        return Err(format!("unknown profile {:?}", profile.unwrap_or_default()));
    }
    Ok(selected)
}

fn merge(base: &mut Table, profile: Table) {
    for (key, value) in profile {
        if let (Some(Value::Table(base)), Value::Table(profile)) = (base.get_mut(&key), &value) {
            merge(base, profile.clone());
        } else {
            base.insert(key, value);
        }
    }
}

fn validate(table: Table) -> Result<Settings, String> {
    let settings: Settings = Value::Table(table)
        .try_into()
        .map_err(|error: toml::de::Error| error.to_string())?;
    if !(1..=crate::jobs::MAX_WORKERS).contains(&settings.global.max_auto_jobs) {
        return Err("global.max-auto-jobs must be between 1 and 512".into());
    }
    if let JobCount::Number(count) = settings.global.jobs {
        if !(1..=crate::jobs::MAX_WORKERS).contains(&count) {
            return Err("global.jobs must be auto or an integer between 1 and 512".into());
        }
    }
    Ok(settings)
}

pub(crate) fn option_value<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, serde::de::value::Error> {
    T::deserialize(serde::de::value::StrDeserializer::new(value))
}
