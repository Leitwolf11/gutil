use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};

use super::{Command, Error, Info};

#[derive(Clone, Copy)]
#[repr(u8)]
enum Field {
    Kernel,
    Node,
    Release,
    Version,
    Machine,
    Processor,
    Platform,
    Os,
}

const FIELDS: [(Field, u8, &str, &str); 8] = [
    (Field::Kernel, b's', "kernel-name", "Kernel name"),
    (Field::Node, b'n', "nodename", "Node name"),
    (Field::Release, b'r', "kernel-release", "Kernel release"),
    (Field::Version, b'v', "kernel-version", "Kernel version"),
    (Field::Machine, b'm', "machine", "Machine"),
    (Field::Processor, b'p', "processor", "Processor"),
    (
        Field::Platform,
        b'i',
        "hardware-platform",
        "Hardware platform",
    ),
    (Field::Os, b'o', "operating-system", "Operating system"),
];

#[derive(Clone, Copy)]
enum Display {
    Selected(u8),
    All { labeled: bool },
}

impl Display {
    fn field(&mut self, field: Field) {
        if let Self::Selected(mask) = self {
            *mask |= 1 << field as u8;
        }
    }

    fn all(&mut self, labeled: bool) {
        *self = Self::All {
            labeled: labeled || matches!(self, Self::All { labeled: true }),
        };
    }
}

enum Parse {
    Show(Display),
    Info(Info),
}

fn parse(command: Command, args: impl Iterator<Item = OsString>) -> Result<Parse, Error> {
    let mut options = true;
    let mut operand = None;
    let mut display = Display::Selected(0);
    for argument in args {
        let bytes = argument.as_encoded_bytes();
        if options && bytes == b"--" {
            options = false;
        } else if options && bytes.len() > 1 && bytes.starts_with(b"-") {
            if let Some(long) = bytes.strip_prefix(b"--") {
                let mut matches = ["help", "version", "all", "all-labeled"]
                    .into_iter()
                    .take(if command == Command::Arch { 2 } else { 4 })
                    .chain(
                        FIELDS
                            .iter()
                            .map(|entry| entry.2)
                            .filter(|_| command == Command::Uname),
                    )
                    .filter(|key| key.as_bytes().starts_with(long));
                if let Some(key) = matches
                    .next()
                    .filter(|key| key.as_bytes() == long || matches.next().is_none())
                {
                    match key {
                        "help" => return Ok(Parse::Info(Info::Help)),
                        "version" => return Ok(Parse::Info(Info::Version)),
                        "all" => display.all(false),
                        "all-labeled" => display.all(true),
                        field => {
                            if let Some(&(field, _, _, _)) =
                                FIELDS.iter().find(|entry| entry.2 == field)
                            {
                                display.field(field);
                            }
                        }
                    }
                    continue;
                }
                return Err(Error::Option(argument));
            }
            if command == Command::Arch {
                return Err(Error::Option(argument));
            }
            for &flag in &bytes[1..] {
                match flag {
                    b'a' => display.all(false),
                    b'A' => display.all(true),
                    flag => {
                        let Some(&(field, _, _, _)) = FIELDS.iter().find(|entry| entry.1 == flag)
                        else {
                            return Err(Error::Option(argument));
                        };
                        display.field(field);
                    }
                }
            }
        } else {
            if std::env::var_os("POSIXLY_CORRECT").is_some() {
                return Err(Error::Operand(argument));
            }
            operand.get_or_insert(argument);
        }
    }
    if let Some(argument) = operand {
        return Err(Error::Operand(argument));
    }
    if matches!(display, Display::Selected(0)) {
        display.field(if command == Command::Arch {
            Field::Machine
        } else {
            Field::Kernel
        });
    }
    Ok(Parse::Show(display))
}

pub(super) fn run(
    command: Command,
    args: impl Iterator<Item = OsString>,
    output: &mut impl Write,
) -> Result<u8, Error> {
    match parse(command, args)? {
        Parse::Info(info) => info.write(command, output)?,
        Parse::Show(display) => {
            #[cfg(unix)]
            let system = nix::sys::utsname::uname().map_err(io::Error::from)?;
            #[cfg(unix)]
            render(display, |field| unix_value(&system, field), output)?;
            #[cfg(windows)]
            render(display, windows_value, output)?;
        }
    }
    Ok(0)
}

fn render<'a>(
    display: Display,
    value: impl Fn(Field) -> Cow<'a, OsStr>,
    output: &mut impl Write,
) -> io::Result<()> {
    let mut first = true;
    for (field, _, _, label) in FIELDS {
        match display {
            Display::Selected(mask) if mask & (1 << field as u8) == 0 => continue,
            Display::All { .. } if matches!(field, Field::Processor | Field::Platform) => continue,
            _ => {}
        }
        if matches!(display, Display::All { labeled: true }) {
            write!(output, "{label}: ")?;
        } else if !first {
            output.write_all(b" ")?;
        }
        output.write_all(value(field).as_encoded_bytes())?;
        if matches!(display, Display::All { labeled: true }) {
            output.write_all(b"\n")?;
        }
        first = false;
    }
    if !matches!(display, Display::All { labeled: true }) {
        output.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(unix)]
fn unix_value(system: &nix::sys::utsname::UtsName, field: Field) -> Cow<'_, OsStr> {
    Cow::Borrowed(match field {
        Field::Node => system.nodename(),
        Field::Release => system.release(),
        Field::Version => system.version(),
        Field::Machine => system.machine(),
        Field::Processor | Field::Platform => OsStr::new("unknown"),
        Field::Os if cfg!(target_os = "linux") => OsStr::new("GNU/Linux"),
        #[cfg(target_os = "android")]
        Field::Os => OsStr::new("Android"),
        #[cfg(target_os = "android")]
        Field::Kernel => system.sysname(),
        #[cfg(not(target_os = "android"))]
        Field::Kernel | Field::Os => system.sysname(),
    })
}

#[cfg(windows)]
fn windows_value(field: Field) -> Cow<'static, OsStr> {
    use sysinfo::System;
    let result = match field {
        Field::Kernel => return Cow::Borrowed(OsStr::new("Windows_NT")),
        Field::Node => System::host_name(),
        Field::Release => System::os_version(),
        Field::Version => System::kernel_version(),
        Field::Machine => return Cow::Owned(System::cpu_arch().into()),
        Field::Processor | Field::Platform => return Cow::Borrowed(OsStr::new("unknown")),
        Field::Os => return Cow::Borrowed(OsStr::new("Windows")),
    };
    Cow::Owned(result.unwrap_or_else(|_| String::from("unknown")).into())
}
