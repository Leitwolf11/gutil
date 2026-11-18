use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use smallvec::SmallVec;

use crate::jobs;
use crate::presentation::{Presentation, ProgressReport};

mod hash;
mod record;
mod verify;

use hash::digest;

const BUFFER_SIZE: usize = 64 * 1024;
const PARALLEL_MIN_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Algorithm {
    Md5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl Algorithm {
    pub(crate) fn parse(name: &OsStr) -> Option<Self> {
        Some(match name.to_str()? {
            "md5sum" => Self::Md5,
            "sha1sum" => Self::Sha1,
            "sha224sum" => Self::Sha224,
            "sha256sum" => Self::Sha256,
            "sha384sum" => Self::Sha384,
            "sha512sum" => Self::Sha512,
            _ => return None,
        })
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Md5 => "gutil md5sum",
            Self::Sha1 => "gutil sha1sum",
            Self::Sha224 => "gutil sha224sum",
            Self::Sha256 => "gutil sha256sum",
            Self::Sha384 => "gutil sha384sum",
            Self::Sha512 => "gutil sha512sum",
        }
    }

    const fn tag(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA1",
            Self::Sha224 => "SHA224",
            Self::Sha256 => "SHA256",
            Self::Sha384 => "SHA384",
            Self::Sha512 => "SHA512",
        }
    }
}

pub(crate) fn run(
    algorithm: Algorithm,
    args: impl Iterator<Item = OsString>,
    jobs: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let (options, jobs) = match Options::parse(args, jobs) {
        Ok(Request::Help) => return help(algorithm),
        Ok(Request::Version) => {
            return writeln!(
                io::stdout(),
                "{} {}",
                algorithm.label(),
                env!("CARGO_PKG_VERSION")
            )
            .map(|()| 0);
        }
        Ok(Request::Run(options, jobs)) => (options, jobs),
        Err(error) => {
            ui.error(&mut io::stderr(), algorithm.label(), &error.to_string())?;
            return Ok(1);
        }
    };

    let result = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(jobs.get())
        .build()?
        .block_on(execute(algorithm, options, jobs, ui));
    match result {
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            ui.error(&mut io::stderr(), algorithm.label(), &error.to_string())?;
            Ok(1)
        }
        result => result,
    }
}

