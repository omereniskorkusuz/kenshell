//! SCP (P5-T01): the classic `scp -f` / `scp -t` protocol over an SSH exec channel. Cisco
//! IOS/IOS-XE, NX-OS and Ruijie RGOS run an SCP server; one file per transfer.
//!
//! The protocol code is generic over the stream so it is tested against a scripted
//! server; the session runs it over `Channel::into_stream`.

use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{AppError, ErrorCode, Result};

/// Largest file accepted from a device (firmware images are a few hundred MB).
pub const MAX_FILE: u64 = 4 * 1024 * 1024 * 1024;
const CHUNK: usize = 32 * 1024;

/// The remote shell command: paths are quoted only when they need it (Cisco's SCP
/// server takes `flash:file` as is, quotes would be part of the name).
pub fn command(mode: char, path: &str) -> String {
    let plain = path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-:+@%".contains(c));
    let p = if plain {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    };
    format!("scp -{mode} {p}")
}

fn remote_error(msg: &[u8]) -> AppError {
    let text = String::from_utf8_lossy(msg)
        .trim()
        .chars()
        .take(300)
        .collect::<String>();
    AppError::with(ErrorCode::Io, format!("scp: {text}"))
}

async fn read_ack<S: AsyncRead + Unpin>(s: &mut S) -> Result<()> {
    let b = s.read_u8().await.map_err(AppError::from)?;
    match b {
        0 => Ok(()),
        1 | 2 => {
            let mut msg = Vec::new();
            loop {
                let c = s.read_u8().await.map_err(AppError::from)?;
                if c == b'\n' || msg.len() > 1024 {
                    break;
                }
                msg.push(c);
            }
            Err(remote_error(&msg))
        }
        other => Err(AppError::with(
            ErrorCode::Io,
            format!("scp: unexpected reply byte {other}"),
        )),
    }
}

async fn read_line<S: AsyncRead + Unpin>(s: &mut S) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let c = s.read_u8().await.map_err(AppError::from)?;
        if c == b'\n' {
            return Ok(line);
        }
        if line.len() > 4096 {
            return Err(AppError::invalid("scp: header too long"));
        }
        line.push(c);
    }
}

/// `C0644 1234 name` → (size, name).
pub fn parse_header(line: &[u8]) -> Result<(u64, String)> {
    let text = String::from_utf8_lossy(line);
    let mut parts = text.splitn(3, ' ');
    let mode = parts.next().unwrap_or("");
    if !mode.starts_with('C') || mode.len() != 5 {
        return Err(AppError::invalid(format!("scp: unexpected header {text}")));
    }
    let size: u64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| AppError::invalid("scp: bad size"))?;
    let name = parts.next().unwrap_or("").to_owned();
    if name.is_empty() || name.contains('/') || name == ".." {
        return Err(AppError::invalid("scp: bad file name"));
    }
    Ok((size, name))
}

/// Receives one file (`scp -f`): writes it to `out`, reports `(done, total)`.
pub async fn download<S, W>(
    s: &mut S,
    out: &mut W,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<u64>
where
    S: AsyncRead + AsyncWrite + Unpin,
    W: AsyncWrite + Unpin,
{
    s.write_all(&[0]).await.map_err(AppError::from)?;
    let size = loop {
        let first = s.read_u8().await.map_err(AppError::from)?;
        match first {
            b'C' => {
                let mut line = vec![b'C'];
                line.extend(read_line(s).await?);
                let (size, _name) = parse_header(&line)?;
                break size;
            }
            // Times (`scp -p`): acknowledge and wait for the file header.
            b'T' => {
                read_line(s).await?;
                s.write_all(&[0]).await.map_err(AppError::from)?;
            }
            1 | 2 => return Err(remote_error(&read_line(s).await?)),
            b'D' => return Err(AppError::invalid("scp: that is a directory")),
            other => {
                return Err(AppError::with(
                    ErrorCode::Io,
                    format!("scp: unexpected reply byte {other}"),
                ));
            }
        }
    };
    if size > MAX_FILE {
        return Err(AppError::invalid("scp: file too large"));
    }
    s.write_all(&[0]).await.map_err(AppError::from)?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    progress(0, size);
    while done < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::with(ErrorCode::Io, "cancelled"));
        }
        let want = usize::try_from((size - done).min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = s.read(&mut buf[..want]).await.map_err(AppError::from)?;
        if n == 0 {
            return Err(AppError::with(
                ErrorCode::ConnectionClosed,
                "scp: connection closed mid-file",
            ));
        }
        out.write_all(&buf[..n]).await.map_err(AppError::from)?;
        done += n as u64;
        progress(done, size);
    }
    out.flush().await.map_err(AppError::from)?;
    read_ack(s).await?;
    s.write_all(&[0]).await.map_err(AppError::from)?;
    Ok(size)
}

