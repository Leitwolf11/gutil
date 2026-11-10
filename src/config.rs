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
choice!(RemoteCacheMode, Off, ReadOnly, ReadWrite);

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

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct RemoteCache {
    pub(crate) mode: RemoteCacheMode,
    endpoint: Option<String>,
    pub(crate) timeout_ms: u64,
}

impl Default for RemoteCache {
    fn default() -> Self {
        Self {
            mode: RemoteCacheMode::Off,
            endpoint: None,
            timeout_ms: 3_000,
        }
    }
}

impl RemoteCache {
    pub(crate) fn endpoint(&self) -> Result<Option<reqwest::Url>, String> {
        let Some(endpoint) = &self.endpoint else {
            return if self.mode == RemoteCacheMode::Off {
                Ok(None)
            } else {
                Err("cache.remote.endpoint is required when remote caching is enabled".into())
            };
        };
        let url = reqwest::Url::parse(endpoint).map_err(|_| "invalid cache.remote.endpoint URL")?;
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
            || !url.has_host()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("cache.remote.endpoint must use HTTPS (HTTP is allowed on loopback) and omit credentials, query, and fragment".into());
        }
        Ok(Some(url))
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct LocalCache {
    pub(crate) enabled: bool,
    pub(crate) directory: Option<PathBuf>,
}

impl Default for LocalCache {
    fn default() -> Self {
        Self {
            enabled: true,
            directory: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Cache {
    pub(crate) max_entry_bytes: usize,
    pub(crate) local: LocalCache,
    pub(crate) remote: RemoteCache,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            max_entry_bytes: 16 * 1024 * 1024,
            local: LocalCache::default(),
            remote: RemoteCache::default(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct Settings {
    pub(crate) global: Global,
    pub(crate) output: Output,
    pub(crate) diagnostics: Diagnostics,
    pub(crate) ls: Ls,
    pub(crate) cat: Cat,
    pub(crate) pwd: Pwd,
    pub(crate) cache: Cache,
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
    pub(crate) project_discovered: bool,
}

pub(crate) fn load(source: Source, profile: Option<&str>) -> Result<Loaded, String> {
    let discovering = matches!(source, Source::Discover);
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
                project_discovered: discovering && path == Path::new("gutil.toml"),
                path: Some(path),
            })
        }
        None if profile.is_some() => Err("a profile requires a configuration file".to_owned()),
        None => Ok(Loaded {
            settings: Settings::default(),
            path: None,
            project_discovered: false,
        }),
    }
}

fn discover() -> Result<Option<(String, PathBuf)>, String> {
    let home = home_dir().map(|path| path.join(".gutil").join("config.toml"));
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

pub(crate) fn home_dir() -> Option<PathBuf> {
    let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(home_key)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
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
    if !(1..=256 * 1024 * 1024).contains(&settings.cache.max_entry_bytes) {
        return Err("cache.max-entry-bytes must be between 1 and 268435456".into());
    }
    if !(1..=60_000).contains(&settings.cache.remote.timeout_ms) {
        return Err("cache.remote.timeout-ms must be between 1 and 60000".into());
    }
    settings.cache.remote.endpoint()?;
    Ok(settings)
}

pub(crate) fn option_value<T: serde::de::DeserializeOwned>(
    value: &str,
) -> Result<T, serde::de::value::Error> {
    T::deserialize(serde::de::value::StrDeserializer::new(value))
}
