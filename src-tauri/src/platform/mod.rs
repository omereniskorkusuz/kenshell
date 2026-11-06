//! Platform-specific helpers. Keep OS conditionals here, not in domain modules.

pub mod net;

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
/// Writes a private key (or other secret file): owner read/write only on Unix
/// (`0600`, like ssh-keygen); on Windows the file inherits the user profile ACL.
pub fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    // An existing file keeps its old mode with `create`; tighten it explicitly.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(data)?;
    f.sync_all()
}

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
    #[test]
    #[cfg(unix)]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ks-priv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("key");
        std::fs::write(&p, b"old").unwrap(); // 0644 by default
        super::write_private(&p, b"secret").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::read(&p).unwrap(), b"secret");
        std::fs::remove_dir_all(dir).unwrap();
    }

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
