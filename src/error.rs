//! Errors raised while reading a schema or writing generated code.

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    /// The schema is readable but not something codegen can support.
    Schema(String),
    /// A feature of `hyperschema` 1.26.2 that this generator does not emit.
    Unsupported(String),
    Io(std::io::Error),
    /// `build` was called outside a build script.
    NoOutDir,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Json(e) => write!(f, "invalid schema json: {e}"),
            Error::Schema(m) => f.write_str(m),
            Error::Unsupported(m) => write!(f, "unsupported: {m}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::NoOutDir => {
                f.write_str("OUT_DIR is not set; build() must run from a build script")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Json(e) => Some(e),
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
