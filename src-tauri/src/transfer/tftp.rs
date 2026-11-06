//! Embedded TFTP server (P5-T02): RFC 1350 with the blksize (RFC 2348) and tsize
//! (RFC 2349) options, so `copy tftp: flash:` / `copy running-config tftp:` work without a
//! separate program. Files are served from one root folder only (no `..`, no absolute
//! paths); uploads are optional and never overwrite unless allowed. Runs only while on.

use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;

const RRQ: u16 = 1;
const WRQ: u16 = 2;
const DATA: u16 = 3;
const ACK: u16 = 4;
const ERROR: u16 = 5;
const OACK: u16 = 6;
const DEFAULT_BLOCK: usize = 512;
const MAX_BLOCK: usize = 65_464;
const RETRIES: u32 = 5;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub write: bool,
    pub filename: String,
    pub blksize: Option<usize>,
    pub tsize: Option<u64>,
}

/// Parses an RRQ / WRQ (`opcode | filename\0 | mode\0 | [option\0 value\0]*`).
pub fn parse_request(p: &[u8]) -> Option<Request> {
    let op = u16::from_be_bytes([*p.first()?, *p.get(1)?]);
    if op != RRQ && op != WRQ {
        return None;
    }
    let mut fields = p[2..]
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    let filename = fields.next()?;
    let mode = fields.next()?.to_ascii_lowercase();
    if filename.is_empty() || (mode != "octet" && mode != "netascii") {
        return None;
    }
    let mut req = Request {
        write: op == WRQ,
        filename,
        blksize: None,
        tsize: None,
    };
    while let (Some(k), Some(v)) = (fields.next(), fields.next()) {
        match k.to_ascii_lowercase().as_str() {
            "blksize" => req.blksize = v.parse().ok().filter(|n| (8..=MAX_BLOCK).contains(n)),
            "tsize" => req.tsize = v.parse().ok(),
            _ => {}
        }
    }
    Some(req)
}

fn error_packet(code: u16, msg: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(5 + msg.len());
    p.extend_from_slice(&ERROR.to_be_bytes());
    p.extend_from_slice(&code.to_be_bytes());
    p.extend_from_slice(msg.as_bytes());
    p.push(0);
    p
}

fn oack(options: &[(&str, String)]) -> Vec<u8> {
    let mut p = OACK.to_be_bytes().to_vec();
    for (k, v) in options {
        p.extend_from_slice(k.as_bytes());
        p.push(0);
        p.extend_from_slice(v.as_bytes());
        p.push(0);
    }
    p
}

