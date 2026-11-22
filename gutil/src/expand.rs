use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::Path;

use crate::jobs;
use crate::presentation::Presentation;

mod options;
mod stops;
mod stream;
mod unexpand;

use options::{Options, Request};
use stream::Expander;
use unexpand::Unexpander;

const HELP: &str = "Usage: gutil expand [OPTIONS] [FILE]...\n\nReplace tabs with spaces. Use '-' for standard input.\n\n  -i, --initial         Convert tabs only before the first nonblank\n  -t, --tabs LIST       Set a repeating width or a list of tab stops\n  -j, --jobs, --threads N  Limit runtime workers\n      --help            Print help\n      --version         Print version\n\nA final /N repeats every N columns from column zero. A final +N\nrepeats every N columns from the final explicit stop.\n";

const UNEXPAND_HELP: &str = "Usage: gutil unexpand [OPTIONS] [FILE]...\n\nReplace blanks with tabs. Use '-' for standard input.\n\n  -a, --all             Convert eligible blanks across the entire line\n      --first-only      Convert only leading blanks, overriding -a and -t\n  -t, --tabs LIST       Set tab stops and enable conversion across the line\n  -j, --jobs, --threads N  Limit runtime workers\n      --help            Print help\n      --version         Print version\n\nA final /N repeats every N columns from column zero. A final +N\nrepeats every N columns from the final explicit stop.\n";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Expand,
    Unexpand,
}

impl Command {
    fn label(self) -> &'static str {
        match self {
            Self::Expand => "gutil expand",
            Self::Unexpand => "gutil unexpand",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Self::Expand => HELP,
            Self::Unexpand => UNEXPAND_HELP,
        }
    }
}

pub(crate) fn run(
    command: Command,
    arguments: impl Iterator<Item = OsString>,
    workers: jobs::Count,
    ui: Presentation,
) -> io::Result<u8> {
    let label = command.label();
    let options = match Request::parse(command, arguments, workers) {
        Ok(Request::Help) => {
            return io::stdout()
                .write_all(command.help().as_bytes())
                .map(|()| 0)
        }
        Ok(Request::Version) => {
            return writeln!(io::stdout(), "{label} {}", env!("CARGO_PKG_VERSION")).map(|()| 0);
        }
        Ok(Request::Convert(options)) => options,
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), label, &error.to_string());
            return Ok(1);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(options.workers.get())
        .build()?;
    match runtime.block_on(async move {
        tokio::task::spawn_blocking(move || execute(command, &options, ui))
            .await
            .map_err(io::Error::other)?
    }) {
        Ok(status) => Ok(status),
        Err(error) => {
            let _ = ui.error(&mut io::stderr(), label, &error.to_string());
            Ok(1)
        }
    }
}

fn execute(command: Command, options: &Options, ui: Presentation) -> io::Result<u8> {
    let label = command.label();
    let mut converter = match command {
        Command::Expand => Converter::Expand(Expander::new(options)),
        Command::Unexpand => Converter::Unexpand(Unexpander::new(options)),
    };
    let utf8 = stream::utf8_locale();
    let mut input_buffer = vec![0; stream::BUFFER_SIZE + 3];
    let stdout = io::stdout();
    let capacity = if stdout.is_terminal() {
        0
    } else {
        stream::BUFFER_SIZE
    };
    let mut output = BufWriter::with_capacity(capacity, stdout.lock());
    let mut errors = io::stderr().lock();
    let mut progress = ui.progress(label, "bytes", true);
    let mut status = 0;
    for path in &options.paths {
        let read_error = if path == Path::new("-") {
            converter.copy(
                &mut io::stdin().lock(),
                &mut output,
                &mut input_buffer,
                &mut progress,
                &mut errors,
                utf8,
            )?
        } else {
            match File::open(path) {
                Ok(mut file) => converter.copy(
                    &mut file,
                    &mut output,
                    &mut input_buffer,
                    &mut progress,
                    &mut errors,
                    utf8,
                )?,
                Err(error) => Some(error),
            }
        };
        if let Some(error) = read_error {
            let _ = ui.error(&mut errors, label, &format!("{}: {error}", path.display()));
            status = 1;
        }
    }
    if let Converter::Unexpand(converter) = &mut converter {
        converter.finish(&mut output)?;
    }
    output.flush()?;
    if let Some(progress) = &mut progress {
        progress.finish(&mut errors);
    }
    Ok(status)
}

enum Converter<'a> {
    Expand(Expander<'a>),
    Unexpand(Unexpander<'a>),
}

impl Converter<'_> {
    fn copy(
        &mut self,
        input: &mut impl io::Read,
        output: &mut impl Write,
        buffer: &mut [u8],
        progress: &mut Option<crate::presentation::ProgressReport>,
        errors: &mut impl Write,
        utf8: bool,
    ) -> io::Result<Option<io::Error>> {
        stream::copy_input(
            input,
            output,
            buffer,
            progress,
            errors,
            utf8,
            |bytes, output| match self {
                Self::Expand(converter) => converter.process(bytes, utf8, output),
                Self::Unexpand(converter) => converter.process(bytes, utf8, output),
            },
        )
    }
}
