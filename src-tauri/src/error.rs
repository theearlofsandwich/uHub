//! The error type of the commands and the code behind them. It reaches the
//! shell as `{ kind, message }`: `message` is user-facing, `kind` a stable
//! camelCase tag (see `ErrorKind` in src/services/types.ts).

use serde::{ser::SerializeStruct, Serialize, Serializer};
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown service {0}")]
    UnknownService(String),
    /// The service's display name.
    #[error("{0} is disabled")]
    ServiceDisabled(String),
    #[error("main window missing")]
    WebviewMissing,
    #[error("{0}")]
    CreateFailed(String),
    /// A config value or command argument that failed validation.
    #[error("{0}")]
    InvalidConfig(String),
    #[error("{context}: {source}")]
    Io { context: String, source: io::Error },
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidConfig(message.into())
    }

    /// An I/O error with what was being done.
    pub fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io { context: context.into(), source }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::UnknownService(_) => "unknownService",
            Self::ServiceDisabled(_) => "serviceDisabled",
            Self::WebviewMissing => "webviewMissing",
            Self::CreateFailed(_) => "createFailed",
            Self::InvalidConfig(_) => "invalidConfig",
            Self::Io { .. } => "io",
            Self::Tauri(_) => "tauri",
        }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("Error", 2)?;
        s.serialize_field("kind", self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_kind_and_message() {
        let json = |e: Error| serde_json::to_value(e).unwrap();
        assert_eq!(
            json(Error::UnknownService("x".into())),
            serde_json::json!({"kind": "unknownService", "message": "unknown service x"})
        );
        let e = Error::io("saving a.json", io::Error::other("disk full"));
        assert_eq!(json(e), serde_json::json!({"kind": "io", "message": "saving a.json: disk full"}));
    }
}
