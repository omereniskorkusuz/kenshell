//! DTOs shared between backend modules and the frontend (serialized camelCase).
//! The TypeScript mirror lives in `src/lib/ipc.ts` — keep both in sync.

use serde::{Deserialize, Serialize};

use crate::error::AppError;

// ---------------------------------------------------------------- storage

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Serial,
    Ssh,
    Telnet,
    Rlogin,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FlowControl {
    #[default]
    None,
    RtsCts,
    XonXoff,
}

/// Serial line settings. `baud: None` = automatic detection (autobaud).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialSettings {
    /// Port path used to connect (`/dev/cu.usbserial-…` on macOS, `COM4` on Windows).
    pub port: String,
    pub baud: Option<u32>,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow_control: FlowControl,
    /// Initial DTR/RTS line state (`None` = driver default).
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: None,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            flow_control: FlowControl::None,
            dtr: None,
            rts: None,
        }
    }
}

impl SerialSettings {
    /// Framing label, e.g. `8N1`.
    pub fn framing(&self) -> String {
        let p = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        format!("{}{}{}", self.data_bits, p, self.stop_bits)
    }
}

/// A saved session (connection profile). Never contains secrets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub protocol: Protocol,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub serial: Option<SerialSettings>,
    /// `prod` | `lab` | `test` | custom label (visual only).
    pub env_tag: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    /// SSH options (ssh sessions only).
    pub ssh: Option<SshOptions>,
    /// Shared login (user name + keychain password) instead of the session's own.
    pub credential_profile_id: Option<i64>,
    /// Commands sent once the device is idle after connecting (empty = off).
    pub auto_commands: Vec<String>,
    pub sort_order: i64,
    /// Unix epoch milliseconds.
    pub created_at: i64,
    pub updated_at: i64,
    pub last_connected_at: Option<i64>,
}

/// Fields a user can create/update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInput {
    pub folder_id: Option<i64>,
    pub name: String,
    pub protocol: Protocol,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub serial: Option<SerialSettings>,
    pub env_tag: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub ssh: Option<SshOptions>,
    #[serde(default)]
    pub credential_profile_id: Option<i64>,
    #[serde(default)]
    pub auto_commands: Vec<String>,
}

/// Remembered facts about a console cable, keyed by USB serial number (or port path).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMemory {
    pub key: String,
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub baud: u32,
    pub last_device: Option<String>,
    pub updated_at: i64,
}

// ---------------------------------------------------------------- console detection

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChipInfo {
    /// e.g. `FTDI`, `Prolific`, `Silicon Labs`, `WCH`, `Cisco`
    pub vendor: String,
    /// e.g. `FT232R`, `PL2303`, `CP210x`, `CH340`, `USB Console`
    pub model: String,
    pub vid: u16,
    pub pid: u16,
    pub driver_name: Option<String>,
    /// Official driver download page.
    pub driver_url: Option<String>,
}

/// One detected serial console (macOS tty/cu pair merged into one entry).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsolePort {
    /// Stable identity: USB serial number if known, else the port path.
    pub key: String,
    /// Path to connect to (`cu.*` on macOS, `COMx` on Windows).
    pub path: String,
    /// The merged twin (`tty.*` on macOS).
    pub alt_path: Option<String>,
    /// Short human label: `COM4`, `usbserial-A10K3XYZ`.
    pub display_name: String,
    pub chip: Option<ChipInfo>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
    pub remembered_baud: Option<u32>,
    pub last_device: Option<String>,
}

/// A known console chip is on USB but the OS created no serial port → driver missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverHint {
    pub chip: ChipInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleSnapshot {
    pub ports: Vec<ConsolePort>,
    pub driver_hints: Vec<DriverHint>,
}

/// Emitted as event [`crate::events::CONSOLE_CHANGED`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleChange {
    pub snapshot: ConsoleSnapshot,
    /// Keys of ports that appeared.
    pub attached: Vec<String>,
    /// Keys of ports that disappeared.
    pub detached: Vec<String>,
}

// ---------------------------------------------------------------- live sessions

