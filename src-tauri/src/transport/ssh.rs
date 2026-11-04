//! SSH v2 client (russh) — P2-T01 (auth), P2-T02 (host keys), P2-T04 (legacy
//! algorithms), P2-T06 (keepalive).
//!
//! * Host key: first connection → learned silently and stored; same key → accepted;
//!   different key → connection BLOCKED with `HostKeyChanged` (the one sanctioned
//!   interruption in the product rules). The user must explicitly forget the old key.
//! * Auth order: private key file (passphrase prompted if encrypted) →
//!   keyboard-interactive (what Cisco/Ruijie use) → password (3 attempts). Missing
//!   username → "login as:" prompt. All prompts go through [`Prompter`] (answered in
//!   the terminal); answers are never logged and are dropped right after use.
//! * Legacy option adds dh-group1/14-sha1, ssh-rsa, aes*-cbc, 3des-cbc, hmac-sha1.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use russh::{ChannelMsg, ChannelWriteHalf, MethodKind, Preferred, cipher, kex, mac};
use tokio::task::JoinHandle;

use super::net::{self, label};
use super::{HostKeyStore, Prompter, Transport, TransportEvent, TransportSink};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{
    HostKeyInfo, KnownHost, NetLink, PromptItem, PromptRequest, Protocol, SshOptions,
};

/// SSH handshake (version exchange + kex + host key) timeout.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
pub const DEFAULT_KEEPALIVE_SECS: u32 = 30;
const PASSWORD_ATTEMPTS: usize = 3;
const KBD_ROUNDS: usize = 8;
pub const TERM: &str = "xterm-256color";

/// Default preferences + legacy algorithms appended (lowest priority), so modern
/// servers still negotiate modern algorithms.
pub fn preferred(legacy: bool) -> Preferred {
    let mut p = Preferred::default();
    if !legacy {
        return p;
    }
    let mut kexes = p.kex.to_vec();
    kexes.extend([kex::DH_G14_SHA1, kex::DH_GEX_SHA1, kex::DH_G1_SHA1]);
    let mut ciphers = p.cipher.to_vec();
    ciphers.extend([
        cipher::AES_128_CBC,
        cipher::AES_192_CBC,
        cipher::AES_256_CBC,
        cipher::TRIPLE_DES_CBC,
    ]);
    let mut macs = p.mac.to_vec();
    for m in [mac::HMAC_SHA1, mac::HMAC_SHA1_ETM] {
        if !macs.contains(&m) {
            macs.push(m);
        }
    }
    dedup(&mut kexes);
    dedup(&mut ciphers);
    p.kex = Cow::Owned(kexes);
    p.cipher = Cow::Owned(ciphers);
    p.mac = Cow::Owned(macs);
    // `ssh-rsa` (SHA-1 signatures) is already in the default key list as
    // `Rsa { hash: None }`, accepted when the server offers only that.
    p
}

fn dedup<T: PartialEq + Clone>(v: &mut Vec<T>) {
    let mut out: Vec<T> = Vec::with_capacity(v.len());
    for x in v.drain(..) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    *v = out;
}

#[derive(Debug, Clone)]
enum KeyVerdict {
    Accepted(HostKeyInfo),
    Changed { stored: String, offered: String },
    Rejected(String),
}

struct Client {
    host: String,
    port: u16,
    keys: Arc<dyn HostKeyStore>,
    verdict: Arc<Mutex<Option<KeyVerdict>>>,
}

