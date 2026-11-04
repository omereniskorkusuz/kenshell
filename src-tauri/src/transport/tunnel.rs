//! Tunnelling helpers (P2-T08/T09): SOCKS5 / HTTP CONNECT proxy clients for the first
//! hop, a minimal SOCKS5 server for dynamic forwarding, and byte piping.

use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::net::{self, label, map_io};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{ProxyConfig, ProxyKind};

/// Anything a tunnel can run over (TCP stream, SSH channel stream).
pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// TCP connection to `host:port`, directly or through `proxy`.
pub async fn connect_via(host: &str, port: u16, proxy: Option<&ProxyConfig>) -> Result<TcpStream> {
    let Some(p) = proxy else {
        return net::connect(host, port).await;
    };
    let mut s = net::connect(&p.host, p.port).await?;
    let via = label(&p.host, p.port);
    let r = match p.kind {
        ProxyKind::Socks5 => socks5_connect(&mut s, host, port).await,
        ProxyKind::Http => http_connect(&mut s, host, port).await,
    };
    r.map_err(|e| {
        AppError::with(
            e.code,
            format!("proxy {via}: {}", e.detail.unwrap_or_default()),
        )
    })?;
    Ok(s)
}

fn proxy_err(msg: impl Into<String>) -> AppError {
    AppError::with(ErrorCode::ConnectionRefused, msg)
}

/// SOCKS5 CONNECT (no authentication), domain name sent as is (resolved by the proxy).
pub async fn socks5_connect<S: Io>(s: &mut S, host: &str, port: u16) -> Result<()> {
    let io = |e: std::io::Error| map_io("socks5", &e);
    if host.len() > 255 {
        return Err(AppError::invalid("host name too long for SOCKS5"));
    }
    s.write_all(&[5, 1, 0]).await.map_err(io)?;
    let mut hello = [0u8; 2];
    s.read_exact(&mut hello).await.map_err(io)?;
    if hello != [5, 0] {
        return Err(proxy_err(
            "SOCKS5 proxy requires authentication (not supported)",
        ));
    }
    let mut req = vec![5, 1, 0, 3, host.len() as u8];
    req.extend(host.as_bytes());
    req.extend(port.to_be_bytes());
    s.write_all(&req).await.map_err(io)?;
    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(io)?;
    if head[1] != 0 {
        return Err(proxy_err(format!("SOCKS5 refused (code {})", head[1])));
    }
    let skip = match head[3] {
        1 => 4 + 2,
        4 => 16 + 2,
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await.map_err(io)?;
            n[0] as usize + 2
        }
        _ => return Err(proxy_err("bad SOCKS5 reply")),
    };
    let mut rest = vec![0u8; skip];
    s.read_exact(&mut rest).await.map_err(io)?;
    Ok(())
}

/// HTTP `CONNECT host:port` (no authentication).
pub async fn http_connect<S: Io>(s: &mut S, host: &str, port: u16) -> Result<()> {
    let io = |e: std::io::Error| map_io("http proxy", &e);
    let target = label(host, port);
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .map_err(io)?;
    let mut buf = Vec::with_capacity(256);
    let mut b = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if buf.len() > 8192 {
            return Err(proxy_err("HTTP proxy reply too long"));
        }
        s.read_exact(&mut b).await.map_err(io)?;
        buf.push(b[0]);
    }
    let status = String::from_utf8_lossy(&buf);
    let ok = status
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .is_some_and(|c| c.starts_with('2'));
    if ok {
        Ok(())
    } else {
        Err(proxy_err(format!(
            "HTTP proxy: {}",
            status.lines().next().unwrap_or("")
        )))
    }
}

/// Target requested by a SOCKS5 client (dynamic forwarding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocksTarget {
    pub host: String,
    pub port: u16,
}

/// Server side of SOCKS5 (no auth, CONNECT only). Returns the target; the caller then
/// opens the tunnel and calls [`socks5_reply`].
pub async fn socks5_accept<S: Io>(s: &mut S) -> std::io::Result<SocksTarget> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(bad("not SOCKS5"));
    }
    let mut methods = vec![0u8; head[1] as usize];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[5, 0xff]).await?;
        return Err(bad("client needs authentication"));
    }
    s.write_all(&[5, 0]).await?;
    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    if req[0] != 5 || req[1] != 1 {
        socks5_reply(s, false).await.ok();
        return Err(bad("only CONNECT is supported"));
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a).await?;
            Ipv4Addr::from(a).to_string()
        }
        4 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a).await?;
            Ipv6Addr::from(a).to_string()
        }
        3 => {
            let mut n = [0u8; 1];
            s.read_exact(&mut n).await?;
            let mut name = vec![0u8; n[0] as usize];
            s.read_exact(&mut name).await?;
            String::from_utf8(name).map_err(|_| bad("bad host name"))?
        }
        _ => return Err(bad("bad address type")),
    };
    let mut p = [0u8; 2];
    s.read_exact(&mut p).await?;
    Ok(SocksTarget {
        host,
        port: u16::from_be_bytes(p),
    })
}

pub async fn socks5_reply<S: Io>(s: &mut S, ok: bool) -> std::io::Result<()> {
    s.write_all(&[5, if ok { 0 } else { 1 }, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn socks5_client_and_server_agree() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        let srv = tokio::spawn(async move {
            let t = socks5_accept(&mut server).await.unwrap();
            socks5_reply(&mut server, true).await.unwrap();
            t
        });
        socks5_connect(&mut client, "core-sw-01.lab", 22)
            .await
            .unwrap();
        assert_eq!(
            srv.await.unwrap(),
            SocksTarget {
                host: "core-sw-01.lab".into(),
                port: 22
            }
        );
    }

    #[tokio::test]
    async fn socks5_refusal_is_reported() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let _ = socks5_accept(&mut server).await;
            socks5_reply(&mut server, false).await.unwrap();
        });
        let e = socks5_connect(&mut client, "x", 1).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ConnectionRefused);
    }

    #[tokio::test]
    async fn http_connect_status() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            let _ = server.read(&mut buf).await;
            server
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await
                .unwrap();
        });
        http_connect(&mut client, "10.0.0.1", 22).await.unwrap();

        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            let _ = server.read(&mut buf).await;
            server
                .write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n")
                .await
                .unwrap();
        });
        assert!(http_connect(&mut client, "10.0.0.1", 22).await.is_err());
    }

    #[tokio::test]
    async fn socks5_server_rejects_garbage() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        client.write_all(&[4, 1, 0, 80]).await.unwrap();
        assert!(socks5_accept(&mut server).await.is_err());
    }
}
