use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::Duration;

use same_file::Handle;

use super::{header, Edge, Options, ParseError};
use crate::presentation::Presentation;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Descriptor,
    Name,
}

pub(super) struct Settings {
    pub(super) mode: Option<Mode>,
    pub(super) retry: bool,
    delay: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: None,
            retry: false,
            delay: Duration::from_secs(1),
        }
    }
}

impl Settings {
    pub(super) fn validate(&self) -> Result<(), ParseError> {
        if self.retry && self.mode.is_none() {
            return Err(ParseError::Follow("--retry requires --follow"));
        }
        Ok(())
    }

    pub(super) fn by_name(&mut self) {
        self.mode = Some(Mode::Name);
        self.retry = true;
    }

    pub(super) fn long(
        &mut self,
        name: &[u8],
        value: Option<&[u8]>,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        match (name, value) {
            (b"--follow", None | Some(b"descriptor")) => self.mode = Some(Mode::Descriptor),
            (b"--follow", Some(b"name")) => self.mode = Some(Mode::Name),
            (b"--retry", None) => self.retry = true,
            (b"--sleep-interval", _) => return self.set_delay(value, args),
            _ => {
                return Err(ParseError::Option(
                    String::from_utf8_lossy(name).into_owned().into(),
                ))
            }
        }
        Ok(())
    }

    pub(super) fn set_delay(
        &mut self,
        value: Option<&[u8]>,
        args: &mut impl Iterator<Item = OsString>,
    ) -> Result<(), ParseError> {
        let owned;
        let value = if let Some(value) = value {
            value
        } else {
            owned = args.next().ok_or(ParseError::Missing("--sleep-interval"))?;
            owned.as_encoded_bytes()
        };
        self.delay = std::str::from_utf8(value)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .and_then(|value| Duration::try_from_secs_f64(value).ok())
            .ok_or(ParseError::Follow("invalid sleep interval"))?;
        Ok(())
    }
}

pub(super) struct Input {
    path: PathBuf,
    open: Option<Open>,
    warned: bool,
}

struct Open {
    file: File,
    identity: Handle,
}

impl Open {
    fn new(file: File) -> io::Result<Self> {
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "follow requires a regular file",
            ));
        }
        let identity = Handle::from_file(file.try_clone()?)?;
        Ok(Self { file, identity })
    }
}

impl Input {
    pub(super) fn opened(path: PathBuf, file: File) -> io::Result<Self> {
        Ok(Self {
            path,
            open: Some(Open::new(file)?),
            warned: false,
        })
    }

    pub(super) fn pending(path: PathBuf) -> Self {
        Self {
            path,
            open: None,
            warned: true,
        }
    }

    fn refresh(&mut self, mode: Mode) -> io::Result<&mut File> {
        if self.open.is_none() || mode == Mode::Name {
            let candidate = Open::new(File::open(&self.path)?)?;
            if self
                .open
                .as_ref()
                .is_none_or(|opened| opened.identity != candidate.identity)
            {
                self.open = Some(candidate);
            }
        }
        let file = &mut self
            .open
            .as_mut()
            .ok_or_else(|| io::Error::other("file is not open"))?
            .file;
        if file.metadata()?.len() < file.stream_position()? {
            file.seek(SeekFrom::Start(0))?;
        }
        Ok(file)
    }
}

pub(super) fn run(
    mut inputs: Vec<Input>,
    options: &Options,
    ui: Presentation,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> io::Result<bool> {
    let Some(mode) = options.follow.mode else {
        return Ok(false);
    };
    let mut failed = false;
    let mut buffer = [0u8; 16 * 1024];
    // The final initial header belongs to the last operand that opened successfully.
    let mut displayed = options
        .paths
        .iter()
        .rev()
        .find(|path| {
            path.as_path() == std::path::Path::new("-")
                || inputs
                    .iter()
                    .any(|input| input.open.is_some() && &input.path == *path)
        })
        .cloned();
    while !inputs.is_empty() {
        let mut full_block = false;
        let mut index = 0;
        while index < inputs.len() {
            let input = &mut inputs[index];
            match input.refresh(mode).and_then(|file| file.read(&mut buffer)) {
                Ok(read) => {
                    input.warned = false;
                    if read != 0 {
                        full_block |= read == buffer.len();
                        if displayed.as_ref() != Some(&input.path) {
                            header(options, &input.path, &mut displayed.is_some(), output)?;
                            displayed = Some(input.path.clone());
                        }
                        output.write_all(&buffer[..read])?;
                        output.flush()?;
                    }
                }
                Err(error) => {
                    if !input.warned {
                        ui.error(
                            errors,
                            Edge::Tail.label(),
                            &format!("{}: {error}", input.path.display()),
                        )?;
                        input.warned = true;
                    }
                    if !options.follow.retry {
                        failed = true;
                        inputs.remove(index);
                        continue;
                    }
                }
            }
            index += 1;
        }
        if !inputs.is_empty() && !full_block {
            std::thread::sleep(options.follow.delay);
        }
    }
    Ok(failed)
}