impl Client {
    fn decide(&self, key: &PublicKey) -> KeyVerdict {
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        let algorithm = key.algorithm().as_str().to_owned();
        let Ok(public_key) = key.to_openssh() else {
            return KeyVerdict::Rejected("unreadable host key".into());
        };
        match self.keys.get(&self.host, self.port) {
            Ok(Some(stored)) if stored.public_key.trim() == public_key.trim() => {
                KeyVerdict::Accepted(HostKeyInfo {
                    algorithm,
                    fingerprint,
                    first_seen: false,
                })
            }
            Ok(Some(stored)) => KeyVerdict::Changed {
                stored: stored.fingerprint,
                offered: fingerprint,
            },
            Ok(None) => {
                let known = KnownHost {
                    host: self.host.clone(),
                    port: self.port,
                    algorithm: algorithm.clone(),
                    public_key,
                    fingerprint: fingerprint.clone(),
                    added_at: 0,
                };
                match self.keys.save(&known) {
                    Ok(()) => KeyVerdict::Accepted(HostKeyInfo {
                        algorithm,
                        fingerprint,
                        first_seen: true,
                    }),
                    Err(e) => KeyVerdict::Rejected(format!("cannot store host key: {e}")),
                }
            }
            Err(e) => KeyVerdict::Rejected(format!("cannot read known hosts: {e}")),
        }
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let verdict = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => self.decide(key),
            // We never advertise certificate host-key algorithms.
            PublicKeyOrCertificate::Certificate(_) => {
                KeyVerdict::Rejected("host certificates are not supported".into())
            }
        };
        let ok = matches!(verdict, KeyVerdict::Accepted(_));
        if let Ok(mut v) = self.verdict.lock() {
            *v = Some(verdict);
        }
        Ok(ok)
    }
}

pub struct SshTransport {
    host: String,
    port: u16,
    username: Option<String>,
    opts: SshOptions,
    sink: TransportSink,
    prompter: Arc<dyn Prompter>,
    keys: Arc<dyn HostKeyStore>,
    size: (u16, u16),
    handle: Option<Handle<Client>>,
    writer: Option<ChannelWriteHalf<client::Msg>>,
    reader: Option<JoinHandle<()>>,
    host_key: Option<HostKeyInfo>,
}

