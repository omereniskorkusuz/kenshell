//! SSH server mode (P2-T22): the same CLI engine behind russh, with password and
//! keyboard-interactive auth (profile credentials). Options for KenShell tests:
//! `--host-key-seed N` (deterministic Ed25519 host key; change N to simulate a
//! replaced device) and `--legacy-only` (offer only diffie-hellman-group1/14-sha1,
//! aes128-cbc, hmac-sha1 — like an old IOS box).

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{HashAlg, PrivateKey, PublicKey};
use russh::server::{self, Auth, Msg, Response, Session as SshSession};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Preferred, cipher, kex, mac};

use crate::engine::Session;
use crate::profile::Profile;

pub struct Options {
    pub bind: String,
    pub port: u16,
    pub host_key_seed: u64,
    pub legacy_only: bool,
    /// OpenSSH public key lines accepted for publickey auth (agent tests).
    pub authorized_keys: Vec<String>,
}

fn host_key(seed: u64) -> PrivateKey {
    let mut bytes = [0u8; 32];
    // Expand the seed deterministically (test tool: not a secret).
    let mut x = seed ^ 0x9E37_79B9_7F4A_7C15;
    for chunk in bytes.chunks_mut(8) {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        chunk.copy_from_slice(&x.to_le_bytes());
    }
    PrivateKey::from(Ed25519Keypair::from_seed(&bytes))
}

pub fn run(profile: &'static Profile, opts: Options) -> std::io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(serve(profile, opts))
}

async fn serve(profile: &'static Profile, opts: Options) -> std::io::Result<()> {
    let key = host_key(opts.host_key_seed);
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256);
    let mut preferred = Preferred::default();
    if opts.legacy_only {
        preferred.kex = Cow::Borrowed(&[kex::DH_G14_SHA1, kex::DH_G1_SHA1]);
        preferred.cipher = Cow::Borrowed(&[cipher::AES_128_CBC]);
        preferred.mac = Cow::Borrowed(&[mac::HMAC_SHA1]);
    }
    let authorized: Arc<Vec<PublicKey>> = Arc::new(
        opts.authorized_keys
            .iter()
            .map(|line| PublicKey::from_openssh(line.trim()))
            .collect::<Result<_, _>>()
            .map_err(|e| std::io::Error::other(format!("--authorized-key: {e}")))?,
    );
    let mut methods = MethodSet::empty();
    if !authorized.is_empty() {
        methods.push(MethodKind::PublicKey);
    }
    methods.push(MethodKind::KeyboardInteractive);
    methods.push(MethodKind::Password);
    let config = Arc::new(server::Config {
        keys: vec![key],
        methods,
        preferred,
        auth_rejection_time: Duration::from_millis(50),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        inactivity_timeout: Some(Duration::from_secs(3600)),
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind((opts.bind.as_str(), opts.port)).await?;
    eprintln!(
        "device-simulator: {} ({}) listening on {} [ssh{}] host key {fingerprint}",
        profile.name,
        profile.vendor,
        listener.local_addr()?,
        if opts.legacy_only {
            ", legacy only"
        } else {
            ""
        },
    );
    // In-memory device storage for SCP (`flash:` files), shared by all connections.
    let files: Files = Arc::new(std::sync::Mutex::new(std::collections::HashMap::from([(
        "flash:startup-config".to_owned(),
        b"hostname sim\n!\ninterface GigabitEthernet0/1\n description uplink\n!\nend\n".to_vec(),
    )])));
    loop {
        let (stream, peer) = listener.accept().await?;
        let config = Arc::clone(&config);
        let authorized = Arc::clone(&authorized);
        let files = Arc::clone(&files);
        tokio::spawn(async move {
            eprintln!("+ {peer} (ssh)");
            let handler = Handler {
                profile,
                engine: None,
                shell: None,
                authorized,
                pending: std::collections::HashMap::new(),
                files,
            };
            match server::run_stream(config, stream, handler).await {
                Ok(session) => {
                    if let Err(e) = session.await {
                        eprintln!("  {peer}: {e}");
                    }
                }
                Err(e) => eprintln!("  {peer}: {e}"),
            }
            eprintln!("- {peer}");
        });
    }
}

struct Handler {
    profile: &'static Profile,
    engine: Option<Session<'static>>,
    /// Channel running the CLI; tunnel channels (direct-tcpip) never reach the engine.
    shell: Option<ChannelId>,
    authorized: Arc<Vec<PublicKey>>,
    /// Session channels not yet a shell or an exec.
    pending: std::collections::HashMap<ChannelId, Channel<Msg>>,
    files: Files,
}

type Files = Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>>;

/// Minimal SCP server (`scp -f path`, `scp -t dir`) over an exec channel's stream.
async fn scp_serve<S>(mut s: S, command: String, files: Files)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut parts = command.split_whitespace();
    let _ = parts.next();
    let mode = parts.next().unwrap_or("");
    let path = parts
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches('\'')
        .to_owned();
    if mode == "-f" {
        let data = files.lock().ok().and_then(|f| f.get(&path).cloned());
        let _ = s.read_u8().await;
        let Some(data) = data else {
            let _ = s
                .write_all(format!("\x01scp: {path}: No such file or directory\n").as_bytes())
                .await;
            return;
        };
        let name = path.rsplit([':', '/']).next().unwrap_or("file");
        let _ = s
            .write_all(format!("C0644 {} {name}\n", data.len()).as_bytes())
            .await;
        let _ = s.read_u8().await;
        let _ = s.write_all(&data).await;
        let _ = s.write_all(&[0]).await;
        let _ = s.read_u8().await;
    } else if mode == "-t" {
        let _ = s.write_all(&[0]).await;
        let mut header = Vec::new();
        while let Ok(c) = s.read_u8().await {
            if c == b'\n' {
                break;
            }
            header.push(c);
        }
        let h = String::from_utf8_lossy(&header).into_owned();
        let mut it = h.splitn(3, ' ');
        let _ = it.next();
        let size: usize = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let name = it.next().unwrap_or("file").to_owned();
        let _ = s.write_all(&[0]).await;
        let mut data = vec![0u8; size];
        if s.read_exact(&mut data).await.is_err() {
            return;
        }
        let _ = s.read_u8().await;
        let key = if path.ends_with(':') || path.ends_with('/') {
            format!("{path}{name}")
        } else {
            format!("{path}/{name}")
        };
        if let Ok(mut f) = files.lock() {
            f.insert(key, data);
        }
        let _ = s.write_all(&[0]).await;
    }
    let _ = s.shutdown().await;
}

