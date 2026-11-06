//! Embedded SCP server (P5-T03): a small SSH server that only runs `scp -f` / `scp -t`
//! (no shell, no forwarding, no SFTP), so a device can
//! `copy scp://user@host/file flash:` and `copy running-config scp://…`. Password and
//! keyboard-interactive login with the server's own credentials; host keys are fresh
//! for every start (Ed25519 + RSA for older IOS) and their fingerprint is shown in the
//! UI. Legacy kex/ciphers are offered at the lowest priority, as the client does.

use std::borrow::Cow;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::{HashAlg, PrivateKey};
use russh::server::{self, Auth, Msg, Response, Session as SshSession};
use russh::{Channel, ChannelId, MethodKind, MethodSet};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

use super::served::{
    Config, Credentials, Event, Events, Ids, confined, join, normalize, part_path, upload_refusal,
};

const CHUNK: usize = 64 * 1024;
const MAX_HEADER: usize = 1024;

/// What the client asked for in its exec command.
#[derive(Debug, PartialEq, Eq)]
pub enum Mode {
    /// `scp -f path`: we send the file.
    Source(String),
    /// `scp -t path`: we receive into it.
    Sink(String),
}

/// Parses `scp [-v] [-p] [-d] -f|-t [--] path`. Recursive copies are refused.
pub fn parse_command(cmd: &str) -> Result<Mode, String> {
    let mut words = cmd.split_whitespace();
    if words.next() != Some("scp") {
        return Err("only scp is available on this server".into());
    }
    let mut mode = None;
    let mut rest = Vec::new();
    let mut opts_done = false;
    for w in words {
        if !opts_done && w.starts_with('-') && w.len() > 1 {
            if w == "--" {
                opts_done = true;
                continue;
            }
            for c in w[1..].chars() {
                match c {
                    'f' | 't' => mode = Some(c),
                    'v' | 'p' | 'd' | 'q' | 'B' => {}
                    'r' => return Err("recursive copy is not supported".into()),
                    _ => return Err(format!("unsupported scp option -{c}")),
                }
            }
        } else {
            opts_done = true;
            rest.push(w);
        }
    }
    let path = rest.join(" ");
    let path = path.trim_matches(|c| c == '\'' || c == '"');
    let path = if path.is_empty() { "." } else { path };
    match mode {
        Some('f') => Ok(Mode::Source(path.to_owned())),
        Some('t') => Ok(Mode::Sink(path.to_owned())),
        _ => Err("scp needs -f or -t".into()),
    }
}

/// One transfer's context for events.
pub struct Ctx<'a> {
    pub cfg: &'a Config,
    pub events: &'a Events,
    pub ids: &'a Ids,
    pub peer: SocketAddr,
}

async fn fail<S: AsyncWrite + Unpin>(s: &mut S, msg: &str) -> Result<(), String> {
    let _ = s.write_all(format!("\x01scp: {msg}\n").as_bytes()).await;
    let _ = s.flush().await;
    Err(msg.to_owned())
}

/// Reads the client's one-byte reply; an error line is returned as Err.
async fn ack<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<(), String> {
    match s.read_u8().await.map_err(|e| e.to_string())? {
        0 => Ok(()),
        _ => {
            let line = read_header(s).await.unwrap_or_default();
            Err(line.trim().to_owned())
        }
    }
}

async fn read_header<S: AsyncRead + Unpin>(s: &mut S) -> Result<String, String> {
    let mut line = Vec::new();
    loop {
        let c = s.read_u8().await.map_err(|e| e.to_string())?;
        if c == b'\n' {
            return Ok(String::from_utf8_lossy(&line).into_owned());
        }
        if line.len() >= MAX_HEADER {
            return Err("protocol header too long".into());
        }
        line.push(c);
    }
}