impl SshTransport {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host: String,
        port: u16,
        username: Option<String>,
        opts: SshOptions,
        sink: TransportSink,
        prompter: Arc<dyn Prompter>,
        keys: Arc<dyn HostKeyStore>,
        cols: u16,
        rows: u16,
    ) -> Self {
        Self {
            host,
            port,
            username,
            opts,
            sink,
            prompter,
            keys,
            size: (cols, rows),
            handle: None,
            writer: None,
            reader: None,
            host_key: None,
        }
    }

    fn target(&self) -> String {
        label(&self.host, self.port)
    }

    fn config(&self) -> client::Config {
        let keepalive = self.opts.keepalive_secs.unwrap_or(DEFAULT_KEEPALIVE_SECS);
        client::Config {
            preferred: preferred(self.opts.legacy_algorithms),
            keepalive_interval: (keepalive > 0).then(|| Duration::from_secs(keepalive.into())),
            keepalive_max: 3,
            inactivity_timeout: None,
            nodelay: true,
            ..Default::default()
        }
    }

    async fn ask(&self, instruction: String, items: Vec<PromptItem>) -> Result<Vec<String>> {
        let n = items.len();
        let answers = self
            .prompter
            .ask(PromptRequest { instruction, items })
            .await
            .ok_or_else(|| AppError::with(ErrorCode::AuthFailed, "login cancelled"))?;
        if answers.len() != n {
            return Err(AppError::invalid("wrong number of answers"));
        }
        Ok(answers)
    }

    async fn resolve_user(&mut self) -> Result<String> {
        if let Some(u) = self.username.clone().filter(|u| !u.trim().is_empty()) {
            return Ok(u);
        }
        let user = self
            .ask(
                String::new(),
                vec![PromptItem {
                    text: "login as: ".into(),
                    echo: true,
                }],
            )
            .await?
            .remove(0)
            .trim()
            .to_owned();
        if user.is_empty() {
            return Err(AppError::with(ErrorCode::AuthFailed, "no user name"));
        }
        self.username = Some(user.clone());
        Ok(user)
    }

    async fn authenticate(&mut self, handle: &mut Handle<Client>, user: &str) -> Result<()> {
        let target = self.target();
        let ssh_err = |e: russh::Error| map_ssh(&target, e);
        let mut methods: Option<russh::MethodSet> = None;
        let allowed = |m: &Option<russh::MethodSet>, k: MethodKind| {
            m.as_ref().is_none_or(|set| set.contains(&k))
        };

        // 0. SSH agent (opt-in, see SshOptions::use_agent)
        if self.opts.use_agent {
            match agent::authenticate(handle, user).await {
                agent::Outcome::Success => return Ok(()),
                agent::Outcome::Failed(Some(remaining)) => methods = Some(remaining),
                agent::Outcome::Failed(None) | agent::Outcome::Unavailable => {}
            }
        }

        // 1. private key file
        if let Some(path) = self.opts.key_path.clone().filter(|p| !p.trim().is_empty()) {
            let key = match russh::keys::load_secret_key(&path, None) {
                Ok(k) => Some(k),
                Err(russh::keys::Error::KeyIsEncrypted) => {
                    let pass = self
                        .ask(
                            String::new(),
                            vec![PromptItem {
                                text: format!("Passphrase for key \"{path}\": "),
                                echo: false,
                            }],
                        )
                        .await?
                        .remove(0);
                    match russh::keys::load_secret_key(&path, Some(&pass)) {
                        Ok(k) => Some(k),
                        Err(e) => {
                            tracing::warn!("ssh key {path}: {e}");
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("ssh key {path}: {e}");
                    None
                }
            };
            if let Some(key) = key {
                let hash = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(ssh_err)?
                    .flatten();
                let res = handle
                    .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await
                    .map_err(ssh_err)?;
                match res {
                    AuthResult::Success => return Ok(()),
                    AuthResult::Failure {
                        remaining_methods, ..
                    } => methods = Some(remaining_methods),
                }
            }
        }

        // 2. keyboard-interactive (Cisco / Ruijie default)
        if allowed(&methods, MethodKind::KeyboardInteractive) {
            let mut resp = handle
                .authenticate_keyboard_interactive_start(user, None)
                .await
                .map_err(ssh_err)?;
            for _ in 0..KBD_ROUNDS {
                match resp {
                    KeyboardInteractiveAuthResponse::Success => return Ok(()),
                    KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods, ..
                    } => {
                        methods = Some(remaining_methods);
                        break;
                    }
                    KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        let answers = if prompts.is_empty() {
                            Vec::new()
                        } else {
                            let instruction = [name, instructions]
                                .into_iter()
                                .filter(|s| !s.trim().is_empty())
                                .collect::<Vec<_>>()
                                .join("\n");
                            let items = prompts
                                .into_iter()
                                .map(|p| PromptItem {
                                    text: p.prompt,
                                    echo: p.echo,
                                })
                                .collect();
                            self.ask(instruction, items).await?
                        };
                        resp = handle
                            .authenticate_keyboard_interactive_respond(answers)
                            .await
                            .map_err(ssh_err)?;
                    }
                }
            }
        }

        // 3. password
        if allowed(&methods, MethodKind::Password) {
            for attempt in 0..PASSWORD_ATTEMPTS {
                let text = if attempt == 0 {
                    format!("{user}@{}'s password: ", self.host)
                } else {
                    "Access denied\r\nPassword: ".into()
                };
                let pass = self
                    .ask(String::new(), vec![PromptItem { text, echo: false }])
                    .await?
                    .remove(0);
                match handle
                    .authenticate_password(user, pass)
                    .await
                    .map_err(ssh_err)?
                {
                    AuthResult::Success => return Ok(()),
                    AuthResult::Failure { .. } => continue,
                }
            }
        }
        Err(AppError::with(
            ErrorCode::AuthFailed,
            format!("{target}: authentication failed"),
        ))
    }
}

