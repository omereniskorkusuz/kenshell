//! App-wide error type. Serialized to the frontend as `{ code, detail }`; the frontend
//! maps `code` to a translated, human message (`errors.<code>` in i18n). `detail` is a
//! short technical hint for logs/tooltips and must NEVER contain secrets.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// Port is open in another application.
    PortBusy,
    /// Port path does not exist (cable unplugged?).
    PortNotFound,
    /// OS denied access (permissions, dialout group …).
    PermissionDenied,
    /// The device/cable disappeared during the session.
    DeviceGone,
    NotFound,
    Invalid,
    Unsupported,
    Io,
    Db,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}{}", detail.as_deref().map(|d| format!(": {d}")).unwrap_or_default())]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: Option<String>,
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }

    pub fn invalid(detail: impl Into<String>) -> Self {
        Self::with(ErrorCode::Invalid, detail)
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::with(ErrorCode::NotFound, detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::with(ErrorCode::Internal, detail)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as K;
        let code = match e.kind() {
            K::NotFound => ErrorCode::PortNotFound,
            K::PermissionDenied => ErrorCode::PermissionDenied,
            _ => ErrorCode::Io,
        };
        Self::with(code, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_detail() {
        let e = AppError::with(ErrorCode::PortBusy, "COM4");
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"code":"portBusy","detail":"COM4"}"#);
        assert_eq!(e.to_string(), "PortBusy: COM4");
    }
}
