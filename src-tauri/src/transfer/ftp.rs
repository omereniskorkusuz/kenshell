//! Embedded FTP server (P5-T03): enough of RFC 959 (+ EPSV/EPRT, SIZE) for
//! `copy ftp://user:pass@host/file flash:` and `copy running-config ftp://…`.
//! One root folder, one login (random password per start), passive and active data
//! connections — active only back to the control connection's own address (no FTP
//! bounce) and passive only accepted from it. Binary transfers; no delete, rename or
//! mkdir.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

use super::served::{
    Config, Credentials, Event, Events, Ids, confined, join, normalize, part_path, upload_refusal,
};

const MAX_LINE: usize = 1024;
const DATA_TIMEOUT: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
const AUTH_ATTEMPTS: u32 = 3;
const CHUNK: usize = 64 * 1024;

/// Accepts control connections until the task is aborted.
pub async fn serve(
    listener: TcpListener,
    cfg: Arc<Config>,
    creds: Arc<Credentials>,
    events: Events,
) {
    let ids = Arc::new(Ids::default());
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        let (cfg, creds, events, ids) = (
            Arc::clone(&cfg),
            Arc::clone(&creds),
            Arc::clone(&events),
            Arc::clone(&ids),
        );
        tokio::spawn(async move {
            let _ = Session {
                cfg,
                creds,
                events,
                ids,
                peer,
                cwd: "/".into(),
                user: None,
                authed: false,
                failures: 0,
                passive: None,
                active: None,
            }
            .run(stream)
            .await;
        });
    }
}

struct Session {
    cfg: Arc<Config>,
    creds: Arc<Credentials>,
    events: Events,
    ids: Arc<Ids>,
    peer: SocketAddr,
    cwd: String,
    user: Option<String>,
    authed: bool,
    failures: u32,
    passive: Option<TcpListener>,
    active: Option<SocketAddr>,
}

/// Reads one CRLF line (bounded). None at EOF or on an over-long line.
async fn read_line<R: AsyncRead + Unpin>(r: &mut BufReader<R>) -> Option<String> {
    let mut buf = Vec::new();
    let n = (&mut *r)
        .take(MAX_LINE as u64)
        .read_until(b'\n', &mut buf)
        .await
        .ok()?;
    if n == 0 || (n == MAX_LINE && !buf.ends_with(b"\n")) {
        return None;
    }
    Some(
        String::from_utf8_lossy(&buf)
            .trim_end_matches(['\r', '\n'])
            .to_owned(),
    )
}

/// `h1,h2,h3,h4,p1,p2` → address.
fn parse_port(arg: &str) -> Option<SocketAddr> {
    let n: Vec<u8> = arg
        .split(',')
        .map(|x| x.trim().parse().ok())
        .collect::<Option<_>>()?;
    if n.len() != 6 {
        return None;
    }
    let ip = IpAddr::from([n[0], n[1], n[2], n[3]]);
    Some(SocketAddr::new(
        ip,
        (u16::from(n[4]) << 8) | u16::from(n[5]),
    ))
}

/// `|1|ip|port|` / `|2|ip|port|` → address.
fn parse_eprt(arg: &str) -> Option<SocketAddr> {
    let d = arg.chars().next()?;
    let f: Vec<&str> = arg.split(d).collect();
    if f.len() != 5 {
        return None;
    }
    Some(SocketAddr::new(f[2].parse().ok()?, f[3].parse().ok()?))
}

/// `Mon DD  YYYY` for a LIST line (UTC; devices only show it).
fn ls_date(secs: u64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // Days since 1970-01-01 → civil date (H. Hinnant's algorithm).
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{} {day:>2}  {year}", MONTHS[(month - 1) as usize])
}

