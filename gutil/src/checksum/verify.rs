use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use super::{record, Checksum, Options, Report, Verification};

impl Checksum<'_> {
    pub(super) fn verify(&mut self, options: &Options) -> io::Result<bool> {
        let mut fail = false;
        for path in &options.files {
            let summary = if path == Path::new("-") {
                self.verify_list(&mut io::stdin().lock(), true, path, &options.verification)
            } else {
                File::open(path).and_then(|file| {
                    self.verify_list(
                        &mut BufReader::new(file),
                        false,
                        path,
                        &options.verification,
                    )
                })
            };
            match summary {
                Ok(summary) => fail |= self.report(path, &options.verification, &summary)?,
                Err(error) => {
                    fail = true;
                    self.error(&format!("{}: {error}", path.display()))?;
                }
            }
        }
        Ok(fail)
    }

    fn verify_list(
        &mut self,
        reader: &mut dyn BufRead,
        stdin: bool,
        path: &Path,
        options: &Verification,
    ) -> io::Result<Summary> {
        let mut summary = Summary::default();
        let mut line = Vec::new();
        let mut number = 0;
        while reader.read_until(b'\n', &mut line)? != 0 {
            number += 1;
            let content = line.strip_suffix(b"\n").unwrap_or(&line);
            let content = content.strip_suffix(b"\r").unwrap_or(content);
            if !content.starts_with(b"#") {
                let entry = record::parse(self.algorithm, content)
                    .filter(|entry| !stdin || entry.name.as_ref() != b"-");
                if let Some(record) = entry {
                    self.verify_record(&record, options, &mut summary)?;
                } else {
                    summary.malformed += 1;
                    if options.warn && options.report != Report::Status {
                        self.error(&format!(
                            "{}: {number}: improperly formatted checksum line",
                            path.display()
                        ))?;
                    }
                }
            }
            line.clear();
        }
        Ok(summary)
    }

    fn verify_record(
        &mut self,
        record: &record::Entry<'_>,
        options: &Verification,
        summary: &mut Summary,
    ) -> io::Result<()> {
        summary.records += 1;
        let path = record::path(&record.name);
        let result = path.and_then(|path| self.hash_file(&path));
        let status = match result {
            Ok(digest) if digest.as_bytes().eq_ignore_ascii_case(record.digest) => {
                summary.matched += 1;
                if options.report != Report::All {
                    return Ok(());
                }
                b": OK\n".as_slice()
            }
            Ok(_) => {
                summary.mismatched += 1;
                b": FAILED\n"
            }
            Err(error) if options.ignore_missing && error.kind() == io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(error) => {
                summary.unreadable += 1;
                self.error(&format!(
                    "{}: {error}",
                    String::from_utf8_lossy(&record.name)
                ))?;
                b": FAILED open or read\n"
            }
        };
        if options.report != Report::Status {
            let name = record::escape(&record.name);
            if matches!(name, std::borrow::Cow::Owned(_)) {
                self.output.write_all(b"\\")?;
            }
            self.output.write_all(&name)?;
            self.output.write_all(status)?;
        }
        Ok(())
    }

    fn report(
        &mut self,
        path: &Path,
        options: &Verification,
        summary: &Summary,
    ) -> io::Result<bool> {
        if options.report != Report::Status {
            if summary.records == 0 {
                self.error(&format!(
                    "{}: no properly formatted checksum lines found",
                    path.display()
                ))?;
            } else {
                for (count, note) in [
                    (summary.malformed, "improperly formatted checksum lines"),
                    (summary.mismatched, "computed checksums did not match"),
                    (summary.unreadable, "listed files could not be read"),
                ] {
                    if count != 0 {
                        self.ui.warning(
                            self.errors,
                            self.algorithm.label(),
                            &format!("{count} {note}"),
                        )?;
                    }
                }
                if options.ignore_missing
                    && summary.matched == 0
                    && summary.mismatched == 0
                    && summary.unreadable == 0
                {
                    self.error(&format!("{}: no file was verified", path.display()))?;
                }
            }
        }
        Ok(summary.matched == 0
            || summary.mismatched != 0
            || summary.unreadable != 0
            || (options.strict && summary.malformed != 0))
    }
}

#[derive(Default)]
struct Summary {
    records: usize,
    matched: usize,
    mismatched: usize,
    unreadable: usize,
    malformed: usize,
}