/// `root` + a client file name, or None when it would leave the root.
pub fn resolve(root: &Path, name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    let rel = Path::new(name.trim_start_matches('/'));
    let mut out = root.to_path_buf();
    let mut depth = 0;
    for c in rel.components() {
        match c {
            Component::Normal(p) => {
                out.push(p);
                depth += 1;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (depth > 0).then_some(out)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Event {
    Started {
        id: u64,
        peer: String,
        file: String,
        upload: bool,
        size: Option<u64>,
    },
    Progress {
        id: u64,
        bytes: u64,
    },
    Done {
        id: u64,
        bytes: u64,
    },
    Failed {
        id: u64,
        error: String,
    },
}

pub struct Config {
    pub root: PathBuf,
    pub allow_upload: bool,
    pub overwrite: bool,
}

/// Serves requests on `socket` until the task is aborted.
pub async fn serve(socket: UdpSocket, cfg: Arc<Config>, events: Arc<dyn Fn(Event) + Send + Sync>) {
    let mut buf = vec![0u8; 1024];
    let mut next_id = 1u64;
    loop {
        let Ok((n, peer)) = socket.recv_from(&mut buf).await else {
            continue;
        };
        let Some(req) = parse_request(&buf[..n]) else {
            continue;
        };
        let id = next_id;
        next_id += 1;
        let (cfg, events) = (Arc::clone(&cfg), Arc::clone(&events));
        let local = socket.local_addr().ok();
        tokio::spawn(async move {
            // Each transfer talks from its own port (its TID), as RFC 1350 says.
            let bind = local.map_or_else(
                || "0.0.0.0:0".parse().expect("addr"),
                |a| SocketAddr::new(a.ip(), 0),
            );
            let Ok(sock) = UdpSocket::bind(bind).await else {
                return;
            };
            if sock.connect(peer).await.is_err() {
                return;
            }
            let result = if req.write {
                receive(&sock, &cfg, &req, id, peer, &*events).await
            } else {
                send(&sock, &cfg, &req, id, peer, &*events).await
            };
            match result {
                Ok(bytes) => events(Event::Done { id, bytes }),
                Err(e) => events(Event::Failed { id, error: e }),
            }
        });
    }
}

async fn exchange(
    sock: &UdpSocket,
    packet: &[u8],
    want: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; MAX_BLOCK + 4];
    for _ in 0..RETRIES {
        sock.send(packet).await.map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        loop {
            match tokio::time::timeout_at(deadline, sock.recv(&mut buf)).await {
                Err(_) => break,
                Ok(Err(e)) => return Err(e.to_string()),
                Ok(Ok(n)) => {
                    let p = &buf[..n];
                    if p.len() >= 4 && u16::from_be_bytes([p[0], p[1]]) == ERROR {
                        return Err(format!(
                            "client: {}",
                            String::from_utf8_lossy(&p[4..]).trim_end_matches('\0')
                        ));
                    }
                    if want(p) {
                        return Ok(p.to_vec());
                    }
                }
            }
        }
    }
    Err("timed out".into())
}

fn is_ack(p: &[u8], block: u16) -> bool {
    p.len() >= 4
        && u16::from_be_bytes([p[0], p[1]]) == ACK
        && u16::from_be_bytes([p[2], p[3]]) == block
}

async fn send(
    sock: &UdpSocket,
    cfg: &Config,
    req: &Request,
    id: u64,
    peer: SocketAddr,
    events: &(dyn Fn(Event) + Send + Sync),
) -> Result<u64, String> {
    let Some(path) = resolve(&cfg.root, &req.filename) else {
        let _ = sock.send(&error_packet(2, "access violation")).await;
        return Err("path outside the TFTP folder".into());
    };
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => {
            let _ = sock.send(&error_packet(1, "file not found")).await;
            return Err(format!("not found: {}", req.filename));
        }
    };
    let size = file.metadata().await.map(|m| m.len()).ok();
    events(Event::Started {
        id,
        peer: peer.to_string(),
        file: req.filename.clone(),
        upload: false,
        size,
    });
    let block_size = req.blksize.unwrap_or(DEFAULT_BLOCK);
    let mut opts = Vec::new();
    if let Some(b) = req.blksize {
        opts.push(("blksize", b.to_string()));
    }
    if req.tsize.is_some()
        && let Some(s) = size
    {
        opts.push(("tsize", s.to_string()));
    }
    if !opts.is_empty() {
        exchange(sock, &oack(&opts), |p| is_ack(p, 0)).await?;
    }
    let mut block: u16 = 1;
    let mut total = 0u64;
    let mut data = vec![0u8; block_size];
    loop {
        let mut n = 0;
        while n < block_size {
            let r = file.read(&mut data[n..]).await.map_err(|e| e.to_string())?;
            if r == 0 {
                break;
            }
            n += r;
        }
        let mut packet = DATA.to_be_bytes().to_vec();
        packet.extend_from_slice(&block.to_be_bytes());
        packet.extend_from_slice(&data[..n]);
        exchange(sock, &packet, |p| is_ack(p, block)).await?;
        total += n as u64;
        if block.is_multiple_of(64) {
            events(Event::Progress { id, bytes: total });
        }
        if n < block_size {
            return Ok(total);
        }
        block = block.wrapping_add(1);
    }
}

async fn receive(
    sock: &UdpSocket,
    cfg: &Config,
    req: &Request,
    id: u64,
    peer: SocketAddr,
    events: &(dyn Fn(Event) + Send + Sync),
) -> Result<u64, String> {
    if !cfg.allow_upload {
        let _ = sock.send(&error_packet(2, "uploads are off")).await;
        return Err("upload refused (uploads are off)".into());
    }
    let Some(path) = resolve(&cfg.root, &req.filename) else {
        let _ = sock.send(&error_packet(2, "access violation")).await;
        return Err("path outside the TFTP folder".into());
    };
    if !cfg.overwrite && tokio::fs::metadata(&path).await.is_ok() {
        let _ = sock.send(&error_packet(6, "file already exists")).await;
        return Err(format!("{} exists (overwrite is off)", req.filename));
    }
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    // Written to a temporary name and moved into place when complete.
    let part = path.with_extension("part");
    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|e| e.to_string())?;
    events(Event::Started {
        id,
        peer: peer.to_string(),
        file: req.filename.clone(),
        upload: true,
        size: req.tsize,
    });
    let block_size = req.blksize.unwrap_or(DEFAULT_BLOCK);
    let mut opts = Vec::new();
    if let Some(b) = req.blksize {
        opts.push(("blksize", b.to_string()));
    }
    if let Some(t) = req.tsize {
        opts.push(("tsize", t.to_string()));
    }
    let mut reply = if opts.is_empty() {
        [ACK.to_be_bytes(), 0u16.to_be_bytes()].concat()
    } else {
        oack(&opts)
    };
    let mut expect: u16 = 1;
    let mut total = 0u64;
    let result = loop {
        let want = expect;
        let p = match exchange(sock, &reply, |p| {
            p.len() >= 4
                && u16::from_be_bytes([p[0], p[1]]) == DATA
                && u16::from_be_bytes([p[2], p[3]]) == want
        })
        .await
        {
            Ok(p) => p,
            Err(e) => break Err(e),
        };
        let payload = &p[4..];
        if let Err(e) = file.write_all(payload).await {
            let _ = sock
                .send(&error_packet(3, "disk full or write error"))
                .await;
            break Err(e.to_string());
        }
        total += payload.len() as u64;
        reply = [ACK.to_be_bytes(), want.to_be_bytes()].concat();
        if want.is_multiple_of(64) {
            events(Event::Progress { id, bytes: total });
        }
        if payload.len() < block_size {
            let _ = sock.send(&reply).await;
            break Ok(total);
        }
        expect = expect.wrapping_add(1);
    };
    drop(file);
    match result {
        Ok(total) => {
            tokio::fs::rename(&part, &path)
                .await
                .map_err(|e| e.to_string())?;
            Ok(total)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn rrq(op: u16, name: &str, opts: &[(&str, &str)]) -> Vec<u8> {
        let mut p = op.to_be_bytes().to_vec();
        p.extend_from_slice(name.as_bytes());
        p.push(0);
        p.extend_from_slice(b"octet\0");
        for (k, v) in opts {
            p.extend_from_slice(k.as_bytes());
            p.push(0);
            p.extend_from_slice(v.as_bytes());
            p.push(0);
        }
        p
    }

    #[test]
    fn requests_and_paths() {
        let r = parse_request(&rrq(
            RRQ,
            "cat9k.bin",
            &[("blksize", "1428"), ("tsize", "0")],
        ))
        .unwrap();
        assert_eq!(
            (r.write, r.filename.as_str(), r.blksize, r.tsize),
            (false, "cat9k.bin", Some(1428), Some(0))
        );
        assert_eq!(
            parse_request(&rrq(WRQ, "a", &[("blksize", "4")]))
                .unwrap()
                .blksize,
            None,
            "below 8 ignored"
        );
        assert!(parse_request(b"\x00\x03xx").is_none());
        assert!(parse_request(&rrq(RRQ, "", &[])).is_none());
        let root = Path::new("/srv/tftp");
        assert_eq!(
            resolve(root, "sw1-confg").unwrap(),
            Path::new("/srv/tftp/sw1-confg")
        );
        assert_eq!(
            resolve(root, "/images/x.bin").unwrap(),
            Path::new("/srv/tftp/images/x.bin")
        );
        for bad in ["../etc/passwd", "a/../../b", "..\\x", ""] {
            assert!(resolve(root, bad).is_none(), "{bad}");
        }
    }

    async fn server(
        cfg: Config,
    ) -> (
        SocketAddr,
        Arc<Mutex<Vec<Event>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let e = Arc::clone(&events);
        let task = tokio::spawn(serve(
            sock,
            Arc::new(cfg),
            Arc::new(move |ev| e.lock().unwrap().push(ev)),
        ));
        (addr, events, task)
    }

    /// A tiny TFTP client (what a switch does with `copy tftp:`).
    async fn get(
        server: SocketAddr,
        name: &str,
        blksize: Option<usize>,
    ) -> Result<Vec<u8>, String> {
        let c = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let bs = blksize.map(|b| b.to_string());
        let opts: Vec<(&str, &str)> = bs
            .as_deref()
            .map(|b| vec![("blksize", b)])
            .unwrap_or_default();
        c.send_to(&rrq(RRQ, name, &opts), server).await.unwrap();
        let mut out = Vec::new();
        let mut buf = vec![0u8; 70_000];
        let block = blksize.unwrap_or(512);
        loop {
            let (n, from) = tokio::time::timeout(Duration::from_secs(3), c.recv_from(&mut buf))
                .await
                .map_err(|_| "timeout")?
                .unwrap();
            let p = &buf[..n];
            match u16::from_be_bytes([p[0], p[1]]) {
                OACK => c.send_to(&[0, 4, 0, 0], from).await.map(|_| ()).unwrap(),
                DATA => {
                    out.extend_from_slice(&p[4..]);
                    c.send_to(&[0, 4, p[2], p[3]], from).await.unwrap();
                    if n - 4 < block {
                        return Ok(out);
                    }
                }
                ERROR => return Err(String::from_utf8_lossy(&p[4..n - 1]).into_owned()),
                _ => {}
            }
        }
    }

    async fn put(server: SocketAddr, name: &str, data: &[u8]) -> Result<(), String> {
        let c = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        c.send_to(&rrq(WRQ, name, &[]), server).await.unwrap();
        let mut buf = vec![0u8; 1024];
        let mut block: u16 = 0;
        let mut chunks = data.chunks(512).collect::<Vec<_>>();
        if data.len().is_multiple_of(512) {
            chunks.push(&[]);
        }
        loop {
            let (n, from) = tokio::time::timeout(Duration::from_secs(3), c.recv_from(&mut buf))
                .await
                .map_err(|_| "timeout")?
                .unwrap();
            let p = &buf[..n];
            match u16::from_be_bytes([p[0], p[1]]) {
                ACK => {
                    if usize::from(block) == chunks.len() {
                        return Ok(());
                    }
                    block += 1;
                    let mut d = vec![0, 3];
                    d.extend_from_slice(&block.to_be_bytes());
                    d.extend_from_slice(chunks[usize::from(block) - 1]);
                    c.send_to(&d, from).await.unwrap();
                }
                ERROR => return Err(String::from_utf8_lossy(&p[4..n - 1]).into_owned()),
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn download_upload_and_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let image: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        std::fs::write(dir.path().join("fw.bin"), &image).unwrap();
        let (addr, events, task) = server(Config {
            root: dir.path().into(),
            allow_upload: true,
            overwrite: false,
        })
        .await;

        assert_eq!(
            get(addr, "fw.bin", None).await.unwrap(),
            image,
            "512-byte blocks"
        );
        assert_eq!(
            get(addr, "fw.bin", Some(1428)).await.unwrap(),
            image,
            "blksize option"
        );
        assert_eq!(
            get(addr, "../../etc/passwd", None).await.unwrap_err(),
            "access violation"
        );
        assert_eq!(get(addr, "nope", None).await.unwrap_err(), "file not found");

        let cfg = b"hostname sw1\n!\nend\n".repeat(100);
        put(addr, "backups/sw1-confg", &cfg).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            std::fs::read(dir.path().join("backups/sw1-confg")).unwrap(),
            cfg
        );
        assert_eq!(
            put(addr, "backups/sw1-confg", b"x").await.unwrap_err(),
            "file already exists"
        );
        task.abort();

        let ev = events.lock().unwrap().clone();
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Done { bytes, .. } if *bytes == image.len() as u64))
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Started { upload: true, .. }))
        );

        // Uploads off.
        let (addr, _, task) = server(Config {
            root: dir.path().into(),
            allow_upload: false,
            overwrite: true,
        })
        .await;
        assert_eq!(put(addr, "x", b"y").await.unwrap_err(), "uploads are off");
        task.abort();
    }
}
