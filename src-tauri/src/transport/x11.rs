//! X11 forwarding (P5-T09), the OpenSSH way: the server gets a random *fake*
//! MIT-MAGIC-COOKIE-1; every X11 channel it opens back to us must present exactly
//! that cookie in its connection setup, which we then replace with the real cookie of
//! the local display (from `xauth`) before passing the connection on. The real cookie
//! never leaves this computer; a channel with any other cookie is closed.

use std::path::PathBuf;

pub const PROTO: &str = "MIT-MAGIC-COOKIE-1";
/// X11 client setup header: order, pad, major, minor, name len, data len, pad.
const HEADER: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Display {
    /// Local socket (`:0` → `/tmp/.X11-unix/X0`, or XQuartz's launchd socket path).
    Unix(PathBuf),
    Tcp(String, u16),
}

/// Parses `DISPLAY`: `:0`, `:0.0`, `unix:1`, `localhost:10.0`, `host:2`, or an
/// absolute socket path (macOS XQuartz: `/private/tmp/com.apple.launchd.x/org.xquartz:0`).
/// Returns the display and its screen number. `unix_sockets` is false on Windows,
/// where `:N` means TCP port 6000 + N on this computer (VcXsrv, X410, Xming).
pub fn parse_display(s: &str, unix_sockets: bool) -> Option<(Display, u32)> {
    let s = s.trim();
    if s.starts_with('/') {
        return Some((Display::Unix(PathBuf::from(s)), 0));
    }
    let (host, rest) = s.rsplit_once(':')?;
    let (num, screen) = rest.split_once('.').unwrap_or((rest, "0"));
    let num: u16 = num.parse().ok()?;
    let screen: u32 = screen.parse().ok()?;
    if num > 1000 {
        return None;
    }
    let display = match host {
        "" | "unix" if unix_sockets => {
            Display::Unix(PathBuf::from(format!("/tmp/.X11-unix/X{num}")))
        }
        "" | "unix" => Display::Tcp("127.0.0.1".into(), 6000 + num),
        h => Display::Tcp(h.to_owned(), 6000 + num),
    };
    Some((display, screen))
}