#[async_trait]
impl Transport for SshTransport {
    async fn connect(&mut self) -> Result<()> {
        if self.handle.is_some() {
            return Err(AppError::invalid("already connected"));
        }
        let target = self.target();
        let stream = net::connect(&self.host, self.port).await?;
        let verdict = Arc::new(Mutex::new(None));
        let client = Client {
            host: self.host.clone(),
            port: self.port,
            keys: Arc::clone(&self.keys),
            verdict: Arc::clone(&verdict),
        };
        let handshake = client::connect_stream(Arc::new(self.config()), stream, client);
        let result = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await;
        let verdict = verdict.lock().ok().and_then(|v| v.clone());
        let mut handle = match (result, verdict) {
            (_, Some(KeyVerdict::Changed { stored, offered })) => {
                return Err(AppError::with(
                    ErrorCode::HostKeyChanged,
                    format!("{target}: stored {stored}, offered {offered}"),
                ));
            }
            (_, Some(KeyVerdict::Rejected(why))) => {
                return Err(AppError::with(ErrorCode::Io, format!("{target}: {why}")));
            }
            (Err(_), _) => {
                return Err(AppError::with(
                    ErrorCode::Timeout,
                    format!("{target}: handshake"),
                ));
            }
            (Ok(Err(e)), _) => return Err(map_ssh(&target, e)),
            (Ok(Ok(h)), Some(KeyVerdict::Accepted(info))) => {
                self.host_key = Some(info);
                h
            }
            (Ok(Ok(_)), None) => {
                return Err(AppError::internal("host key was not checked"));
            }
        };

        let user = self.resolve_user().await?;
        self.authenticate(&mut handle, &user).await?;

        let ssh_err = |e: russh::Error| map_ssh(&target, e);
        let channel = handle.channel_open_session().await.map_err(ssh_err)?;
        let (cols, rows) = self.size;
        channel
            .request_pty(false, TERM, cols.into(), rows.into(), 0, 0, &[])
            .await
            .map_err(ssh_err)?;
        channel.request_shell(false).await.map_err(ssh_err)?;
        let (mut read, write) = channel.split();

        let sink = self.sink.clone();
        self.reader = Some(tokio::spawn(async move {
            loop {
                match read.wait().await {
                    Some(ChannelMsg::Data { data })
                    | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        if sink
                            .send(TransportEvent::Data(data.to_vec()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Some(ChannelMsg::Eof | ChannelMsg::Close) | None => break,
                    Some(_) => {}
                }
            }
            let _ = sink.send(TransportEvent::Closed(None)).await;
        }));
        self.writer = Some(write);
        self.handle = Some(handle);
        Ok(())
    }

    async fn write(&mut self, data: &[u8]) -> Result<()> {
        let target = self.target();
        let w = self
            .writer
            .as_ref()
            .ok_or_else(|| AppError::invalid("not connected"))?;
        w.data(data).await.map_err(|e| map_ssh(&target, e))
    }

    async fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.size = (cols, rows);
        let target = self.target();
        match &self.writer {
            Some(w) => w
                .window_change(cols.into(), rows.into(), 0, 0)
                .await
                .map_err(|e| map_ssh(&target, e)),
            None => Ok(()),
        }
    }

    async fn send_break(&mut self) -> Result<()> {
        Err(AppError::with(
            ErrorCode::Unsupported,
            "BREAK over SSH is not supported",
        ))
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(r) = self.reader.take() {
            r.abort();
        }
        if let Some(w) = self.writer.take() {
            let _ = w.close().await;
        }
        if let Some(h) = self.handle.take() {
            let _ = h
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
        Ok(())
    }

    fn net_link(&self) -> Option<NetLink> {
        Some(NetLink {
            protocol: Protocol::Ssh,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            host_key: self.host_key.clone(),
        })
    }
}

/// russh error → friendly code. Never includes secrets (russh errors carry none).
fn map_ssh(target: &str, e: russh::Error) -> AppError {
    use russh::Error as E;
    let code = match &e {
        E::NoCommonAlgo { .. } => ErrorCode::NoCommonAlgorithm,
        E::IO(io) => return net::map_io(target, io),
        E::Disconnect | E::HUP => ErrorCode::ConnectionClosed,
        E::ConnectionTimeout | E::KeepaliveTimeout | E::InactivityTimeout | E::Elapsed(_) => {
            ErrorCode::Timeout
        }
        E::NotAuthenticated | E::NoAuthMethod | E::UnsupportedAuthMethod => ErrorCode::AuthFailed,
        _ => ErrorCode::Io,
    };
    AppError::with(code, format!("{target}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_adds_old_algorithms_at_the_end_only() {
        let modern = preferred(false);
        assert!(!modern.kex.contains(&kex::DH_G1_SHA1));
        assert!(!modern.cipher.contains(&cipher::AES_128_CBC));
        let legacy = preferred(true);
        assert_eq!(
            legacy.kex.first(),
            modern.kex.first(),
            "modern stays preferred"
        );
        assert!(legacy.kex.contains(&kex::DH_G1_SHA1));
        assert!(legacy.kex.contains(&kex::DH_G14_SHA1));
        assert!(legacy.cipher.contains(&cipher::AES_128_CBC));
        assert!(legacy.cipher.contains(&cipher::TRIPLE_DES_CBC));
        assert!(legacy.mac.contains(&mac::HMAC_SHA1));
        let mut k = legacy.kex.to_vec();
        let n = k.len();
        dedup(&mut k);
        assert_eq!(k.len(), n, "no duplicates");
    }
}

/// SSH agent authentication (P2-T05): macOS/Linux `SSH_AUTH_SOCK`, Windows OpenSSH
/// agent named pipe, then Pageant.
mod agent {
    use russh::client::{AuthResult, Handle};
    use russh::keys::agent::client::AgentClient;
    use tokio::io::{AsyncRead, AsyncWrite};

    use super::Client;

    /// Each identity costs one server-side auth attempt; stay well below the usual
    /// network-device limit so the password still gets its turn.
    const MAX_AGENT_KEYS: usize = 2;

    pub enum Outcome {
        Success,
        /// Remaining methods from the last failure (if any key was tried).
        Failed(Option<russh::MethodSet>),
        Unavailable,
    }

    pub async fn authenticate(handle: &mut Handle<Client>, user: &str) -> Outcome {
        #[cfg(unix)]
        {
            match AgentClient::connect_env().await {
                Ok(mut a) => try_keys(handle, user, &mut a).await,
                Err(e) => {
                    tracing::debug!("ssh agent unavailable: {e}");
                    Outcome::Unavailable
                }
            }
        }
        #[cfg(windows)]
        {
            if let Ok(mut a) = AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await
            {
                return try_keys(handle, user, &mut a).await;
            }
            match AgentClient::connect_pageant().await {
                Ok(mut a) => try_keys(handle, user, &mut a).await,
                Err(e) => {
                    tracing::debug!("ssh agent unavailable: {e}");
                    Outcome::Unavailable
                }
            }
        }
    }

    async fn try_keys<S>(
        handle: &mut Handle<Client>,
        user: &str,
        agent: &mut AgentClient<S>,
    ) -> Outcome
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let ids = match agent.request_identities().await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::debug!("ssh agent identities: {e}");
                return Outcome::Unavailable;
            }
        };
        if ids.is_empty() {
            return Outcome::Unavailable;
        }
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten();
        let mut last = None;
        for id in ids.iter().take(MAX_AGENT_KEYS) {
            let key = id.public_key().into_owned();
            let hash = key.algorithm().is_rsa().then_some(hash).flatten();
            match handle
                .authenticate_publickey_with(user, key, hash, agent)
                .await
            {
                Ok(AuthResult::Success) => return Outcome::Success,
                Ok(AuthResult::Failure {
                    remaining_methods, ..
                }) => last = Some(remaining_methods),
                Err(e) => tracing::debug!("ssh agent key rejected: {e}"),
            }
        }
        Outcome::Failed(last)
    }
}