impl Session {
    async fn run(mut self, stream: TcpStream) -> std::io::Result<()> {
        let (r, mut w) = stream.into_split();
        let local = w.local_addr()?;
        let mut r = BufReader::new(r);
        reply(&mut w, "220 KenShell FTP ready").await?;
        loop {
            let Ok(line) = tokio::time::timeout(IDLE_TIMEOUT, read_line(&mut r)).await else {
                let _ = reply(&mut w, "421 Idle timeout").await;
                return Ok(());
            };
            let Some(line) = line else {
                return Ok(());
            };
            let (cmd, arg) = line.split_once(' ').unwrap_or((&line, ""));
            let cmd = cmd.to_ascii_uppercase();
            let arg = arg.trim();
            if !self.authed
                && !matches!(
                    cmd.as_str(),
                    "USER" | "PASS" | "QUIT" | "FEAT" | "SYST" | "OPTS" | "NOOP" | "AUTH"
                )
            {
                reply(&mut w, "530 Please log in with USER and PASS").await?;
                continue;
            }
            match cmd.as_str() {
                "USER" => {
                    self.user = Some(arg.to_owned());
                    self.authed = false;
                    reply(&mut w, "331 Password required").await?;
                }
                "PASS" => {
                    let user = self.user.clone().unwrap_or_default();
                    if self.creds.check(&user, arg) {
                        self.authed = true;
                        reply(&mut w, "230 Logged in").await?;
                    } else {
                        self.failures += 1;
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        reply(&mut w, "530 Login incorrect").await?;
                        if self.failures >= AUTH_ATTEMPTS {
                            return Ok(());
                        }
                    }
                }
                "AUTH" => reply(&mut w, "502 TLS is not offered").await?,
                "QUIT" => {
                    reply(&mut w, "221 Bye").await?;
                    return Ok(());
                }
                "SYST" => reply(&mut w, "215 UNIX Type: L8").await?,
                "FEAT" => {
                    reply(
                        &mut w,
                        "211-Features:\r\n SIZE\r\n EPSV\r\n EPRT\r\n PASV\r\n UTF8\r\n211 End",
                    )
                    .await?;
                }
                "OPTS" | "NOOP" | "MODE" | "STRU" | "ALLO" => reply(&mut w, "200 OK").await?,
                "TYPE" => reply(&mut w, "200 Type set").await?,
                "PWD" | "XPWD" => {
                    let msg = format!("257 \"{}\" is the current directory", self.cwd);
                    reply(&mut w, &msg).await?;
                }
                "CWD" | "XCWD" | "CDUP" | "XCUP" => {
                    let target = if cmd.ends_with("UP") { ".." } else { arg };
                    match normalize(&self.cwd, target) {
                        Some(v) if self.is_dir(&v) => {
                            self.cwd = v;
                            reply(&mut w, "250 Directory changed").await?;
                        }
                        _ => reply(&mut w, "550 No such directory").await?,
                    }
                }
                "PASV" | "EPSV" => {
                    let ip = local.ip();
                    match TcpListener::bind((ip, 0)).await {
                        Ok(l) => {
                            let port = l.local_addr()?.port();
                            self.passive = Some(l);
                            self.active = None;
                            let msg = match (cmd.as_str(), ip) {
                                ("PASV", IpAddr::V4(v4)) => {
                                    let o = v4.octets();
                                    format!(
                                        "227 Entering Passive Mode ({},{},{},{},{},{})",
                                        o[0],
                                        o[1],
                                        o[2],
                                        o[3],
                                        port >> 8,
                                        port & 0xff
                                    )
                                }
                                ("PASV", _) => {
                                    self.passive = None;
                                    "522 Use EPSV for IPv6".to_owned()
                                }
                                _ => format!("229 Entering Extended Passive Mode (|||{port}|)"),
                            };
                            reply(&mut w, &msg).await?;
                        }
                        Err(_) => reply(&mut w, "425 Cannot open a data port").await?,
                    }
                }
                "PORT" | "EPRT" => {
                    let addr = if cmd == "PORT" {
                        parse_port(arg)
                    } else {
                        parse_eprt(arg)
                    };
                    match addr {
                        // Only back to the client itself: no FTP bounce through us.
                        Some(a) if a.ip() == self.peer.ip() && a.port() >= 1024 => {
                            self.active = Some(a);
                            self.passive = None;
                            reply(&mut w, "200 PORT command successful").await?;
                        }
                        _ => reply(&mut w, "501 Address refused").await?,
                    }
                }
                "SIZE" => match self.file(arg) {
                    Some(p) => match tokio::fs::metadata(&p).await {
                        Ok(m) if m.is_file() => reply(&mut w, &format!("213 {}", m.len())).await?,
                        _ => reply(&mut w, "550 No such file").await?,
                    },
                    None => reply(&mut w, "550 No such file").await?,
                },
                "LIST" | "NLST" => self.list(&mut w, arg, cmd == "NLST").await?,
                "RETR" => self.retr(&mut w, arg).await?,
                "STOR" => self.stor(&mut w, arg).await?,
                "ABOR" => reply(&mut w, "226 Nothing to abort").await?,
                _ => reply(&mut w, "502 Command not implemented").await?,
            }
        }
    }