/// `scp -f path`: sends one file.
pub async fn source<S>(s: &mut S, ctx: &Ctx<'_>, arg: &str) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ack(s).await?;
    let path = normalize("/", arg)
        .filter(|v| v != "/")
        .map(|v| join(&ctx.cfg.root, &v))
        .filter(|p| confined(&ctx.cfg.root, p));
    let file = match &path {
        Some(p) => tokio::fs::File::open(p).await.ok(),
        None => None,
    };
    let size = match &file {
        Some(f) => f
            .metadata()
            .await
            .ok()
            .filter(|m| m.is_file())
            .map(|m| m.len()),
        None => None,
    };
    let (Some(mut file), Some(size)) = (file, size) else {
        return fail(s, &format!("{arg}: No such file or directory")).await;
    };
    let name = arg.rsplit(['/', '\\']).next().unwrap_or("file");
    s.write_all(format!("C0644 {size} {name}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    ack(s).await?;
    let id = ctx.ids.next();
    (ctx.events)(Event::Started {
        id,
        peer: ctx.peer.to_string(),
        file: arg.trim_start_matches('/').to_owned(),
        upload: false,
        size: Some(size),
    });
    let result = async {
        let n = copy_exact(&mut file, s, size, id, ctx.events).await?;
        s.write_all(&[0]).await.map_err(|e| e.to_string())?;
        s.flush().await.map_err(|e| e.to_string())?;
        ack(s).await?;
        Ok::<_, String>(n)
    }
    .await;
    report(ctx, id, result)
}

/// `scp -t target`: receives one or more files into `target` (a folder, or the file
/// name for a single file).
pub async fn sink<S>(s: &mut S, ctx: &Ctx<'_>, target: &str) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let Some(virt) = normalize("/", target) else {
        return fail(s, &format!("{target}: outside the shared folder")).await;
    };
    let target_path = join(&ctx.cfg.root, &virt);
    let into_dir = virt == "/" || target.ends_with('/') || target_path.is_dir();
    s.write_all(&[0]).await.map_err(|e| e.to_string())?;
    s.flush().await.map_err(|e| e.to_string())?;
    let mut received = 0;
    loop {
        let kind = match s.read_u8().await {
            Ok(k) => k,
            Err(_) => return Ok(()), // the client is done
        };
        let header = read_header(s).await?;
        match kind {
            b'T' => {
                s.write_all(&[0]).await.map_err(|e| e.to_string())?;
                s.flush().await.map_err(|e| e.to_string())?;
            }
            b'C' => {
                let mut f = header.splitn(3, ' ');
                let _mode = f.next();
                let size: u64 = f
                    .next()
                    .and_then(|x| x.parse().ok())
                    .ok_or("bad scp header")?;
                let name = f.next().unwrap_or("").to_owned();
                if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
                    return fail(s, &format!("{name}: bad file name")).await;
                }
                if !into_dir && received > 0 {
                    return fail(s, "target is not a directory").await;
                }
                let path = if into_dir {
                    target_path.join(&name)
                } else {
                    target_path.clone()
                };
                if let Some(why) = upload_refusal(ctx.cfg, &path).await {
                    return fail(s, &format!("{name}: {why}")).await;
                }
                receive_one(s, ctx, &path, &name, size).await?;
                received += 1;
            }
            b'D' | b'E' => return fail(s, "directories are not supported").await,
            // \x01 / \x02: the client reports an error and stops.
            _ => return Err(header.trim().to_owned()),
        }
    }
}

