use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use rusty_rich::progress::Task;
use rusty_rich::progress_columns::{BarColumn, ProgressColumn, SpinnerColumn};
use rusty_rich::{Progress, Style};

use crate::config::{DiagnosticFormat, Diagnostics, Settings, Verbosity, When};

#[derive(Clone, Copy)]
pub(crate) struct Presentation {
    diagnostics: Diagnostics,
    progress_mode: When,
    terminal: bool,
    color: bool,
}

impl Presentation {
    pub(crate) fn new(settings: Settings) -> Self {
        let terminal = io::stderr().is_terminal();
        Self {
            diagnostics: settings.diagnostics,
            progress_mode: settings.global.progress,
            terminal,
            color: color_enabled(settings.output.color, terminal),
        }
    }

    pub(crate) fn error(
        self,
        writer: &mut impl Write,
        command: &str,
        message: &str,
    ) -> io::Result<()> {
        self.diagnostic(writer, command, "error", message)
    }

    pub(crate) fn warning(
        self,
        writer: &mut impl Write,
        command: &str,
        message: &str,
    ) -> io::Result<()> {
        if self.diagnostics.verbosity == Verbosity::Quiet {
            return Ok(());
        }
        self.diagnostic(writer, command, "warning", message)
    }

    pub(crate) fn context(self, writer: &mut impl Write, message: &str) -> io::Result<()> {
        if self.diagnostics.verbosity != Verbosity::Verbose {
            return Ok(());
        }
        self.diagnostic(writer, "gutil", "info", message)
    }

    fn diagnostic(
        self,
        writer: &mut impl Write,
        command: &str,
        level: &str,
        message: &str,
    ) -> io::Result<()> {
        if self.diagnostics.format == DiagnosticFormat::Json {
            return writeln!(
                writer,
                "{}",
                serde_json::json!({
                    "type": "diagnostic", "level": level, "command": command, "message": message,
                })
            );
        }
        let label = if self.color {
            let style = match level {
                "error" => "bold red",
                "warning" => "bold yellow",
                _ => "bold cyan",
            };
            Style::from_str(style).render(command)
        } else {
            command.to_owned()
        };
        writeln!(writer, "{label}: {message}")
    }

    pub(crate) fn progress(
        self,
        command: &'static str,
        unit: &'static str,
    ) -> Option<ProgressReport> {
        let mode = self.progress_mode;
        mode.enabled(self.terminal).then(|| ProgressReport {
            presentation: self,
            command,
            unit,
            amount: 0,
            total: None,
            started: Instant::now(),
            last: None,
            task: None,
        })
    }
}

pub(crate) fn color_enabled(mode: When, terminal: bool) -> bool {
    mode == When::Always
        || (mode == When::Auto && terminal && std::env::var_os("NO_COLOR").is_none())
}

pub(crate) struct ProgressReport {
    presentation: Presentation,
    command: &'static str,
    unit: &'static str,
    amount: u64,
    total: Option<u64>,
    started: Instant,
    last: Option<Instant>,
    task: Option<(Progress, usize)>,
}

impl ProgressReport {
    pub(crate) fn set_total(&mut self, total: u64) {
        self.total = Some(total);
    }

    pub(crate) fn advance(&mut self, amount: u64, writer: &mut impl Write) {
        self.amount = self.amount.saturating_add(amount);
        self.emit(writer, false);
    }

    pub(crate) fn finish(&mut self, writer: &mut impl Write) {
        self.emit(writer, true);
    }

    #[allow(clippy::cast_precision_loss)] // Rich renders approximate widths; JSON retains exact counts.
    fn emit(&mut self, writer: &mut impl Write, finished: bool) {
        let now = Instant::now();
        let mode = self.presentation.progress_mode;
        if (mode == When::Auto && now.duration_since(self.started) < Duration::from_millis(250))
            || (!finished
                && self
                    .last
                    .is_some_and(|last| now.duration_since(last) < Duration::from_millis(250)))
        {
            return;
        }
        self.last = Some(now);
        if self.presentation.diagnostics.format == DiagnosticFormat::Json {
            let _ = writeln!(
                writer,
                "{}",
                serde_json::json!({
                    "type": "progress", "command": self.command, "unit": self.unit,
                    "completed": self.amount, "total": self.total, "finished": finished,
                })
            );
            return;
        }
        let (progress, id) = self.task.get_or_insert_with(|| {
            let mut columns: Vec<Box<dyn ProgressColumn>> = Vec::new();
            if self.presentation.terminal && self.presentation.color {
                if self.total.is_some() {
                    columns.push(Box::new(BarColumn::new().width(16)));
                } else {
                    columns.push(Box::new(
                        SpinnerColumn::new().style(Style::from_str("cyan")),
                    ));
                }
            }
            columns.push(Box::new(Description));
            let mut progress = Progress::new().with_columns(columns);
            let id = progress.add_task("", self.total.map(|total| total as f64));
            (progress, id)
        });
        if let Some(task) = progress.tasks.get_mut(id) {
            let count = if self.unit == "bytes" && self.presentation.terminal {
                rusty_rich::filesize::format_size(self.amount, None)
            } else {
                format!("{} {}", self.amount, self.unit)
            };
            let count = if let Some(total) = self.total {
                let total = if self.unit == "bytes" && self.presentation.terminal {
                    rusty_rich::filesize::format_size(total, None)
                } else {
                    format!("{total} {}", self.unit)
                };
                format!("{count} / {total}")
            } else {
                count
            };
            task.completed = self.amount as f64;
            if finished && task.total.is_none() {
                task.total = Some(task.completed);
            }
            let state = if finished { "done" } else { "working" };
            task.description = format!("{}: {count} ({state})", self.command);
        }
        if finished {
            progress.stop_task(*id);
        }
        let line = progress.render(80);
        let _ = writer.write_all(line.as_bytes());
    }
}

#[derive(Debug)]
struct Description;

impl ProgressColumn for Description {
    fn render(&self, task: &Task, _: usize, _: Duration) -> String {
        task.description.clone()
    }
}
