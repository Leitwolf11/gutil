use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::{self, File};
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode, Url};
use sha2::{Digest, Sha256};

use crate::config::{self, RemoteCacheMode};
use crate::presentation::Presentation;

const COMMAND: &str = "gutil cache";
const HELP: &str = "Usage: gutil cache [--offline] put [--] FILE\n       gutil cache [--offline] get SHA256\n\nStore a regular file by content and print its SHA-256 digest, or retrieve its bytes.\nUse --offline to skip remote cache access.\n";

pub(crate) fn run(
    mut args: impl Iterator<Item = OsString>,
    offline: bool,
    loaded: &config::Loaded,
    ui: Presentation,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<u8> {
    let action = args.next();
    let action = action.as_deref().and_then(|value| value.to_str());
    match action {
        Some("-h" | "--help") => return output.write_all(HELP.as_bytes()).map(|()| 0),
        Some("--version") => {
            return writeln!(output, "gutil {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        _ => {}
    }
    let mut operand = args.next();
    if action == Some("put") && operand.as_deref().is_some_and(|value| value == "--") {
        operand = args.next();
    }
    let operation = match (action, operand, args.next()) {
        (Some("put"), Some(path), None) => Ok(Operation::Put(PathBuf::from(path))),
        (Some("get"), Some(key), None) => key
            .into_string()
            .ok()
            .filter(|key| key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .map(|key| Operation::Get(key.to_ascii_lowercase()))
            .ok_or("expected a 64-digit hexadecimal SHA-256 digest"),
        _ => Err("expected 'put FILE' or 'get SHA256' (see 'gutil cache --help')"),
    };
    let operation = match operation {
        Ok(operation) => operation,
        Err(error) => return ui.error(errors, COMMAND, error).map(|()| 2),
    };

    let mut cache = Cache {
        loaded,
        offline,
        ui,
        errors,
    };
    let local = cache.local_directory()?;
    match operation {
        Operation::Put(path) => {
            let bytes = read_file(&path, cache.limit()).map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })?;
            let key = digest(&bytes);
            cache.put(local.as_deref(), &key, bytes)?;
            writeln!(output, "{key}")?;
        }
        Operation::Get(key) => output.write_all(&cache.get(local.as_deref(), &key)?)?,
    }
    Ok(0)
}

enum Operation {
    Put(PathBuf),
    Get(String),
}

struct Cache<'a, W> {
    loaded: &'a config::Loaded,
    offline: bool,
    ui: Presentation,
    errors: &'a mut W,
}

impl<W: Write> Cache<'_, W> {
    fn limit(&self) -> usize {
        self.loaded.settings.cache.max_entry_bytes
    }

    fn local_directory(&mut self) -> io::Result<Option<PathBuf>> {
        let local = &self.loaded.settings.cache.local;
        if !local.enabled {
            return Ok(None);
        }
        if let Some(directory) = &local.directory {
            let base = self
                .loaded
                .path
                .as_deref()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(""));
            return Ok(Some(base.join(directory).join("sha256")));
        }
        if let Some(home) = config::home_dir() {
            return Ok(Some(home.join(".gutil").join("cache").join("sha256")));
        }
        self.warn("local cache has no home directory or configured path")?;
        Ok(None)
    }

    fn put(&mut self, local: Option<&Path>, key: &str, bytes: Vec<u8>) -> io::Result<()> {
        let local_stored = self.store_local(local, key, &bytes)?;
        let remote_stored = self.loaded.settings.cache.remote.mode == RemoteCacheMode::ReadWrite
            && self
                .with_remote(
                    move |remote, url| async move { remote.put(url, bytes).await },
                    key,
                )?
                .is_some();
        if local_stored || remote_stored {
            Ok(())
        } else {
            Err(io::Error::other("no writable cache stored the entry"))
        }
    }

    fn get(&mut self, local: Option<&Path>, key: &str) -> io::Result<Vec<u8>> {
        if let Some(directory) = local {
            match read_file(&directory.join(key), self.limit()).and_then(|bytes| verify(key, bytes))
            {
                Ok(bytes) => return Ok(bytes),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => self.warn(&format!("local cache read failed: {error}"))?,
            }
        }
        let limit = self.limit();
        if let Some(Some(bytes)) = self.with_remote(
            |remote, url| async move { remote.get(url, key, limit).await },
            key,
        )? {
            self.store_local(local, key, &bytes)?;
            return Ok(bytes);
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("cache entry {key} is unavailable"),
        ))
    }

    fn store_local(&mut self, local: Option<&Path>, key: &str, bytes: &[u8]) -> io::Result<bool> {
        let Some(directory) = local else {
            return Ok(false);
        };
        match store_file(directory, key, bytes) {
            Ok(()) => Ok(true),
            Err(error) => {
                self.warn(&format!("local cache write failed: {error}"))?;
                Ok(false)
            }
        }
    }

    fn with_remote<T, F: Future<Output = io::Result<T>>>(
        &mut self,
        operation: impl FnOnce(Remote, Url) -> F,
        key: &str,
    ) -> io::Result<Option<T>> {
        let settings = &self.loaded.settings.cache.remote;
        if self.offline || settings.mode == RemoteCacheMode::Off {
            return Ok(None);
        }
        if self.loaded.project_discovered {
            self.warn("remote cache skipped for an automatically discovered project file; select it with --config to allow access")?;
            return Ok(None);
        }
        let result = (|| {
            let endpoint = settings
                .endpoint()
                .map_err(io::Error::other)?
                .ok_or_else(|| io::Error::other("remote cache endpoint is missing"))?;
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async {
                let (remote, url) = Remote::new(endpoint, key, settings.timeout_ms)?;
                operation(remote, url).await
            })
        })();
        match result {
            Ok(value) => Ok(Some(value)),
            Err(error) => {
                self.warn(&format!("remote cache request failed: {error}"))?;
                Ok(None)
            }
        }
    }

    fn warn(&mut self, message: &str) -> io::Result<()> {
        self.ui.warning(self.errors, COMMAND, message)
    }
}