/// The cookie in `xauth list` output (`host/unix:0  MIT-MAGIC-COOKIE-1  <hex>`).
pub fn parse_xauth(out: &str) -> Option<Vec<u8>> {
    out.lines().find_map(|l| {
        let mut f = l.split_whitespace();
        let _display = f.next()?;
        (f.next()? == PROTO).then_some(())?;
        hex_decode(f.next()?)
    })
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.len() > 512 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

const fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

/// Length of the whole setup packet once its 12-byte header is known.
pub fn setup_len(header: &[u8; HEADER]) -> Option<usize> {
    let be = match header[0] {
        b'B' => true,
        b'l' => false,
        _ => return None,
    };
    let u16_at = |i: usize| {
        let b = [header[i], header[i + 1]];
        usize::from(if be {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    Some(HEADER + pad4(u16_at(6)) + pad4(u16_at(8)))
}

/// Checks the fake cookie in a complete setup packet and returns the packet with the
/// real authorization (or none). Err when the cookie is not ours.
pub fn rewrite_setup(
    setup: &[u8],
    fake: &[u8],
    real: Option<&[u8]>,
) -> Result<Vec<u8>, &'static str> {
    let header: &[u8; HEADER] = setup
        .get(..HEADER)
        .and_then(|h| h.try_into().ok())
        .ok_or("short setup")?;
    if setup_len(header) != Some(setup.len()) {
        return Err("malformed setup");
    }
    let be = header[0] == b'B';
    let get = |i: usize| {
        let b = [header[i], header[i + 1]];
        usize::from(if be {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        })
    };
    let (name_len, data_len) = (get(6), get(8));
    let name = &setup[HEADER..HEADER + name_len];
    let data_at = HEADER + pad4(name_len);
    let data = &setup[data_at..data_at + data_len];
    if name != PROTO.as_bytes() || data != fake {
        return Err("wrong X11 authorization");
    }
    let put = |n: usize| -> [u8; 2] {
        let n = u16::try_from(n).unwrap_or(0);
        if be { n.to_be_bytes() } else { n.to_le_bytes() }
    };
    let (new_name, new_data): (&[u8], &[u8]) = match real {
        Some(r) => (PROTO.as_bytes(), r),
        None => (&[], &[]),
    };
    let mut out = header[..6].to_vec();
    out.extend_from_slice(&put(new_name.len()));
    out.extend_from_slice(&put(new_data.len()));
    out.extend_from_slice(&header[10..12]);
    out.extend_from_slice(new_name);
    out.resize(HEADER + pad4(new_name.len()), 0);
    out.extend_from_slice(new_data);
    out.resize(HEADER + pad4(new_name.len()) + pad4(new_data.len()), 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(be: bool, name: &[u8], data: &[u8]) -> Vec<u8> {
        let p = |n: usize| {
            let n = n as u16;
            if be { n.to_be_bytes() } else { n.to_le_bytes() }
        };
        let mut s = vec![if be { b'B' } else { b'l' }, 0];
        s.extend_from_slice(&p(11));
        s.extend_from_slice(&p(0));
        s.extend_from_slice(&p(name.len()));
        s.extend_from_slice(&p(data.len()));
        s.extend_from_slice(&[0, 0]);
        s.extend_from_slice(name);
        s.resize(HEADER + pad4(name.len()), 0);
        s.extend_from_slice(data);
        s.resize(HEADER + pad4(name.len()) + pad4(data.len()), 0);
        s
    }

    #[test]
    fn displays() {
        let unix = |p: &str| Display::Unix(PathBuf::from(p));
        assert_eq!(
            parse_display(":0", true),
            Some((unix("/tmp/.X11-unix/X0"), 0))
        );
        assert_eq!(
            parse_display(":1.2", true),
            Some((unix("/tmp/.X11-unix/X1"), 2))
        );
        assert_eq!(
            parse_display(":0", false),
            Some((Display::Tcp("127.0.0.1".into(), 6000), 0))
        );
        assert_eq!(
            parse_display("localhost:10.0", true),
            Some((Display::Tcp("localhost".into(), 6010), 0))
        );
        let xq = "/private/tmp/com.apple.launchd.AbC/org.xquartz:0";
        assert_eq!(parse_display(xq, true), Some((unix(xq), 0)));
        for bad in ["", "0", ":x", ":99999", "host:"] {
            assert_eq!(parse_display(bad, true), None, "{bad}");
        }
    }

    #[test]
    fn xauth_output() {
        let out = "mac.local/unix:0  MIT-MAGIC-COOKIE-1  0a1b2c3d4e5f60718293a4b5c6d7e8f9\n";
        assert_eq!(
            parse_xauth(out).map(|c| hex(&c)).as_deref(),
            Some("0a1b2c3d4e5f60718293a4b5c6d7e8f9")
        );
        assert_eq!(parse_xauth("x  XDM-AUTHORIZATION-1  00\n"), None);
        assert_eq!(parse_xauth(""), None);
    }

    #[test]
    fn cookie_is_swapped_only_when_it_is_ours() {
        let fake = [7u8; 16];
        let real = [9u8; 16];
        for be in [true, false] {
            let s = setup(be, PROTO.as_bytes(), &fake);
            assert_eq!(setup_len(s[..12].try_into().unwrap()), Some(s.len()));
            let out = rewrite_setup(&s, &fake, Some(&real)).unwrap();
            assert_eq!(out, setup(be, PROTO.as_bytes(), &real));
            assert_eq!(rewrite_setup(&s, &fake, None).unwrap(), setup(be, b"", b""));
            let other = setup(be, PROTO.as_bytes(), &[1u8; 16]);
            assert!(rewrite_setup(&other, &fake, Some(&real)).is_err());
            assert!(rewrite_setup(&setup(be, b"", b""), &fake, Some(&real)).is_err());
        }
        assert!(rewrite_setup(b"x", &fake, None).is_err());
    }
}