/// Sends one file (`scp -t`) of `size` bytes from `input` named `name`.
pub async fn upload<S, R>(
    s: &mut S,
    input: &mut R,
    size: u64,
    name: &str,
    cancel: &AtomicBool,
    progress: &mut (dyn FnMut(u64, u64) + Send),
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    if name.is_empty() || name.contains(['/', '\n', '\r']) {
        return Err(AppError::invalid("scp: bad file name"));
    }
    read_ack(s).await?;
    s.write_all(format!("C0644 {size} {name}\n").as_bytes())
        .await
        .map_err(AppError::from)?;
    read_ack(s).await?;
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    progress(0, size);
    while done < size {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::with(ErrorCode::Io, "cancelled"));
        }
        let n = input.read(&mut buf).await.map_err(AppError::from)?;
        if n == 0 {
            return Err(AppError::with(
                ErrorCode::Io,
                "local file shrank while sending",
            ));
        }
        let n = n.min(usize::try_from(size - done).unwrap_or(n));
        s.write_all(&buf[..n]).await.map_err(AppError::from)?;
        done += n as u64;
        progress(done, size);
    }
    s.write_all(&[0]).await.map_err(AppError::from)?;
    s.flush().await.map_err(AppError::from)?;
    read_ack(s).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_headers() {
        assert_eq!(
            command('f', "flash:startup-config"),
            "scp -f flash:startup-config"
        );
        assert_eq!(command('t', "/tmp/a b"), "scp -t '/tmp/a b'");
        assert_eq!(command('f', "it's"), "scp -f 'it'\\''s'");
        assert_eq!(
            parse_header(b"C0644 12 run.cfg").unwrap(),
            (12, "run.cfg".into())
        );
        assert!(parse_header(b"C0644 12 ../etc/passwd").is_err());
        assert!(parse_header(b"D0755 0 dir").is_err());
        assert!(parse_header(b"C0644 x name").is_err());
    }

    #[tokio::test]
    async fn download_from_a_scripted_server() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let srv = tokio::spawn(async move {
            assert_eq!(server.read_u8().await.unwrap(), 0);
            server.write_all(b"C0644 11 run.cfg\n").await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), 0);
            server.write_all(b"hostname sw").await.unwrap();
            server.write_all(&[0]).await.unwrap();
            assert_eq!(server.read_u8().await.unwrap(), 0);
        });
        let mut out = Vec::new();
        let mut seen = Vec::new();
        let n = download(
            &mut client,
            &mut out,
            &AtomicBool::new(false),
            &mut |d, t| seen.push((d, t)),
        )
        .await
        .unwrap();
        srv.await.unwrap();
        assert_eq!(n, 11);
        assert_eq!(out, b"hostname sw");
        assert_eq!(seen.last(), Some(&(11, 11)));
    }

    #[tokio::test]
    async fn upload_and_remote_errors() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let srv = tokio::spawn(async move {
            server.write_all(&[0]).await.unwrap();
            let mut header = Vec::new();
            loop {
                let c = server.read_u8().await.unwrap();
                if c == b'\n' {
                    break;
                }
                header.push(c);
            }
            assert_eq!(header, b"C0644 5 a.bin");
            server.write_all(&[0]).await.unwrap();
            let mut data = [0u8; 6];
            server.read_exact(&mut data).await.unwrap();
            assert_eq!(&data, b"hello\0");
            server.write_all(&[0]).await.unwrap();
        });
        let mut input: &[u8] = b"hello";
        upload(
            &mut client,
            &mut input,
            5,
            "a.bin",
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .await
        .unwrap();
        srv.await.unwrap();

        // The device refuses: the message comes back as the error.
        let (mut client, mut server) = tokio::io::duplex(64);
        tokio::spawn(async move {
            let _ = server.read_u8().await;
            server
                .write_all(b"\x01scp: flash:x: No such file\n")
                .await
                .unwrap();
        });
        let err = download(
            &mut client,
            &mut Vec::new(),
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("No such file"), "{err}");
    }
}