    fn is_dir(&self, virt: &str) -> bool {
        let p = join(&self.cfg.root, virt);
        p.is_dir() && confined(&self.cfg.root, &p)
    }

    /// A file path under the root (never the root itself).
    fn file(&self, arg: &str) -> Option<std::path::PathBuf> {
        let v = normalize(&self.cwd, arg).filter(|v| v != "/")?;
        let p = join(&self.cfg.root, &v);
        confined(&self.cfg.root, &p).then_some(p)
    }

    /// Opens the data connection prepared by PASV/EPSV or PORT/EPRT.
    async fn data(&mut self) -> Option<TcpStream> {
        if let Some(l) = self.passive.take() {
            let (s, from) = tokio::time::timeout(DATA_TIMEOUT, l.accept())
                .await
                .ok()?
                .ok()?;
            // Only the client that asked may connect to the data port.
            return (from.ip() == self.peer.ip()).then_some(s);
        }
        let a = self.active.take()?;
        tokio::time::timeout(DATA_TIMEOUT, TcpStream::connect(a))
            .await
            .ok()?
            .ok()
    }

    async fn list<W: AsyncWrite + Unpin>(
        &mut self,
        w: &mut W,
        arg: &str,
        names_only: bool,
    ) -> std::io::Result<()> {
        // Clients often send `LIST -la`; options are ignored.
        let arg = if arg.starts_with('-') { "" } else { arg };
        let Some(virt) = normalize(&self.cwd, arg).filter(|v| self.is_dir(v)) else {
            return reply(w, "550 No such directory").await;
        };
        let mut lines = String::new();
        if let Ok(mut rd) = tokio::fs::read_dir(join(&self.cfg.root, &virt)).await {
            let mut entries = Vec::new();
            while let Ok(Some(e)) = rd.next_entry().await {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".part") {
                    continue;
                }
                if let Ok(m) = e.metadata().await {
                    entries.push((name, m));
                }
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, m) in entries {
                if names_only {
                    lines.push_str(&format!("{name}\r\n"));
                    continue;
                }
                let secs = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                lines.push_str(&format!(
                    "{} 1 kenshell kenshell {:>12} {} {name}\r\n",
                    if m.is_dir() {
                        "drwxr-xr-x"
                    } else {
                        "-rw-r--r--"
                    },
                    m.len(),
                    ls_date(secs),
                ));
            }
        }
        reply(w, "150 Here comes the listing").await?;
        let Some(mut d) = self.data().await else {
            return reply(w, "425 No data connection").await;
        };
        let ok = d.write_all(lines.as_bytes()).await.is_ok() && d.shutdown().await.is_ok();
        reply(
            w,
            if ok {
                "226 Listing sent"
            } else {
                "426 Listing aborted"
            },
        )
        .await
    }

    async fn retr<W: AsyncWrite + Unpin>(&mut self, w: &mut W, arg: &str) -> std::io::Result<()> {
        let Some(path) = self.file(arg) else {
            return reply(w, "550 No such file").await;
        };
        let mut file = match tokio::fs::File::open(&path).await {
            Ok(f) if f.metadata().await.is_ok_and(|m| m.is_file()) => f,
            _ => return reply(w, "550 No such file").await,
        };
        let size = file.metadata().await.ok().map(|m| m.len());
        reply(w, "150 Opening BINARY mode data connection").await?;
        let Some(mut d) = self.data().await else {
            return reply(w, "425 No data connection").await;
        };
        let id = self.ids.next();
        (self.events)(Event::Started {
            id,
            peer: self.peer.to_string(),
            file: arg.trim_start_matches('/').to_owned(),
            upload: false,
            size,
        });
        let result = copy(&mut file, &mut d, id, &self.events).await;
        let _ = d.shutdown().await;
        self.finish(w, id, result).await
    }

    async fn stor<W: AsyncWrite + Unpin>(&mut self, w: &mut W, arg: &str) -> std::io::Result<()> {
        let Some(path) = normalize(&self.cwd, arg)
            .filter(|v| v != "/")
            .map(|v| join(&self.cfg.root, &v))
        else {
            return reply(w, "553 File name not allowed").await;
        };
        if let Some(why) = upload_refusal(&self.cfg, &path).await {
            return reply(w, &format!("553 Refused: {why}")).await;
        }
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let part = part_path(&path);
        let Ok(mut file) = tokio::fs::File::create(&part).await else {
            return reply(w, "553 Cannot create the file").await;
        };
        reply(w, "150 Ok to send data").await?;
        let Some(mut d) = self.data().await else {
            let _ = tokio::fs::remove_file(&part).await;
            return reply(w, "425 No data connection").await;
        };
        let id = self.ids.next();
        (self.events)(Event::Started {
            id,
            peer: self.peer.to_string(),
            file: arg.trim_start_matches('/').to_owned(),
            upload: true,
            size: None,
        });
        let mut result = copy(&mut d, &mut file, id, &self.events).await;
        if let Ok(n) = result {
            result = file.flush().await.map(|()| n);
        }
        drop(file);
        match &result {
            Ok(_) => {
                if let Err(e) = tokio::fs::rename(&part, &path).await {
                    result = Err(e);
                }
            }
            Err(_) => {
                let _ = tokio::fs::remove_file(&part).await;
            }
        }
        self.finish(w, id, result).await
    }

    async fn finish<W: AsyncWrite + Unpin>(
        &self,
        w: &mut W,
        id: u64,
        result: std::io::Result<u64>,
    ) -> std::io::Result<()> {
        match result {
            Ok(bytes) => {
                (self.events)(Event::Done { id, bytes });
                reply(w, "226 Transfer complete").await
            }
            Err(e) => {
                (self.events)(Event::Failed {
                    id,
                    error: e.to_string(),
                });
                reply(w, "426 Transfer aborted").await
            }
        }
    }
}