fn help(algorithm: Algorithm) -> io::Result<u8> {
    writeln!(
        io::stdout(),
        "Usage: {} [OPTION]... [FILE]...\n\nPrint or check {} checksums. Read stdin when FILE is '-' or omitted.\n\n  -b, --binary      Mark generated records as binary\n  -c, --check       Verify files using a checksum list\n  -t, --text        Mark generated records as text\n  -z, --zero        Use NUL record terminators and raw file names\n      --tag         Write BSD-style records\n      --ignore-missing, --quiet, --status, --strict, -w, --warn\n  -j, --jobs, --threads N  Limit concurrent filesystem jobs\n      --help         Print help\n      --version      Print version",
        algorithm.label(),
        algorithm.tag(),
    )
    .map(|()| 0)
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum InputMode {
    #[default]
    Default,
    Binary,
    Text,
}

#[derive(Default)]
struct Options {
    files: SmallVec<[PathBuf; 4]>,
    check: bool,
    mode: InputMode,
    tagged: bool,
    zero: bool,
    verification: Verification,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Report {
    #[default]
    All,
    Failures,
    Status,
}

#[derive(Default)]
struct Verification {
    report: Report,
    strict: bool,
    warn: bool,
    ignore_missing: bool,
}

#[derive(Debug, thiserror::Error)]
enum ParseError {
    #[error("unrecognized option {0:?}")]
    Option(OsString),
    #[error("{0}")]
    Conflict(&'static str),
    #[error(transparent)]
    Jobs(#[from] jobs::InvalidCount),
}

enum Request {
    Help,
    Version,
    Run(Options, jobs::Count),
}

impl Options {
    fn parse(
        mut args: impl Iterator<Item = OsString>,
        mut workers: jobs::Count,
    ) -> Result<Request, ParseError> {
        let mut options = Self::default();
        let mut flags = true;
        while let Some(arg) = args.next() {
            if flags {
                if let Some(count) = jobs::option(arg.as_encoded_bytes(), &mut args) {
                    workers = count?;
                    continue;
                }
            }
            match arg.as_encoded_bytes() {
                b"--" if flags => flags = false,
                b"--help" if flags => return Ok(Request::Help),
                b"--version" if flags => return Ok(Request::Version),
                b"--tag" if flags => {
                    options.tagged = true;
                    options.mode = InputMode::Binary;
                }
                b"--ignore-missing" if flags => options.verification.ignore_missing = true,
                b"--quiet" if flags => {
                    options.verification.report = options.verification.report.max(Report::Failures);
                }
                b"--status" if flags => options.verification.report = Report::Status,
                b"--strict" if flags => options.verification.strict = true,
                b"--binary" if flags => options.flag(b'b'),
                b"--check" if flags => options.flag(b'c'),
                b"--text" if flags => options.flag(b't'),
                b"--warn" if flags => options.flag(b'w'),
                b"--zero" if flags => options.flag(b'z'),
                [b'-', short @ ..] if flags && !short.is_empty() => {
                    for (index, &flag) in short.iter().enumerate() {
                        if flag == b'j' {
                            workers = jobs::short_value(&short[index + 1..], &mut args)?;
                            break;
                        }
                        if !b"bctwz".contains(&flag) {
                            return Err(ParseError::Option(arg));
                        }
                        options.flag(flag);
                    }
                }
                _ => options.files.push(PathBuf::from(arg)),
            }
        }
        options.validate()?;
        if options.files.is_empty() {
            options.files.push(PathBuf::from("-"));
        }
        Ok(Request::Run(options, workers))
    }

    fn flag(&mut self, flag: u8) {
        match flag {
            b'b' => self.mode = InputMode::Binary,
            b'c' => self.check = true,
            b't' => self.mode = InputMode::Text,
            b'w' => self.verification.warn = true,
            b'z' => self.zero = true,
            _ => unreachable!("only parsed short flags reach this method"),
        }
    }

    fn validate(&self) -> Result<(), ParseError> {
        let verification = &self.verification;
        let conflict = if self.check && (self.mode != InputMode::Default || self.zero) {
            "verification does not accept output formatting options"
        } else if self.tagged && self.mode == InputMode::Text {
            "--tag does not support --text mode"
        } else if !self.check
            && (verification.report != Report::All
                || verification.strict
                || verification.warn
                || verification.ignore_missing)
        {
            "verification options require --check"
        } else {
            return Ok(());
        };
        Err(ParseError::Conflict(conflict))
    }
}

type HashResult = (PathBuf, io::Result<String>);

async fn execute(
    algorithm: Algorithm,
    options: Options,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let can_parallelize =
        !options.check && !ui.progress_enabled() && workers.get() > 1 && options.files.len() > 1;
    let digests = if can_parallelize {
        let paths = options.files.to_vec();
        let regular_paths = tokio::task::spawn_blocking(move || {
            let size = paths.iter().try_fold(0u64, |size, path| {
                if path == Path::new("-") {
                    return None;
                }
                let metadata = fs::metadata(path).ok()?;
                metadata
                    .is_file()
                    .then(|| size.saturating_add(metadata.len()))
            });
            size.is_some_and(|size| size >= PARALLEL_MIN_BYTES)
                .then_some(paths)
        })
        .await
        .map_err(io::Error::other)?;
        if let Some(paths) = regular_paths {
            Some(
                jobs::map(paths, workers, move |path| {
                    let result = File::open(&path).and_then(|mut file| {
                        digest(algorithm, &mut file, &mut vec![0; BUFFER_SIZE], |_| {})
                    });
                    (path, result)
                })
                .await?,
            )
        } else {
            None
        }
    } else {
        None
    };
    tokio::task::spawn_blocking(move || write_results(algorithm, &options, ui, digests))
        .await
        .map_err(io::Error::other)?
}

fn write_results(
    algorithm: Algorithm,
    options: &Options,
    ui: Presentation,
    digests: Option<Vec<HashResult>>,
) -> io::Result<u8> {
    let mut stdout = BufWriter::new(io::stdout().lock());
    let mut stderr = io::stderr().lock();
    let mut command = Checksum {
        algorithm,
        output: &mut stdout,
        errors: &mut stderr,
        ui,
        progress: ui.progress(algorithm.label(), "bytes", true),
        buffer: vec![0; BUFFER_SIZE],
    };
    let fail = if options.check {
        command.verify(options)?
    } else if let Some(digests) = digests {
        let mut fail = false;
        for (path, result) in digests {
            fail |= command.write_generated(&path, result, options)?;
        }
        fail
    } else {
        command.generate(options)?
    };
    command.output.flush()?;
    if let Some(progress) = &mut command.progress {
        progress.finish(command.errors);
    }
    Ok(u8::from(fail))
}

struct Checksum<'a, W> {
    algorithm: Algorithm,
    output: &'a mut dyn Write,
    errors: &'a mut W,
    ui: Presentation,
    progress: Option<ProgressReport>,
    buffer: Vec<u8>,
}

impl<W: Write> Checksum<'_, W> {
    fn generate(&mut self, options: &Options) -> io::Result<bool> {
        let mut fail = false;
        for path in &options.files {
            let result = self.hash_file(path);
            fail |= self.write_generated(path, result, options)?;
        }
        Ok(fail)
    }

    fn write_generated(
        &mut self,
        path: &Path,
        result: io::Result<String>,
        options: &Options,
    ) -> io::Result<bool> {
        match result.and_then(|digest| Ok((digest, record::filename(path.as_os_str())?))) {
            Ok((digest, name)) => {
                record::write(
                    self.output,
                    self.algorithm,
                    digest.as_bytes(),
                    name,
                    options,
                )?;
                Ok(false)
            }
            Err(error) => {
                self.error(&format!("{}: {error}", path.display()))?;
                Ok(true)
            }
        }
    }

    fn hash_file(&mut self, path: &Path) -> io::Result<String> {
        if path == Path::new("-") {
            return self.hash(&mut io::stdin().lock());
        }
        self.hash(&mut File::open(path)?)
    }

    fn hash(&mut self, input: &mut dyn Read) -> io::Result<String> {
        digest(self.algorithm, input, &mut self.buffer, |read| {
            if let Some(progress) = &mut self.progress {
                progress.advance(read, self.errors);
            }
        })
    }

    fn error(&mut self, message: &str) -> io::Result<()> {
        self.ui.error(self.errors, self.algorithm.label(), message)
    }
}
