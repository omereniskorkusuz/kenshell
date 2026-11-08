//! Local X server access for X11 forwarding (P5-T09): which display to use, its
//! authorization cookie (`xauth`) and the connection itself. macOS: XQuartz (its
//! launchd socket, `/opt/X11/bin/xauth`); Linux: `/tmp/.X11-unix`; Windows: an X server
//! such as VcXsrv / X410 on TCP 6000 + N.

use crate::transport::tunnel::Io;
use crate::transport::x11::{self, Display};

/// Unix-domain X sockets exist on this OS.
pub const UNIX_SOCKETS: bool = cfg!(unix);

fn output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes up from a GUI app.
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The local display: `$DISPLAY`; on macOS apps started from Finder do not inherit it,
/// so launchd's value (set by XQuartz) is asked for; Windows defaults to `:0`.
pub fn default_display() -> Option<String> {
    if let Ok(d) = std::env::var("DISPLAY")
        && !d.trim().is_empty()
    {
        return Some(d);
    }
    if cfg!(target_os = "macos") {
        return output("/bin/launchctl", &["getenv", "DISPLAY"]).filter(|d| !d.is_empty());
    }
    cfg!(windows).then(|| ":0".to_owned())
}

/// The display's MIT-MAGIC-COOKIE-1, when `xauth` has one (blocking).
pub fn cookie(display: &str) -> Option<Vec<u8>> {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &["/opt/X11/bin/xauth", "xauth"]
    } else {
        &["xauth"]
    };
    candidates
        .iter()
        .find_map(|p| output(p, &["list", display]))
        .and_then(|out| x11::parse_xauth(&out))
}

pub async fn connect(display: &Display) -> std::io::Result<Box<dyn Io>> {
    match display {
        Display::Tcp(host, port) => {
            let s = tokio::net::TcpStream::connect((host.as_str(), *port)).await?;
            Ok(Box::new(s))
        }
        #[cfg(unix)]
        Display::Unix(path) => Ok(Box::new(tokio::net::UnixStream::connect(path).await?)),
        #[cfg(not(unix))]
        Display::Unix(_) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "no unix sockets on this OS",
        )),
    }
}
