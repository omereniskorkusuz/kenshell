//! Shared TCP plumbing for network transports: connect with timeout and friendly
//! error mapping (host not found / refused / timeout).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::task::JoinHandle;

use super::{Transport, TransportEvent, TransportSink};
use crate::model::{NetLink, Protocol};

use crate::error::{AppError, ErrorCode, Result};

/// TCP connect timeout (host down / filtered port).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Read buffer per network read.
pub const READ_CHUNK: usize = 64 * 1024;

/// `host:port` label for errors and logs (IPv6 hosts get brackets).
pub fn label(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

pub async fn connect(host: &str, port: u16) -> Result<TcpStream> {
    let target = label(host, port);
    let addrs: Vec<_> = match tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio::net::lookup_host((host.trim_matches(['[', ']']), port)),
    )
    .await
    {
        Err(_) => return Err(AppError::with(ErrorCode::Timeout, format!("{target}: DNS"))),
        Ok(Err(e)) => {
            return Err(AppError::with(
                ErrorCode::HostNotFound,
                format!("{target}: {e}"),
            ));
        }
        Ok(Ok(a)) => a.collect(),
    };
    if addrs.is_empty() {
        return Err(AppError::with(ErrorCode::HostNotFound, target));
    }
    let mut last = AppError::with(ErrorCode::HostNotFound, target.clone());
    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(e)) => last = map_io(&target, &e),
            Err(_) => last = AppError::with(ErrorCode::Timeout, format!("{target}: connect")),
        }
    }
    Err(last)
}

/// Maps a socket error to a friendly code.
pub fn map_io(target: &str, e: &std::io::Error) -> AppError {
    use std::io::ErrorKind as K;
    let code = match e.kind() {
        K::ConnectionRefused => ErrorCode::ConnectionRefused,
        K::TimedOut => ErrorCode::Timeout,
        K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof => {
            ErrorCode::ConnectionClosed
        }
        K::HostUnreachable | K::NetworkUnreachable | K::AddrNotAvailable => ErrorCode::Timeout,
        K::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::Io,
    };
    AppError::with(code, format!("{target}: {e}"))
}

// ---------------------------------------------------------------- generic TCP transport

/// Protocol framing on top of a TCP byte stream (telnet IAC, rlogin handshake, raw).
pub trait Codec: Send + 'static {
    /// Bytes sent right after the TCP connect (negotiation / handshake).
    fn greeting(&mut self) -> Vec<u8> {
        Vec::new()
    }
    /// Wire bytes in → terminal `data` + `reply` bytes to send back to the device.
    fn decode(&mut self, input: &[u8], data: &mut Vec<u8>, reply: &mut Vec<u8>);
    /// Terminal bytes → wire bytes.
    fn encode(&mut self, input: &[u8]) -> Vec<u8>;
    /// Window size change → bytes to send (if the protocol negotiated it).
    fn resize(&mut self, _cols: u16, _rows: u16) -> Option<Vec<u8>> {
        None
    }
    /// BREAK → bytes to send, if the protocol has one.
    fn brk(&self) -> Option<Vec<u8>> {
        None
    }
}

type Writer = Arc<tokio::sync::Mutex<OwnedWriteHalf>>;

/// A TCP connection driven by a [`Codec`]: reader task → sink; writes via a shared half.
pub struct TcpTransport<C: Codec> {
    protocol: Protocol,
    host: String,
    port: u16,
    username: Option<String>,
    codec: Arc<Mutex<C>>,
    sink: TransportSink,
    writer: Option<Writer>,
    reader: Option<JoinHandle<()>>,
    size: (u16, u16),
}

impl<C: Codec> TcpTransport<C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        protocol: Protocol,
        host: String,
        port: u16,
        username: Option<String>,
        codec: C,
        sink: TransportSink,
        cols: u16,
        rows: u16,
    ) -> Self {
        Self {
            protocol,
            host,
            port,
            username,
            codec: Arc::new(Mutex::new(codec)),
            sink,
            writer: None,
            reader: None,
            size: (cols, rows),
        }
    }

    fn codec(&self) -> std::sync::MutexGuard<'_, C> {
        self.codec.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn writer(&self) -> Result<Writer> {
        self.writer
            .clone()
            .ok_or_else(|| AppError::invalid("not connected"))
    }

    async fn send(&self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let target = label(&self.host, self.port);
        let w = self.writer()?;
        let mut w = w.lock().await;
        w.write_all(bytes).await.map_err(|e| map_io(&target, &e))
    }
}

#[async_trait]
impl<C: Codec> Transport for TcpTransport<C> {
    async fn connect(&mut self) -> Result<()> {
        if self.writer.is_some() {
            return Err(AppError::invalid("already connected"));
        }
        let stream = connect(&self.host, self.port).await?;
        let (mut rd, wr) = stream.into_split();
        let writer: Writer = Arc::new(tokio::sync::Mutex::new(wr));
        self.writer = Some(Arc::clone(&writer));
        let greeting = self.codec().greeting();
        self.send(&greeting).await?;
        let (cols, rows) = self.size;
        let naws = self.codec().resize(cols, rows);
        if let Some(b) = naws {
            self.send(&b).await?;
        }

        let codec = Arc::clone(&self.codec);
        let sink = self.sink.clone();
        let target = label(&self.host, self.port);
        self.reader = Some(tokio::spawn(async move {
            let mut buf = vec![0u8; READ_CHUNK];
            let end = loop {
                let n = match rd.read(&mut buf).await {
                    Ok(0) => break None,
                    Ok(n) => n,
                    Err(e) => break Some(map_io(&target, &e)),
                };
                let (mut data, mut reply) = (Vec::new(), Vec::new());
                codec.lock().unwrap_or_else(|p| p.into_inner()).decode(
                    &buf[..n],
                    &mut data,
                    &mut reply,
                );
                if !reply.is_empty() {
                    let mut w = writer.lock().await;
                    if let Err(e) = w.write_all(&reply).await {
                        break Some(map_io(&target, &e));
                    }
                }
                if !data.is_empty() && sink.send(TransportEvent::Data(data)).await.is_err() {
                    return; // session gone
                }
            };
            let _ = sink.send(TransportEvent::Closed(end)).await;
        }));
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let bytes = self.codec().encode(data);
        self.send(&bytes).await
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.size = (cols, rows);
        if self.writer.is_none() {
            return Ok(());
        }
        let msg = self.codec().resize(cols, rows);
        match msg {
            Some(b) => self.send(&b).await,
            None => Ok(()),
        }
    }

    async fn send_break(&mut self) -> Result<()> {
        let msg = self.codec().brk();
        match msg {
            Some(b) => self.send(&b).await,
            None => Err(AppError::with(
                crate::error::ErrorCode::Unsupported,
                "this protocol has no BREAK",
            )),
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(r) = self.reader.take() {
            r.abort();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.lock().await.shutdown().await;
        }
        Ok(())
    }

    fn net_link(&self) -> Option<NetLink> {
        Some(NetLink {
            protocol: self.protocol,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            host_key: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(label("10.0.0.1", 22), "10.0.0.1:22");
        assert_eq!(label("fe80::1", 23), "[fe80::1]:23");
    }

    #[tokio::test]
    async fn refused_and_unknown_host_are_friendly() {
        // Bind then drop to get a port that is (almost certainly) closed.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let e = connect("127.0.0.1", port).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::ConnectionRefused);
        let e = connect("does-not-exist.invalid", 22).await.unwrap_err();
        assert!(matches!(
            e.code,
            ErrorCode::HostNotFound | ErrorCode::Timeout
        ));
    }
}
