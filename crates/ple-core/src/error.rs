//! Internal error type for `ple-core`.
//!
//! Public functions still return `Result<_, String>` (the cross-crate contract
//! with the desktop crate, `bindings` and `api_ui`). Internal helpers return
//! [`Error`], and `?` converts it to `String` through `From<Error> for String`.
//! `Display` reproduces the former message text exactly: wrapped I/O, JSON and
//! CSV errors print as their source's `to_string()`.
use std::{fmt, path::PathBuf};

#[derive(Debug)]
pub(crate) enum Error {
    /// Filesystem or stream I/O failure.
    Io(std::io::Error),
    /// JSON serialization or parsing failure.
    Json(serde_json::Error),
    /// CSV writer failure.
    Csv(csv::Error),
    /// A named numeric input was NaN or infinite.
    NonFinite(String),
    /// A raw CSV already exists; raw data is never replaced.
    RawCsvExists(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Json(e) => write!(f, "{e}"),
            Self::Csv(e) => write!(f, "{e}"),
            Self::NonFinite(name) => write!(f, "{name} must be finite"),
            Self::RawCsvExists(path) => write!(f, "Refusing to replace raw CSV {}", path.display()),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Csv(e) => Some(e),
            Self::NonFinite(_) | Self::RawCsvExists(_) => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<csv::Error> for Error {
    fn from(e: csv::Error) -> Self {
        Self::Csv(e)
    }
}

/// Keeps existing `Result<_, String>` signatures and call sites unchanged.
impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_matches_former_string_errors() {
        let io = std::io::Error::other("disk full");
        let expected = io.to_string();
        assert_eq!(String::from(Error::from(io)), expected);
        let json = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let expected = json.to_string();
        let error = Error::from(json);
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(&error).is_some());
        assert_eq!(
            Error::NonFinite("file power".into()).to_string(),
            "file power must be finite"
        );
        assert_eq!(
            Error::RawCsvExists(PathBuf::from("raw.csv")).to_string(),
            "Refusing to replace raw CSV raw.csv"
        );
    }
}