impl Handler {
    fn valid(&self, user: &str, password: &str) -> bool {
        user == self.profile.username && password == self.profile.password
    }

    fn key_ok(&self, user: &str, key: &PublicKey) -> bool {
        user == self.profile.username
            && self
                .authorized
                .iter()
                .any(|k| k.key_data() == key.key_data())
    }
}

impl server::Handler for Handler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if self.valid(user, password) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: server::PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if self.key_ok(user, offer.public_key) {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    /// Called after russh verified the signature.
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(if self.key_ok(user, key.public_key) {
            eprintln!("  publickey auth: {}", key.algorithm);
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
                Ok(if self.valid(user, &pw) {
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
        self.pending.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    /// `scp -f` / `scp -t` (KenShell's SCP transfers, P5-T01).
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).into_owned();
        match self.pending.remove(&channel) {
            Some(ch) if command.starts_with("scp ") => {
                session.channel_success(channel)?;
                let files = Arc::clone(&self.files);
                tokio::spawn(scp_serve(ch.into_stream(), command, files));
            }
            _ => session.channel_failure(channel)?,
        }
        Ok(())
    }

    /// direct-tcpip: lets KenShell tests use this server as a jump host and for
    /// local / dynamic port forwarding.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        let Ok(port) = u16::try_from(port_to_connect) else {
            return Ok(());
        };
        match tokio::net::TcpStream::connect((host_to_connect, port)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut ch, &mut tcp).await;
                });
            }
            Err(e) => eprintln!("  direct-tcpip {host_to_connect}:{port}: {e}"),
        }
        Ok(())
    }

    /// tcpip-forward (remote forwarding): listen on 127.0.0.1:port and hand every
    /// connection back to the client as a forwarded-tcpip channel.
    async fn tcpip_forward(
        &mut self,
        _address: &str,
        port: &mut u32,
        session: &mut SshSession,
    ) -> Result<bool, Self::Error> {
        let Ok(p) = u16::try_from(*port) else {
            return Ok(false);
        };
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", p)).await else {
            return Ok(false);
        };
        *port = u32::from(listener.local_addr().map(|a| a.port()).unwrap_or(p));
        let bound = *port;
        let handle = session.handle();
        tokio::spawn(async move {
            while let Ok((mut sock, peer)) = listener.accept().await {
                let h = handle.clone();
                tokio::spawn(async move {
                    if let Ok(ch) = h
                        .channel_open_forwarded_tcpip(
                            "127.0.0.1",
                            bound,
                            peer.ip().to_string(),
                            u32::from(peer.port()),
                        )
                        .await
                    {
                        let mut stream = ch.into_stream();
                        let _ = tokio::io::copy_bidirectional(&mut sock, &mut stream).await;
                    }
                });
            }
        });
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        // A shell channel is driven through `data` below, not as a stream.
        self.pending.remove(&channel);
        let mut engine = Session::new_authenticated(self.profile);
        let out = engine.start();
        self.engine = Some(engine);
        self.shell = Some(channel);
        session.data(channel, out)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        if self.shell != Some(channel) {
            return Ok(());
        }
        let Some(engine) = self.engine.as_mut() else {
            return Ok(());
        };
        let out = engine.feed(data);
        if !out.is_empty() {
            session.data(channel, out)?;
        }
        if engine.is_closed() {
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }
}