async fn receive_one<S>(
    s: &mut S,
    ctx: &Ctx<'_>,
    path: &std::path::Path,
    name: &str,
    size: u64,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let part = part_path(path);
    let Ok(mut file) = tokio::fs::File::create(&part).await else {
        return fail(s, &format!("{name}: cannot create the file")).await;
    };
    s.write_all(&[0]).await.map_err(|e| e.to_string())?;
    s.flush().await.map_err(|e| e.to_string())?;
    let id = ctx.ids.next();
    let shown = path.strip_prefix(&ctx.cfg.root).map_or_else(
        |_| name.to_owned(),
        |p| p.to_string_lossy().replace('\\', "/"),
    );
    (ctx.events)(Event::Started {
        id,
        peer: ctx.peer.to_string(),
        file: shown,
        upload: true,
        size: Some(size),
    });
    let result = async {
        let n = copy_exact(s, &mut file, size, id, ctx.events).await?;
        file.flush().await.map_err(|e| e.to_string())?;
        ack(s).await?;
        Ok::<_, String>(n)
    }
    .await;
    drop(file);
    let result = match result {
        Ok(n) => tokio::fs::rename(&part, path)
            .await
            .map(|()| n)
            .map_err(|e| e.to_string()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    };
    let ok = result.is_ok();
    report(ctx, id, result)?;
    if ok {
        s.write_all(&[0]).await.map_err(|e| e.to_string())?;
        s.flush().await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn report(ctx: &Ctx<'_>, id: u64, result: Result<u64, String>) -> Result<(), String> {
    match result {
        Ok(bytes) => {
            (ctx.events)(Event::Done { id, bytes });
            Ok(())
        }
        Err(error) => {
            (ctx.events)(Event::Failed {
                id,
                error: error.clone(),
            });
            Err(error)
        }
    }
}

async fn copy_exact<R, W>(
    r: &mut R,
    w: &mut W,
    size: u64,
    id: u64,
    events: &Events,
) -> Result<u64, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    let mut reported = 0u64;
    while done < size {
        let want = usize::try_from((size - done).min(CHUNK as u64)).unwrap_or(CHUNK);
        let n = r.read(&mut buf[..want]).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("connection closed during the transfer".into());
        }
        w.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        done += n as u64;
        if done - reported >= 1 << 20 {
            reported = done;
            events(Event::Progress { id, bytes: done });
        }
    }
    Ok(done)
}

// ------------------------------------------------------------------ SSH server

/// Fresh host keys for one server start (RSA generation takes a moment: call from a
/// blocking thread). `rsa` false keeps tests fast.
pub fn host_keys(rsa: bool) -> crate::error::Result<Vec<PrivateKey>> {
    use crate::keys::{KeyKind, generate};
    let mut keys = vec![generate(KeyKind::Ed25519, "kenshell-scp")?];
    if rsa {
        keys.push(generate(KeyKind::Rsa3072, "kenshell-scp")?);
    }
    Ok(keys)
}

/// SHA-256 fingerprints of the host keys, as devices show them.
pub fn fingerprints(keys: &[PrivateKey]) -> Vec<String> {
    keys.iter()
        .map(|k| k.public_key().fingerprint(HashAlg::Sha256).to_string())
        .collect()
}

/// Serves SCP over SSH on `listener` until the task is aborted.
pub async fn serve(
    listener: TcpListener,
    keys: Vec<PrivateKey>,
    cfg: Arc<Config>,
    creds: Arc<Credentials>,
    events: Events,
) {
    let mut methods = MethodSet::empty();
    methods.push(MethodKind::Password);
    methods.push(MethodKind::KeyboardInteractive);
    let config = Arc::new(server::Config {
        keys,
        methods,
        preferred: crate::transport::ssh::preferred(true),
        auth_rejection_time: Duration::from_millis(500),
        auth_rejection_time_initial: Some(Duration::ZERO),
        max_auth_attempts: 3,
        inactivity_timeout: Some(Duration::from_secs(300)),
        ..Default::default()
    });
    let ids = Arc::new(Ids::default());
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        let handler = Handler {
            cfg: Arc::clone(&cfg),
            creds: Arc::clone(&creds),
            events: Arc::clone(&events),
            ids: Arc::clone(&ids),
            peer,
            pending: HashMap::new(),
        };
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            if let Ok(session) = server::run_stream(config, stream, handler).await {
                let _ = session.await;
            }
        });
    }
}

struct Handler {
    cfg: Arc<Config>,
    creds: Arc<Credentials>,
    events: Events,
    ids: Arc<Ids>,
    peer: SocketAddr,
    pending: HashMap<ChannelId, Channel<Msg>>,
}