/// Runtime id of an open (live) session. Distinct from the saved `SessionRecord.id`.
pub type RuntimeId = u32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OpenTarget {
    Serial {
        settings: SerialSettings,
    },
    /// ssh / telnet / rlogin / raw.
    Network {
        protocol: Protocol,
        host: String,
        /// `None` = protocol default (22 / 23 / 513; raw has no default).
        port: Option<u16>,
        username: Option<String>,
        #[serde(default)]
        ssh: SshOptions,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    /// Saved session this tab came from, if any (updates `last_connected_at`).
    pub session_id: Option<i64>,
    pub target: OpenTarget,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnState {
    Connecting,
    /// Autobaud running.
    Detecting,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaudTrial {
    pub baud: u32,
    /// 0.0 – 1.0 (+ prompt bonus, capped at 1.0)
    pub score: f32,
    pub prompt_match: bool,
    pub bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthKind {
    /// Port open, nothing received (device off / wrong cable).
    NoData,
    /// The USB cable vanished; session kept in `disconnected`.
    CableRemoved,
    /// Same cable re-attached.
    CableReturned,
}

/// Connection parameters once established (identity bar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialLink {
    pub path: String,
    pub display_name: String,
    pub baud: u32,
    /// e.g. `8N1`
    pub framing: String,
    /// e.g. `FTDI FT232R`
    pub chip: Option<String>,
    /// Baud found by autobaud (vs. manual / remembered).
    pub auto_detected: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SessionEvent {
    State {
        state: ConnState,
        error: Option<AppError>,
    },
    /// Autobaud progress: emitted after each trial and once with `result` at the end.
    Autobaud {
        trials: Vec<BaudTrial>,
        current: Option<u32>,
        result: Option<u32>,
        /// Best candidate when nothing passed the acceptance threshold.
        suggestion: Option<u32>,
    },
    Link {
        link: SerialLink,
    },
    NetLink {
        link: NetLink,
    },
    /// Login questions; answer with `live_answer` (or close the tab to abort).
    Prompt {
        request: PromptRequest,
    },
    /// Session log state after connecting: `path` when logging, `error` if the log
    /// could not be opened (the session itself keeps working).
    Logging {
        path: Option<String>,
        error: Option<AppError>,
    },
    Health {
        kind: HealthKind,
    },
    /// Last event before the session's output moves to another window (P2-T19).
    Parked,
}

// ---------------------------------------------------------------- network sessions (Phase 2)

/// SSH options of a session (all optional; defaults are secure + quiet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SshOptions {
    /// Also offer legacy algorithms (diffie-hellman-group1/14-sha1, ssh-rsa, *-cbc,
    /// 3des-cbc, hmac-sha1) for old Cisco/Ruijie/Juniper devices.
    pub legacy_algorithms: bool,
    /// Private key file (OpenSSH format). Agent and password are tried too.
    pub key_path: Option<String>,
    /// SSH keepalive interval in seconds (`None` = 30, `Some(0)` = off).
    pub keepalive_secs: Option<u32>,
    /// Try keys from the SSH agent (macOS/Linux ssh-agent, Windows OpenSSH agent or
    /// Pageant). Off by default: network devices often allow only ~3 auth attempts,
    /// which agent keys could use up before the password is asked.
    pub use_agent: bool,
    /// Connect through this SSH server first (bastion / jump host).
    pub jump: Option<JumpHost>,
    /// Port forwards started after login.
    pub forwards: Vec<Forward>,
    /// SOCKS5 / HTTP CONNECT proxy for the first TCP connection.
    pub proxy: Option<ProxyConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpHost {
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ForwardKind {
    /// `-L`: local port → (through the server) → target.
    Local,
    /// `-R`: server port → (back through us) → target reachable from this computer.
    Remote,
    /// `-D`: local SOCKS5 proxy through the server.
    Dynamic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Forward {
    pub kind: ForwardKind,
    pub listen_port: u16,
    /// Not used for `dynamic`.
    pub target_host: Option<String>,
    pub target_port: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProxyKind {
    Socks5,
    Http,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
}

impl Protocol {
    pub fn default_port(self) -> Option<u16> {
        match self {
            Protocol::Ssh => Some(22),
            Protocol::Telnet => Some(23),
            Protocol::Rlogin => Some(513),
            Protocol::Raw | Protocol::Serial => None,
        }
    }
}

/// Network connection facts (identity bar / details).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetLink {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// SSH only.
    pub host_key: Option<HostKeyInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyInfo {
    /// e.g. `ssh-ed25519`
    pub algorithm: String,
    /// `SHA256:…`
    pub fingerprint: String,
    /// First connection to this host: the key was just learned and stored.
    pub first_seen: bool,
}

/// One question during login (PuTTY-style, answered inside the terminal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptItem {
    pub text: String,
    /// `false` = secret (password): never echoed, never logged.
    pub echo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    /// Banner / instruction lines to print before the prompts (may be empty).
    pub instruction: String,
    pub items: Vec<PromptItem>,
}

/// A stored SSH host key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHost {
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    /// OpenSSH public key line (`ssh-ed25519 AAAA…`).
    pub public_key: String,
    pub fingerprint: String,
    pub added_at: i64,
}

/// A reusable login. The password itself is only in the OS keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialProfile {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
    /// Filled by the command layer from the keychain (never stored in SQLite).
    #[serde(default)]
    pub has_password: bool,
}
