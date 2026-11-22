use smallvec::SmallVec;

#[derive(Clone, Copy)]
enum Operator {
    Add,
    Remove,
    Assign,
}

#[derive(Clone, Copy)]
enum Class {
    User,
    Group,
    Other,
}

impl Class {
    const fn shift(self) -> u32 {
        match self {
            Self::User => 6,
            Self::Group => 3,
            Self::Other => 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Classes(u8);

impl Classes {
    const ALL: Self = Self(7);

    fn expand(self, bits: u32) -> u32 {
        [(1, 6), (2, 3), (4, 0)]
            .into_iter()
            .fold(0, |value, (class, shift)| {
                value
                    | if self.0 & class != 0 {
                        bits << shift
                    } else {
                        0
                    }
            })
    }

    fn ids(self) -> u32 {
        (if self.0 & 1 != 0 { 0o4000 } else { 0 }) | (if self.0 & 2 != 0 { 0o2000 } else { 0 })
    }

    fn sticky(self) -> u32 {
        if self.0 & 4 != 0 {
            0o1000
        } else {
            0
        }
    }
}

#[derive(Clone, Copy)]
enum Permissions {
    Letters {
        ordinary: u32,
        conditional_execute: bool,
        ids: bool,
        sticky: bool,
    },
    Copy(Class),
    Numeric(u32),
}

#[derive(Clone, Copy)]
enum Change {
    Absolute {
        bits: u32,
        clear_ids: bool,
    },
    Symbolic {
        classes: Option<Classes>,
        operator: Operator,
        permissions: Permissions,
    },
}

pub(crate) struct Mode {
    changes: SmallVec<[Change; 4]>,
}

#[derive(Clone, Copy)]
pub(super) struct Evaluated {
    pub(super) bits: u32,
    pub(super) affected: u32,
}

#[derive(Debug, thiserror::Error)]
#[error("invalid permission mode")]
pub(crate) struct InvalidMode;

impl Mode {
    pub(crate) fn parse(value: &str) -> Result<Self, InvalidMode> {
        if value.is_empty() {
            return Err(InvalidMode);
        }
        let mut changes = SmallVec::new();
        if value.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
            let bits = octal(value.as_bytes())?;
            changes.push(Change::Absolute {
                bits,
                clear_ids: value.len() >= 5,
            });
        } else {
            for clause in value.split(',') {
                parse_clause(clause.as_bytes(), &mut changes)?;
            }
        }
        Ok(Self { changes })
    }

    pub(super) fn evaluate(&self, umask: u32) -> Evaluated {
        self.apply(0o777, umask, true)
    }

    #[cfg(unix)]
    pub(crate) fn fifo_permissions(&self, umask: u32) -> Result<u32, InvalidMode> {
        let bits = self.apply(0o666, umask, false).bits;
        if bits > 0o777 {
            return Err(InvalidMode);
        }
        Ok(bits)
    }

    fn apply(&self, bits: u32, umask: u32, directory: bool) -> Evaluated {
        let mut result = Evaluated { bits, affected: 0 };
        for change in &self.changes {
            match *change {
                Change::Absolute { bits, clear_ids } => {
                    result.bits = bits;
                    result.affected |= 0o1777 | if clear_ids { 0o6000 } else { bits & 0o6000 };
                }
                Change::Symbolic {
                    classes,
                    operator,
                    permissions,
                } => {
                    let selected = classes.unwrap_or(Classes::ALL);
                    let (mut value, mut affected) = match permissions {
                        Permissions::Numeric(bits) => (
                            bits,
                            if matches!(operator, Operator::Assign) {
                                0o7777
                            } else {
                                bits
                            },
                        ),
                        Permissions::Copy(class) => {
                            let bits = selected.expand((result.bits >> class.shift()) & 7);
                            (bits, selected.expand(7) | selected.sticky())
                        }
                        Permissions::Letters {
                            ordinary,
                            conditional_execute,
                            ids,
                            sticky,
                        } => {
                            let execute =
                                conditional_execute && (directory || result.bits & 0o111 != 0);
                            let ordinary = ordinary | u32::from(execute);
                            let bits = selected.expand(ordinary)
                                | if ids { selected.ids() } else { 0 }
                                | if sticky { selected.sticky() } else { 0 };
                            (
                                bits,
                                selected.expand(7)
                                    | selected.sticky()
                                    | if ids { selected.ids() } else { 0 },
                            )
                        }
                    };
                    if classes.is_none() && !matches!(permissions, Permissions::Numeric(_)) {
                        value &= !(umask & 0o777);
                    }
                    if !matches!(operator, Operator::Assign) {
                        affected = value;
                    }
                    result.bits = match operator {
                        Operator::Add => result.bits | value,
                        Operator::Remove => result.bits & !value,
                        Operator::Assign => (result.bits & !affected) | value,
                    };
                    result.affected |= affected;
                }
            }
        }
        result
    }
}

fn octal(bytes: &[u8]) -> Result<u32, InvalidMode> {
    // Both callers have already established that the first byte is a digit.
    bytes.iter().try_fold(0u32, |value, byte| {
        if !matches!(byte, b'0'..=b'7') {
            return Err(InvalidMode);
        }
        value
            .checked_mul(8)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .filter(|value| *value <= 0o7777)
            .ok_or(InvalidMode)
    })
}

fn parse_clause(bytes: &[u8], changes: &mut SmallVec<[Change; 4]>) -> Result<(), InvalidMode> {
    let mut index = 0;
    let mut who = 0;
    while let Some(class) = bytes.get(index).and_then(|byte| match byte {
        b'u' => Some(1),
        b'g' => Some(2),
        b'o' => Some(4),
        b'a' => Some(7),
        _ => None,
    }) {
        who |= class;
        index += 1;
    }
    let classes = (who != 0).then_some(Classes(who));
    if index == bytes.len() {
        return Err(InvalidMode);
    }
    while index < bytes.len() {
        let operator = match bytes[index] {
            b'+' => Operator::Add,
            b'-' => Operator::Remove,
            b'=' => Operator::Assign,
            _ => return Err(InvalidMode),
        };
        index += 1;
        let start = index;
        while index < bytes.len() && !matches!(bytes[index], b'+' | b'-' | b'=') {
            index += 1;
        }
        let body = &bytes[start..index];
        let permissions = if body.first().is_some_and(u8::is_ascii_digit) {
            if classes.is_some() || start != 1 || index != bytes.len() {
                return Err(InvalidMode);
            }
            Permissions::Numeric(octal(body)?)
        } else {
            letters(body)?
        };
        changes.push(Change::Symbolic {
            classes,
            operator,
            permissions,
        });
    }
    Ok(())
}

fn letters(bytes: &[u8]) -> Result<Permissions, InvalidMode> {
    if let [class @ (b'u' | b'g' | b'o')] = bytes {
        return Ok(Permissions::Copy(match class {
            b'u' => Class::User,
            b'g' => Class::Group,
            _ => Class::Other,
        }));
    }
    let mut ordinary = 0;
    let mut conditional_execute = false;
    let mut ids = false;
    let mut sticky = false;
    for byte in bytes {
        match byte {
            b'r' => ordinary |= 4,
            b'w' => ordinary |= 2,
            b'x' => ordinary |= 1,
            b'X' => conditional_execute = true,
            b's' => ids = true,
            b't' => sticky = true,
            _ => return Err(InvalidMode),
        }
    }
    Ok(Permissions::Letters {
        ordinary,
        conditional_execute,
        ids,
        sticky,
    })
}