impl server::Handler for Handler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if self.creds.check(user, password) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        match response {
            None => Ok(Auth::Partial {
                name: Cow::Borrowed(""),
                instructions: Cow::Borrowed(""),
                prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
            }),
            Some(mut answers) => {
                let pw = answers
                    .next()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                Ok(if self.creds.check(user, &pw) {
                    Auth::Accept
                } else {
                    Auth::reject()
                })
            }
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        // A connection only ever needs a couple of channels.
        if self.pending.len() < 4 {
            self.pending.insert(channel.id(), channel);
            reply.accept().await;
        } else {
            reply
                .reject(russh::ChannelOpenFailure::ResourceShortage)
                .await;
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        let Some(ch) = self.pending.remove(&channel) else {
            return session.channel_failure(channel);
        };
        let mode = parse_command(&String::from_utf8_lossy(data));
        session.channel_success(channel)?;
        let (cfg, events, ids, peer) = (
            Arc::clone(&self.cfg),
            Arc::clone(&self.events),
            Arc::clone(&self.ids),
            self.peer,
        );
        tokio::spawn(async move {
            let mut ch = ch;
            let writer = ch.make_writer();
            let code = {
                let reader = ch.make_reader();
                let mut s = tokio::io::join(reader, writer);
                let ctx = Ctx {
                    cfg: &cfg,
                    events: &events,
                    ids: &ids,
                    peer,
                };
                let r = match mode {
                    Ok(Mode::Source(p)) => source(&mut s, &ctx, &p).await,
                    Ok(Mode::Sink(p)) => sink(&mut s, &ctx, &p).await,
                    Err(e) => fail(&mut s, &e).await,
                };
                let _ = s.flush().await;
                u32::from(r.is_err())
            };
            let _ = ch.exit_status(code).await;
            let _ = ch.eof().await;
            let _ = ch.close().await;
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn commands() {
        assert_eq!(
            parse_command("scp -f flash.bin"),
            Ok(Mode::Source("flash.bin".into()))
        );
        assert_eq!(
            parse_command("scp -v -p -t /backups/"),
            Ok(Mode::Sink("/backups/".into()))
        );
        assert_eq!(parse_command("scp -dt ."), Ok(Mode::Sink(".".into())));
        assert_eq!(parse_command("scp -t"), Ok(Mode::Sink(".".into())));
        assert!(parse_command("scp -r -t x").is_err());
        assert!(parse_command("sh -c ls").is_err());
        assert!(parse_command("scp x").is_err());
    }

    fn ctx_parts(dir: &std::path::Path, upload: bool) -> (Config, Events, Arc<Mutex<Vec<Event>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&log);
        (
            Config {
                root: dir.into(),
                allow_upload: upload,
                overwrite: false,
            },
            Arc::new(move |e| sink.lock().unwrap().push(e)),
            log,
        )
    }

    /// The client side of `scp -f`: returns the file or the server's error.
    async fn client_fetch<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> Result<Vec<u8>, String> {
        s.write_all(&[0]).await.unwrap();
        let kind = s.read_u8().await.unwrap();
        let header = read_header(s).await.unwrap();
        if kind != b'C' {
            return Err(header);
        }
        let size: usize = header.split(' ').nth(1).unwrap().parse().unwrap();
        s.write_all(&[0]).await.unwrap();
        let mut data = vec![0u8; size];
        s.read_exact(&mut data).await.unwrap();
        assert_eq!(s.read_u8().await.unwrap(), 0);
        s.write_all(&[0]).await.unwrap();
        Ok(data)
    }

    /// The client side of `scp -t`: sends files, returns the first error line.
    async fn client_send<S: AsyncRead + AsyncWrite + Unpin>(
        s: &mut S,
        files: &[(&str, &[u8])],
    ) -> Result<(), String> {
        assert_eq!(s.read_u8().await.unwrap(), 0);
        for (name, data) in files {
            s.write_all(format!("C0644 {} {name}\n", data.len()).as_bytes())
                .await
                .unwrap();
            if s.read_u8().await.unwrap() != 0 {
                return Err(read_header(s).await.unwrap());
            }
            s.write_all(data).await.unwrap();
            s.write_all(&[0]).await.unwrap();
            assert_eq!(s.read_u8().await.unwrap(), 0);
        }
        Ok(())
    }

    #[tokio::test]
    async fn source_and_sink() {
        let dir = tempfile::tempdir().unwrap();
        let image: Vec<u8> = (0..200_000u32).map(|i| (i % 249) as u8).collect();
        std::fs::write(dir.path().join("ios.bin"), &image).unwrap();
        std::fs::create_dir(dir.path().join("backups")).unwrap();
        let (cfg, events, log) = ctx_parts(dir.path(), true);
        let ids = Ids::default();
        let ctx = Ctx {
            cfg: &cfg,
            events: &events,
            ids: &ids,
            peer: "10.0.0.2:22".parse().unwrap(),
        };

        // Download.
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (srv, got) = tokio::join!(source(&mut a, &ctx, "/ios.bin"), client_fetch(&mut b));
        srv.unwrap();
        assert_eq!(got.unwrap(), image);

        // Missing file and escape attempts.
        for bad in ["nope.bin", "../../etc/passwd"] {
            let (mut a, mut b) = tokio::io::duplex(8192);
            let (srv, got) = tokio::join!(source(&mut a, &ctx, bad), client_fetch(&mut b));
            assert!(srv.is_err());
            assert!(got.unwrap_err().contains("No such file"));
        }

        // Upload two files into a folder.
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (srv, sent) = tokio::join!(
            async {
                let r = sink(&mut a, &ctx, "backups").await;
                drop(a);
                r
            },
            async {
                let r = client_send(
                    &mut b,
                    &[
                        ("sw1-confg", b"hostname sw1\n"),
                        ("sw2-confg", &[7u8; 70_000]),
                    ],
                )
                .await;
                drop(b);
                r
            }
        );
        srv.unwrap();
        sent.unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("backups/sw1-confg")).unwrap(),
            b"hostname sw1\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("backups/sw2-confg"))
                .unwrap()
                .len(),
            70_000
        );

        // Single file under a new name, then the same name again (overwrite off).
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (srv, sent) = tokio::join!(
            async {
                let r = sink(&mut a, &ctx, "/renamed.cfg").await;
                drop(a);
                r
            },
            async {
                let r = client_send(&mut b, &[("whatever", b"x")]).await;
                drop(b);
                r
            }
        );
        srv.unwrap();
        sent.unwrap();
        assert_eq!(std::fs::read(dir.path().join("renamed.cfg")).unwrap(), b"x");
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (srv, sent) = tokio::join!(
            sink(&mut a, &ctx, "renamed.cfg"),
            client_send(&mut b, &[("w", b"y")])
        );
        assert!(srv.is_err());
        assert!(sent.unwrap_err().contains("overwrite is off"));

        // Bad names from the client.
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (srv, sent) = tokio::join!(
            sink(&mut a, &ctx, "backups"),
            client_send(&mut b, &[("../x", b"y")])
        );
        assert!(srv.is_err());
        assert!(sent.unwrap_err().contains("bad file name"));

        let ev = log.lock().unwrap().clone();
        assert!(
            ev.iter()
                .any(|e| matches!(e, Event::Done { bytes: 200_000, .. }))
        );
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, Event::Started { upload: true, .. }))
                .count(),
            3
        );

        // Uploads off.
        let (cfg, events, _) = ctx_parts(dir.path(), false);
        let ctx = Ctx {
            cfg: &cfg,
            events: &events,
            ids: &ids,
            peer: "10.0.0.2:22".parse().unwrap(),
        };
        let (mut a, mut b) = tokio::io::duplex(8192);
        let (_, sent) = tokio::join!(sink(&mut a, &ctx, "."), client_send(&mut b, &[("n", b"y")]));
        assert!(sent.unwrap_err().contains("uploads are off"));
    }

    struct AnyKey;

    impl russh::client::Handler for AnyKey {
        type Error = russh::Error;
        async fn check_server_key(
            &mut self,
            _key: &russh::keys::PublicKeyOrCertificate,
        ) -> Result<bool, Self::Error> {
            Ok(true)
        }
    }

    /// Over real SSH: our own SCP client (P5-T01) against this server.
    #[tokio::test]
    async fn over_ssh_with_the_kenshell_client() {
        use crate::transfer::scp;
        use std::sync::atomic::AtomicBool;

        let dir = tempfile::tempdir().unwrap();
        let image: Vec<u8> = (0..150_000u32).map(|i| (i % 241) as u8).collect();
        std::fs::write(dir.path().join("ios.bin"), &image).unwrap();
        let (cfg, events, log) = ctx_parts(dir.path(), true);
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let creds = Arc::new(Credentials {
            user: "kenshell".into(),
            password: "pw".into(),
        });
        let task = tokio::spawn(serve(
            l,
            host_keys(false).unwrap(),
            Arc::new(cfg),
            creds,
            events,
        ));

        let client_cfg = Arc::new(russh::client::Config::default());
        let mut h = russh::client::connect(Arc::clone(&client_cfg), addr, AnyKey)
            .await
            .unwrap();
        assert!(
            !h.authenticate_password("kenshell", "bad")
                .await
                .unwrap()
                .success()
        );
        assert!(
            h.authenticate_password("kenshell", "pw")
                .await
                .unwrap()
                .success()
        );

        let cancel = AtomicBool::new(false);
        let ch = h.channel_open_session().await.unwrap();
        ch.exec(true, scp::command('f', "ios.bin")).await.unwrap();
        let mut s = ch.into_stream();
        let mut got = Vec::new();
        scp::download(&mut s, &mut got, &cancel, &mut |_, _| {})
            .await
            .unwrap();
        assert_eq!(got, image);

        let ch = h.channel_open_session().await.unwrap();
        ch.exec(true, scp::command('t', "/")).await.unwrap();
        let mut s = ch.into_stream();
        let data = b"hostname sw9\n".to_vec();
        scp::upload(
            &mut s,
            &mut &data[..],
            data.len() as u64,
            "sw9-confg",
            &cancel,
            &mut |_, _| {},
        )
        .await
        .unwrap();
        drop(s);
        // The server finishes writing after its final ack.
        for _ in 0..50 {
            if dir.path().join("sw9-confg").exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(std::fs::read(dir.path().join("sw9-confg")).unwrap(), data);

        // A shell is not offered; a non-scp exec gets an error.
        let ch = h.channel_open_session().await.unwrap();
        ch.exec(true, "show version").await.unwrap();
        let mut s = ch.into_stream();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out).await;
        assert!(String::from_utf8_lossy(&out).contains("only scp"));

        assert!(
            log.lock()
                .unwrap()
                .iter()
                .any(|e| matches!(e, Event::Done { bytes: 150_000, .. }))
        );
        task.abort();
    }
}