fn read_file(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    check_size(size, limit)?;
    let mut bytes = Vec::with_capacity(size);
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    check_size(bytes.len(), limit)?;
    Ok(bytes)
}

fn store_file(directory: &Path, key: &str, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(bytes)?;
    let destination = directory.join(key);
    if let Err(unpublished) = temporary.persist_noclobber(&destination) {
        // Keep an identical entry published by another writer. Replace corrupt entries.
        if !read_file(&destination, bytes.len()).is_ok_and(|existing| existing == bytes) {
            unpublished.file.persist(&destination)?;
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut key = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        key.push(char::from(HEX[usize::from(byte >> 4)]));
        key.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    key
}

fn verify(key: &str, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if digest(&bytes) == key {
        Ok(bytes)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry does not match its SHA-256 digest",
        ))
    }
}

fn check_size(size: usize, limit: usize) -> io::Result<()> {
    if size <= limit {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("entry exceeds the {limit}-byte cache limit"),
        ))
    }
}

struct Remote {
    client: Client,
}

impl Remote {
    fn new(mut url: Url, key: &str, timeout_ms: u64) -> io::Result<(Self, Url)> {
        let mut builder = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .redirect(reqwest::redirect::Policy::none());
        if url.scheme() == "https" {
            match env::var("GUTIL_CACHE_TOKEN") {
                Ok(value) if !value.is_empty() => {
                    let mut authorization = HeaderValue::from_str(&format!("Bearer {value}"))
                        .map_err(|_| {
                            io::Error::other("GUTIL_CACHE_TOKEN is not a valid HTTP credential")
                        })?;
                    authorization.set_sensitive(true);
                    let mut headers = HeaderMap::new();
                    headers.insert(AUTHORIZATION, authorization);
                    builder = builder.default_headers(headers);
                }
                Ok(_) | Err(env::VarError::NotPresent) => {}
                Err(env::VarError::NotUnicode(_)) => {
                    return Err(io::Error::other("GUTIL_CACHE_TOKEN must be valid Unicode"));
                }
            }
        } else {
            builder = builder.no_proxy();
        }
        url.path_segments_mut()
            .map_err(|()| io::Error::other("invalid remote cache endpoint path"))?
            .pop_if_empty()
            .extend(["sha256", key]);
        let client = builder.build().map_err(request_error)?;
        Ok((Self { client }, url))
    }

    async fn get(self, url: Url, key: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
        let mut response = self.client.get(url).send().await.map_err(request_error)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::OK => {}
            status => return Err(io::Error::other(format!("GET returned HTTP {status}"))),
        }
        let length = usize::try_from(response.content_length().unwrap_or(0)).unwrap_or(usize::MAX);
        check_size(length, limit)?;
        let mut bytes = Vec::with_capacity(length);
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            check_size(bytes.len().saturating_add(chunk.len()), limit)?;
            bytes.extend_from_slice(&chunk);
        }
        verify(key, bytes).map(Some)
    }

    async fn put(self, url: Url, bytes: Vec<u8>) -> io::Result<()> {
        let status = self
            .client
            .put(url)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .map_err(request_error)?
            .status();
        if status.is_success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("PUT returned HTTP {status}")))
        }
    }
}

fn request_error(error: reqwest::Error) -> io::Error {
    if error.is_timeout() {
        return io::Error::new(io::ErrorKind::TimedOut, "HTTP request timed out");
    }
    let error = error.without_url();
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    io::Error::other(message)
}
