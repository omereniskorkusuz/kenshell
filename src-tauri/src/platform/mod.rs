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

/// How long a just-closed port may still be held by our own previous session (its
/// reader thread exits within the serial read timeout).
const BUSY_RETRY: std::time::Duration = std::time::Duration::from_millis(150);
const BUSY_ATTEMPTS: u32 = 5;

/// Opens a serial port, retrying briefly while it is busy: closing a tab and opening
/// the same port again right away must not fail just because the old session's reader
/// thread has not let go yet. A port really held by another app still fails (~0.6 s).
/// Blocking — call from a blocking context.
pub fn open_serial(
    builder: &serialport::SerialPortBuilder,
    path: &str,
) -> Result<Box<dyn serialport::SerialPort>, AppError> {
    let mut attempt = 1;
    loop {
        match builder.clone().open() {
            Ok(p) => return Ok(p),
            Err(e) => {
                let err = classify_open_error(path, &e);
                if err.code != ErrorCode::PortBusy || attempt >= BUSY_ATTEMPTS {
                    return Err(err);
                }
                attempt += 1;
                std::thread::sleep(BUSY_RETRY);
            }
        }
    }
}

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
