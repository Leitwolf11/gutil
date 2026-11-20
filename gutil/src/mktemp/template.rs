use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::native::{self, Unit};
use super::options::Error;

pub(super) struct Template {
    prefix: OsString,
    suffix: OsString,
    random_len: usize,
}

impl Template {
    pub(super) fn parse(value: &OsStr, suffix: Option<OsString>) -> Result<Self, Error> {
        let units = native::units(value);
        if units.contains(&0) {
            return Err(Error::Template);
        }
        let component = units
            .iter()
            .rposition(|unit| native::separator(*unit))
            .map_or(0, |index| index + 1);
        let x: Unit = 88; // ASCII X in either native encoding.
        let end = if suffix.is_some() {
            if units.last() != Some(&x) {
                return Err(Error::SuffixTemplate);
            }
            units.len()
        } else {
            units[component..]
                .iter()
                .rposition(|unit| *unit == x)
                .map_or(component, |index| component + index + 1)
        };
        let mut start = end;
        while start > component && units[start - 1] == x {
            start -= 1;
        }
        if end - start < 3 {
            return Err(Error::Template);
        }
        let suffix = suffix.unwrap_or_else(|| native::string(&units[end..]));
        if native::units(&suffix)
            .iter()
            .any(|unit| native::separator(*unit) || *unit == 0)
        {
            return Err(Error::Suffix);
        }
        #[cfg(windows)]
        {
            let mut shape = native::string(&units[..end]);
            shape.push(&suffix);
            if std::path::Path::new(&shape)
                .file_name()
                .is_some_and(|name| name.as_encoded_bytes().contains(&b':'))
            {
                return Err(Error::AlternateStream);
            }
        }
        Ok(Self {
            prefix: native::string(&units[..start]),
            suffix,
            random_len: end - start,
        })
    }

    pub(super) fn name(&self, root: Option<&Path>) -> std::io::Result<PathBuf> {
        const ALPHABET: &[u8; 62] =
            b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut name = self.prefix.clone();
        let mut random = String::with_capacity(self.random_len);
        let mut buffer = [0_u8; 64];
        while random.len() < self.random_len {
            getrandom::fill(&mut buffer)?;
            for byte in buffer {
                // Rejection sampling keeps all 62 characters equally likely.
                if byte < 248 {
                    random.push(char::from(ALPHABET[usize::from(byte % 62)]));
                    if random.len() == self.random_len {
                        break;
                    }
                }
            }
        }
        name.push(random);
        name.push(&self.suffix);
        Ok(root.map_or_else(|| PathBuf::from(&name), |root| root.join(&name)))
    }
}

pub(super) fn has_separator(value: &OsStr) -> bool {
    native::units(value).into_iter().any(native::separator)
}
