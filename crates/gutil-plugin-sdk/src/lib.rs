//! Read the invocation of an gutil WebAssembly plug-in.

use std::env::{self, ArgsOs};
use std::fmt;
use std::iter::Skip;

/// Environment variable set by the gutil launcher.
pub const PROTOCOL_ENV: &str = "GUTIL_PLUGIN_PROTOCOL";
/// Protocol understood by this version of the SDK.
pub const PROTOCOL_VERSION: &str = "1";

/// The arguments forwarded to a plug-in, excluding the executable name.
pub struct Invocation {
    args: Skip<ArgsOs>,
}

impl Invocation {
    /// Connect to gutil and capture the argument iterator.
    ///
    /// # Errors
    ///
    /// Returns an error if the process was started without a protocol version or
    /// the supplied version is unsupported.
    pub fn from_env() -> Result<Self, ProtocolError> {
        match env::var(PROTOCOL_ENV) {
            Ok(version) if version == PROTOCOL_VERSION => Ok(Self {
                args: env::args_os().skip(1), // nosemgrep: args-os, rust.lang.security.args-os.args-os
            }),
            Err(env::VarError::NotPresent) => Err(ProtocolError::Missing),
            _ => Err(ProtocolError::Unsupported),
        }
    }

    /// Return the arguments supplied by the WASI launcher.
    pub fn into_args(self) -> Skip<ArgsOs> {
        self.args
    }
}

/// A launcher protocol mismatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    /// The launcher did not supply a protocol version.
    Missing,
    /// The launcher supplied a protocol version this SDK cannot read.
    Unsupported,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Missing => "start this plug-in with 'gutil plug-in use'",
            Self::Unsupported => "unsupported gutil plug-in protocol version",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ProtocolError {}
