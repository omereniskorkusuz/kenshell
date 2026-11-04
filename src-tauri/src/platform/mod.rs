//! Platform-specific helpers. Keep OS conditionals here, not in domain modules.

use crate::error::{AppError, ErrorCode};

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod imp;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[path = "linux.rs"]
mod imp;

pub use imp::port_exists;

/// Maps a serial-port open error to a friendly [`AppError`] (P1-T11).
///
/// serialport reports a locked port as `NoDevice` (unix: EBUSY from TIOCEXCL/flock;
/// Windows: ERROR_ACCESS_DENIED — but also FILE_NOT_FOUND). We disambiguate by checking
/// whether the port still exists: exists → busy (another app has it), else not found.
pub fn classify_open_error(path: &str, err: &serialport::Error) -> AppError {
    use serialport::ErrorKind as K;
    use std::io::ErrorKind as Io;
    let code = match err.kind() {
        K::NoDevice if port_exists(path) => ErrorCode::PortBusy,
        K::NoDevice | K::Io(Io::NotFound) => ErrorCode::PortNotFound,
        K::Io(Io::PermissionDenied) => ErrorCode::PermissionDenied,
        K::InvalidInput => ErrorCode::Invalid,
        _ => ErrorCode::Io,
    };
    // `detail` is a short technical hint; serialport descriptions never contain secrets.
    AppError::with(code, format!("{path}: {}", err.description))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_port_is_not_found_and_existing_locked_port_is_busy() {
        let e = serialport::Error::new(serialport::ErrorKind::NoDevice, "busy");
        let missing = if cfg!(windows) {
            "COM250"
        } else {
            "/dev/cu.kenshell-does-not-exist"
        };
        assert_eq!(
            classify_open_error(missing, &e).code,
            ErrorCode::PortNotFound
        );
        #[cfg(unix)]
        assert_eq!(
            classify_open_error("/dev/null", &e).code,
            ErrorCode::PortBusy
        );
    }

    #[test]
    fn permission_denied() {
        let e = serialport::Error::new(
            serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied),
            "EACCES",
        );
        assert_eq!(
            classify_open_error("x", &e).code,
            ErrorCode::PermissionDenied
        );
    }
}
