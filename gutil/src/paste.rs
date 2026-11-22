use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use crate::jobs;
use crate::presentation::Presentation;

mod options;

use options::{Options, Request};

const LABEL: &str = "gutil paste";
const HELP: &str = "Usage: gutil paste [OPTIONS] [FILE]...\n\nMerge corresponding records from each file. Use '-' for standard input.\n\n  -d, --delimiters LIST  Cycle through the delimiters in LIST\n  -s, --serial           Merge records within each file\n  -z, --zero-terminated  Read and write NUL-terminated records\n  -j, --jobs, --threads N  Limit runtime workers\n      --help             Print help\n      --version          Print version\n";

pub(crate) fn run(
    args: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let options = match Options::parse(args, workers) {
        Ok(Request::Help) => return io::stdout().write_all(HELP.as_bytes()).map(|()| 0),
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{LABEL} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Run(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    let result = runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(&options, ui))
            .await
            .map_err(io::Error::other)?
    });
    match result {
        Ok(status) => Ok(status),
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), LABEL, &error.to_string());
            Ok(1)
        }
    }
}

enum Input {
    Stdin,
    File(BufReader<File>),
    Finished,
}

impl Input {
    fn open(path: &Path) -> io::Result<Self> {
        if path == Path::new("-") {
            Ok(Self::Stdin)
        } else {
            File::open(path).map(|file| Self::File(BufReader::new(file)))
        }
    }

    fn read(
        &mut self,
        stdin: &mut impl BufRead,
        terminator: u8,
        output: &mut Vec<u8>,
    ) -> io::Result<usize> {
        let count = match self {
            Self::Stdin => stdin.read_until(terminator, output)?,
            Self::File(file) => file.read_until(terminator, output)?,
            Self::Finished => return Ok(0),
        };
        if count == 0 {
            *self = Self::Finished;
        } else if output.last() == Some(&terminator) {
            output.pop();
        }
        Ok(count)
    }
}

struct Merge<'a, R, W, E> {
    options: &'a Options,
    stdin: R,
    output: W,
    errors: E,
    ui: Presentation,
    progress: Option<crate::presentation::ProgressReport>,
    failed: bool,
}

impl<R: BufRead, W: Write, E: Write> Merge<'_, R, W, E> {
    fn report(&mut self, path: &Path, error: &io::Error) {
        self.failed = true;
        let _ = self.ui.error(
            &mut self.errors,
            LABEL,
            &format!("{}: {error}", path.display()),
        );
    }

    fn record(&mut self, path: &Path, input: &mut Input, output: &mut Vec<u8>) -> bool {
        let start = output.len();
        match input.read(&mut self.stdin, self.options.terminator, output) {
            Ok(count) => count != 0,
            Err(error) => {
                *input = Input::Finished;
                self.report(path, &error);
                output.len() != start
            }
        }
    }

    fn end_row(&mut self) -> io::Result<()> {
        self.output.write_all(&[self.options.terminator])?;
        if let Some(progress) = &mut self.progress {
            progress.advance(1, &mut self.errors);
        }
        Ok(())
    }

    fn parallel(&mut self) -> io::Result<()> {
        let mut inputs = Vec::with_capacity(self.options.paths.len());
        for path in &self.options.paths {
            match Input::open(path) {
                Ok(input) => inputs.push(input),
                Err(error) => {
                    self.report(path, &error);
                    return Ok(());
                }
            }
        }
        let mut row = Vec::new();
        loop {
            let mut any = false;
            row.clear();
            for (index, (path, input)) in self.options.paths.iter().zip(&mut inputs).enumerate() {
                if index != 0 {
                    row.extend_from_slice(self.options.delimiter(index - 1));
                }
                any |= self.record(path, input, &mut row);
            }
            if !any {
                return Ok(());
            }
            self.output.write_all(&row)?;
            self.end_row()?;
        }
    }

    fn serial(&mut self) -> io::Result<()> {
        let mut record = Vec::new();
        for path in &self.options.paths {
            let mut input = match Input::open(path) {
                Ok(input) => input,
                Err(error) => {
                    self.report(path, &error);
                    continue;
                }
            };
            let mut delimiter = None;
            loop {
                record.clear();
                if !self.record(path, &mut input, &mut record) {
                    break;
                }
                if let Some(index) = delimiter {
                    self.output.write_all(self.options.delimiter(index))?;
                }
                self.output.write_all(&record)?;
                delimiter = Some(delimiter.map_or(0, |index| index + 1));
            }
            self.end_row()?;
        }
        Ok(())
    }
}

fn execute(options: &Options, ui: Presentation) -> io::Result<u8> {
    let mut merger = Merge {
        options,
        stdin: io::stdin().lock(),
        output: BufWriter::with_capacity(64 * 1024, io::stdout().lock()),
        errors: io::stderr().lock(),
        ui,
        progress: ui.progress(LABEL, "rows", true),
        failed: false,
    };
    if options.serial {
        merger.serial()?;
    } else {
        merger.parallel()?;
    }
    merger.output.flush()?;
    if let Some(progress) = &mut merger.progress {
        progress.finish(&mut merger.errors);
    }
    Ok(u8::from(merger.failed))
}
