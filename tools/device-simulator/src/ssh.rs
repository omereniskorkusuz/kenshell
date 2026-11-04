//! SSH server mode (P2-T22): the same CLI engine behind russh, with password and
//! keyboard-interactive auth (profile credentials). Options for KenShell tests:
//! `--host-key-seed N` (deterministic Ed25519 host key; change N to simulate a
//! replaced device) and `--legacy-only` (offer only diffie-hellman-group1/14-sha1,
//! aes128-cbc, hmac-sha1 — like an old IOS box).

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{HashAlg, PrivateKey};
use russh::server::{self, Auth, Msg, Response, Session as SshSession};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Preferred, cipher, kex, mac};

use crate::engine::Session;
use crate::profile::Profile;

pub struct Options {
    pub bind: String,
    pub port: u16,
    pub host_key_seed: u64,
    pub legacy_only: bool,
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
    let mut methods = MethodSet::empty();
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
    loop {
        let (stream, peer) = listener.accept().await?;
        let config = Arc::clone(&config);
        tokio::spawn(async move {
            eprintln!("+ {peer} (ssh)");
            let handler = Handler {
                profile,
                engine: None,
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
}

impl Handler {
    fn valid(&self, user: &str, password: &str) -> bool {
        user == self.profile.username && password == self.profile.password
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
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
        let mut engine = Session::new_authenticated(self.profile);
        let out = engine.start();
        self.engine = Some(engine);
        session.data(channel, out)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut SshSession,
    ) -> Result<(), Self::Error> {
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