async fn reply<W: AsyncWrite + Unpin>(w: &mut W, msg: &str) -> std::io::Result<()> {
    w.write_all(msg.as_bytes()).await?;
    w.write_all(b"\r\n").await?;
    w.flush().await
}

/// Copies with progress events (at most every 1 MB).
async fn copy<R, W>(r: &mut R, w: &mut W, id: u64, events: &Events) -> std::io::Result<u64>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    let mut reported = 0u64;
    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            return Ok(total);
        }
        w.write_all(&buf[..n]).await?;
        total += n as u64;
        if total - reported >= 1 << 20 {
            reported = total;
            events(Event::Progress { id, bytes: total });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn parsing() {
        assert_eq!(
            parse_port("10,0,0,5,4,1"),
            Some("10.0.0.5:1025".parse().unwrap())
        );
        assert_eq!(parse_port("10,0,0,5,4"), None);
        assert_eq!(parse_port("10,0,0,300,4,1"), None);
        assert_eq!(
            parse_eprt("|1|10.0.0.5|2000|"),
            Some("10.0.0.5:2000".parse().unwrap())
        );
        assert_eq!(
            parse_eprt("|2|::1|2000|"),
            Some("[::1]:2000".parse().unwrap())
        );
        assert_eq!(ls_date(0), "Jan  1  1970");
        assert_eq!(ls_date(1_793_923_200), "Nov  6  2026");
    }

    struct Client {
        r: BufReader<tokio::net::tcp::OwnedReadHalf>,
        w: tokio::net::tcp::OwnedWriteHalf,
    }

    impl Client {
        async fn connect(addr: SocketAddr) -> Self {
            let (r, w) = TcpStream::connect(addr).await.unwrap().into_split();
            let mut c = Self {
                r: BufReader::new(r),
                w,
            };
            assert!(c.line().await.starts_with("220"));
            c
        }
        async fn line(&mut self) -> String {
            let mut last = read_line(&mut self.r).await.unwrap();
            // Multi-line replies end with "NNN " (space).
            while last.len() < 4
                || last.as_bytes()[3] == b'-'
                || !last.as_bytes()[0].is_ascii_digit()
            {
                last = read_line(&mut self.r).await.unwrap();
            }
            last
        }
        async fn cmd(&mut self, c: &str) -> String {
            self.w
                .write_all(format!("{c}\r\n").as_bytes())
                .await
                .unwrap();
            self.line().await
        }
        async fn pasv(&mut self) -> SocketAddr {
            let r = self.cmd("PASV").await;
            assert!(r.starts_with("227"), "{r}");
            let inner = &r[r.find('(').unwrap() + 1..r.find(')').unwrap()];
            parse_port(inner).unwrap()
        }
        /// RETR / LIST through passive mode: the reply and the data.
        async fn fetch(&mut self, c: &str) -> (String, Vec<u8>) {
            let a = self.pasv().await;
            let mut d = TcpStream::connect(a).await.unwrap();
            let first = self.cmd(c).await;
            if !first.starts_with("150") {
                return (first, Vec::new());
            }
            let mut data = Vec::new();
            d.read_to_end(&mut data).await.unwrap();
            (self.line().await, data)
        }
        async fn put(&mut self, name: &str, data: &[u8]) -> String {
            let a = self.pasv().await;
            let mut d = TcpStream::connect(a).await.unwrap();
            let first = self.cmd(&format!("STOR {name}")).await;
            if !first.starts_with("150") {
                return first;
            }
            d.write_all(data).await.unwrap();
            d.shutdown().await.unwrap();
            drop(d);
            self.line().await
        }
    }

    async fn server(
        cfg: Config,
    ) -> (
        SocketAddr,
        Arc<Mutex<Vec<Event>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        let events: Events = Arc::new(move |e| sink.lock().unwrap().push(e));
        let creds = Arc::new(Credentials {
            user: "kenshell".into(),
            password: "pw".into(),
        });
        (
            addr,
            log,
            tokio::spawn(serve(l, Arc::new(cfg), creds, events)),
        )
    }

    #[tokio::test]
    async fn login_list_download_upload_and_refusals() {
        let dir = tempfile::tempdir().unwrap();
        let image: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::create_dir(dir.path().join("images")).unwrap();
        std::fs::write(dir.path().join("images/ios.bin"), &image).unwrap();
        std::fs::write(dir.path().join("sw1-confg"), b"hostname sw1\n").unwrap();
        let (addr, events, task) = server(Config {
            root: dir.path().into(),
            allow_upload: true,
            overwrite: false,
        })
        .await;

        let mut c = Client::connect(addr).await;
        assert!(c.cmd("LIST").await.starts_with("530"));
        assert!(c.cmd("USER kenshell").await.starts_with("331"));
        assert!(c.cmd("PASS nope").await.starts_with("530"));
        assert!(c.cmd("USER kenshell").await.starts_with("331"));
        assert!(c.cmd("PASS pw").await.starts_with("230"));
        assert!(c.cmd("TYPE I").await.starts_with("200"));
        assert_eq!(c.cmd("SIZE images/ios.bin").await, "213 300000");

        let (r, list) = c.fetch("LIST").await;
        assert!(r.starts_with("226"), "{r}");
        let list = String::from_utf8(list).unwrap();
        assert!(
            list.contains("sw1-confg") && list.starts_with("drwx"),
            "{list}"
        );

        let (r, got) = c.fetch("RETR /images/ios.bin").await;
        assert!(r.starts_with("226"), "{r}");
        assert_eq!(got, image);

        assert!(c.cmd("CWD images").await.starts_with("250"));
        assert_eq!(
            c.cmd("PWD").await,
            "257 \"/images\" is the current directory"
        );
        let (_, names) = c.fetch("NLST").await;
        assert_eq!(names, b"ios.bin\r\n");
        assert!(c.cmd("CWD ../..").await.starts_with("550"));
        assert!(c.cmd("CDUP").await.starts_with("250"));
        let (r, _) = c.fetch("RETR ../../etc/passwd").await;
        assert!(r.starts_with("550"), "{r}");

        // Upload, then the same name again (overwrite off).
        let cfg = b"hostname sw2\n!\nend\n";
        assert!(c.put("backups/sw2-confg", cfg).await.starts_with("226"));
        assert_eq!(
            std::fs::read(dir.path().join("backups/sw2-confg")).unwrap(),
            cfg
        );
        assert!(!dir.path().join("backups/sw2-confg.part").exists());
        let r = c.put("backups/sw2-confg", b"x").await;
        assert!(r.starts_with("553") && r.contains("overwrite"), "{r}");

        // Active mode only back to the client itself.
        assert!(c.cmd("PORT 8,8,8,8,4,1").await.starts_with("501"));
        assert!(c.cmd("DELE sw1-confg").await.starts_with("502"));
        assert!(c.cmd("QUIT").await.starts_with("221"));

        let ev = events.lock().unwrap().clone();
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Done { bytes: 300_000, .. }))
        );
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Started { upload: true, .. }))
        );

        // Three wrong passwords close the connection.
        let mut c = Client::connect(addr).await;
        for _ in 0..3 {
            c.cmd("USER kenshell").await;
            c.w.write_all(b"PASS bad\r\n").await.unwrap();
            let _ = read_line(&mut c.r).await;
        }
        assert!(read_line(&mut c.r).await.is_none());
        task.abort();
    }

    #[tokio::test]
    async fn active_mode_and_uploads_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), b"data").unwrap();
        let (addr, _, task) = server(Config {
            root: dir.path().into(),
            allow_upload: false,
            overwrite: false,
        })
        .await;
        let mut c = Client::connect(addr).await;
        c.cmd("USER kenshell").await;
        c.cmd("PASS pw").await;
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        let r = c.cmd(&format!("EPRT |1|127.0.0.1|{p}|")).await;
        assert!(r.starts_with("200"), "{r}");
        assert!(c.cmd("RETR f.txt").await.starts_with("150"));
        let (mut d, _) = l.accept().await.unwrap();
        let mut got = Vec::new();
        d.read_to_end(&mut got).await.unwrap();
        assert_eq!(got, b"data");
        assert!(c.line().await.starts_with("226"));
        let r = c.cmd("STOR x.txt").await;
        assert!(r.starts_with("553") && r.contains("uploads are off"), "{r}");
        task.abort();
    }
}
