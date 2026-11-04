//! Pure input validation / normalization for the repositories (no DB access).
//! Errors are `Invalid` with a short English detail; the UI maps the code.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use crate::error::{AppError, Result};
use crate::model::{ForwardKind, Protocol, SerialSettings, SessionInput, SshOptions};

pub(super) const MAX_NAME_CHARS: usize = 200;
const MAX_HOST_CHARS: usize = 255;
const MAX_USERNAME_CHARS: usize = 255;
const MAX_PORT_PATH_CHARS: usize = 255;
const MAX_ENV_TAG_CHARS: usize = 32;
const MAX_TAG_CHARS: usize = 64;
const MAX_TAGS: usize = 50;
const MAX_SETTING_KEY_CHARS: usize = 128;

/// Accepted serial speeds (covers 50 baud legacy up to 4 Mbaud USB UARTs).
pub(super) const BAUD_RANGE: RangeInclusive<u32> = 50..=4_000_000;
const DATA_BITS: RangeInclusive<u8> = 5..=8;

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Trimmed, non-empty, single-line name of at most [`MAX_NAME_CHARS`] characters.
/// `what` is `"folder"` or `"session"` (used in the detail only).
pub(super) fn name(raw: &str, what: &str) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::invalid(format!("{what} name is empty")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::invalid(format!(
            "{what} name longer than {MAX_NAME_CHARS} characters"
        )));
    }
    if has_control(name) {
        return Err(AppError::invalid(format!(
            "{what} name contains control characters"
        )));
    }
    Ok(name.to_owned())
}

pub(super) fn baud(baud: u32) -> Result<u32> {
    if BAUD_RANGE.contains(&baud) {
        Ok(baud)
    } else {
        Err(AppError::invalid(format!(
            "baud must be {}-{}",
            BAUD_RANGE.start(),
            BAUD_RANGE.end()
        )))
    }
}

pub(super) fn setting_key(raw: &str) -> Result<&str> {
    let key = raw.trim();
    if key.is_empty() || key != raw {
        return Err(AppError::invalid(
            "setting key is empty or has surrounding whitespace",
        ));
    }
    if key.chars().count() > MAX_SETTING_KEY_CHARS || has_control(key) {
        return Err(AppError::invalid("setting key is malformed"));
    }
    Ok(key)
}

/// Optional single-line text: trimmed, empty → `None`.
fn optional_text(raw: Option<&str>, field: &str, max: usize) -> Result<Option<String>> {
    let Some(v) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > max {
        return Err(AppError::invalid(format!(
            "{field} longer than {max} characters"
        )));
    }
    if has_control(v) {
        return Err(AppError::invalid(format!(
            "{field} contains control characters"
        )));
    }
    Ok(Some(v.to_owned()))
}

fn host(raw: Option<&str>) -> Result<String> {
    let host = optional_text(raw, "host", MAX_HOST_CHARS)?
        .ok_or_else(|| AppError::invalid("host is required"))?;
    if host.chars().any(char::is_whitespace) {
        return Err(AppError::invalid("host contains whitespace"));
    }
    Ok(host)
}

fn serial(raw: Option<&SerialSettings>) -> Result<SerialSettings> {
    let s = raw.ok_or_else(|| AppError::invalid("serial settings are required"))?;
    let port = optional_text(Some(&s.port), "serial port", MAX_PORT_PATH_CHARS)?
        .ok_or_else(|| AppError::invalid("serial port is required"))?;
    if let Some(b) = s.baud {
        baud(b)?;
    }
    if !DATA_BITS.contains(&s.data_bits) {
        return Err(AppError::invalid("data bits must be 5-8"));
    }
    if !matches!(s.stop_bits, 1 | 2) {
        return Err(AppError::invalid("stop bits must be 1 or 2"));
    }
    Ok(SerialSettings { port, ..s.clone() })
}

/// Lowercase label (`prod`, `lab`, …) or `None`.
fn env_tag(raw: Option<&str>) -> Result<Option<String>> {
    Ok(optional_text(raw, "env tag", MAX_ENV_TAG_CHARS)?.map(|t| t.to_lowercase()))
}

/// Trimmed, empty ones dropped, deduplicated case-insensitively keeping the first
/// spelling and the original order.
fn tags(raw: &[String]) -> Result<Vec<String>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in raw {
        let Some(tag) = optional_text(Some(t), "tag", MAX_TAG_CHARS)? else {
            continue;
        };
        if seen.insert(tag.to_lowercase()) {
            out.push(tag);
        }
    }
    if out.len() > MAX_TAGS {
        return Err(AppError::invalid(format!("more than {MAX_TAGS} tags")));
    }
    Ok(out)
}

/// A [`SessionInput`] that passed validation. Fields irrelevant to the protocol are
/// cleared (serial sessions keep no host/port; network sessions keep no serial
/// settings) so stored records are always self-consistent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidSession {
    pub folder_id: Option<i64>,
    pub name: String,
    pub protocol: Protocol,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub serial: Option<SerialSettings>,
    pub ssh: Option<SshOptions>,
    pub credential_profile_id: Option<i64>,
    pub auto_commands: Vec<String>,
    pub env_tag: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
}

const MAX_AUTO_COMMANDS: usize = 20;
const MAX_AUTO_COMMAND_CHARS: usize = 512;

/// Post-connect commands: trimmed, blank lines dropped, no control characters.
fn auto_commands(raw: &[String]) -> Result<Vec<String>> {
    let cmds: Vec<String> = raw
        .iter()
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty())
        .collect();
    if cmds.len() > MAX_AUTO_COMMANDS {
        return Err(AppError::invalid(format!(
            "at most {MAX_AUTO_COMMANDS} auto commands"
        )));
    }
    if cmds
        .iter()
        .any(|c| c.chars().count() > MAX_AUTO_COMMAND_CHARS || c.chars().any(char::is_control))
    {
        return Err(AppError::invalid(
            "auto command is too long or has control characters",
        ));
    }
    Ok(cmds)
}

/// Most forwards a session may start (mirrors the editor limit).
pub const MAX_FORWARDS: usize = 16;

/// SSH options only for SSH sessions; blank key paths are dropped, jump host /
/// proxy / forwards are checked (they can also arrive through import).
fn ssh(input: &SessionInput) -> Result<Option<SshOptions>> {
    if input.protocol != Protocol::Ssh {
        return Ok(None);
    }
    let mut o = input.ssh.clone().unwrap_or_default();
    o.key_path = o
        .key_path
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty());
    if let Some(j) = o.jump.as_mut() {
        j.host = host(Some(&j.host))?;
        j.username = optional_text(j.username.as_deref(), "jump user", MAX_USERNAME_CHARS)?;
        if j.port == Some(0) {
            return Err(AppError::invalid("jump port must be 1-65535"));
        }
    }
    if let Some(p) = o.proxy.as_mut() {
        p.host = host(Some(&p.host))?;
        if p.port == 0 {
            return Err(AppError::invalid("proxy port must be 1-65535"));
        }
    }
    if o.forwards.len() > MAX_FORWARDS {
        return Err(AppError::invalid(format!(
            "at most {MAX_FORWARDS} port forwards"
        )));
    }
    for f in &mut o.forwards {
        if f.listen_port == 0 {
            return Err(AppError::invalid("forward listen port must be 1-65535"));
        }
        if f.kind == ForwardKind::Dynamic {
            f.target_host = None;
            f.target_port = None;
            continue;
        }
        f.target_host = Some(host(f.target_host.as_deref())?);
        if matches!(f.target_port, None | Some(0)) {
            return Err(AppError::invalid("forward target port must be 1-65535"));
        }
    }
    Ok(Some(o))
}

pub(super) fn session(input: &SessionInput) -> Result<ValidSession> {
    let name = name(&input.name, "session")?;
    let (host, port, serial) = match input.protocol {
        Protocol::Serial => (None, None, Some(serial(input.serial.as_ref())?)),
        Protocol::Ssh | Protocol::Telnet | Protocol::Rlogin | Protocol::Raw => {
            if input.port == Some(0) {
                return Err(AppError::invalid("port must be 1-65535"));
            }
            (Some(host(input.host.as_deref())?), input.port, None)
        }
    };
    Ok(ValidSession {
        folder_id: input.folder_id,
        name,
        protocol: input.protocol,
        host,
        port,
        username: optional_text(input.username.as_deref(), "username", MAX_USERNAME_CHARS)?,
        serial,
        ssh: ssh(input)?,
        credential_profile_id: input.credential_profile_id,
        auto_commands: auto_commands(&input.auto_commands)?,
        env_tag: env_tag(input.env_tag.as_deref())?,
        tags: tags(&input.tags)?,
        favorite: input.favorite,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn ssh() -> SessionInput {
        SessionInput {
            folder_id: None,
            name: "  core-sw1  ".into(),
            protocol: Protocol::Ssh,
            host: Some(" 10.0.0.1 ".into()),
            port: Some(22),
            username: Some("  admin ".into()),
            serial: Some(SerialSettings::default()),
            env_tag: Some(" PROD ".into()),
            tags: vec![
                " core ".into(),
                "Core".into(),
                "".into(),
                "   ".into(),
                "dc1".into(),
            ],
            favorite: true,
            ssh: None,
            credential_profile_id: None,
            auto_commands: vec![],
        }
    }

    fn serial_input() -> SessionInput {
        SessionInput {
            protocol: Protocol::Serial,
            host: Some("ignored".into()),
            port: Some(23),
            serial: Some(SerialSettings {
                port: " COM4 ".into(),
                baud: Some(9600),
                ..SerialSettings::default()
            }),
            ..ssh()
        }
    }

    fn invalid(input: &SessionInput) -> String {
        let e = session(input).unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid, "{e}");
        e.detail.unwrap()
    }

    #[test]
    fn ssh_tunnels_are_checked() {
        use crate::model::{Forward, JumpHost, ProxyConfig, ProxyKind};
        let with = |f: &dyn Fn(&mut SshOptions)| {
            let mut i = ssh();
            let mut o = SshOptions::default();
            f(&mut o);
            i.ssh = Some(o);
            session(&i)
        };
        let fwd = |kind, listen_port, port: Option<u16>| Forward {
            kind,
            listen_port,
            target_host: port.map(|_| " 10.0.0.9 ".into()),
            target_port: port,
        };
        let ok = with(&|o| {
            o.jump = Some(JumpHost {
                host: " bastion ".into(),
                port: Some(2222),
                username: Some(" ops ".into()),
            });
            o.proxy = Some(ProxyConfig {
                kind: ProxyKind::Socks5,
                host: "127.0.0.1".into(),
                port: 1080,
            });
            o.forwards = vec![
                fwd(ForwardKind::Local, 8080, Some(80)),
                Forward {
                    target_host: Some("junk".into()),
                    ..fwd(ForwardKind::Dynamic, 1080, None)
                },
            ];
        })
        .unwrap()
        .ssh
        .unwrap();
        let jump = ok.jump.unwrap();
        assert_eq!(
            (jump.host.as_str(), jump.username.as_deref()),
            ("bastion", Some("ops"))
        );
        assert_eq!(ok.forwards[0].target_host.as_deref(), Some("10.0.0.9"));
        assert_eq!(ok.forwards[1].target_host, None, "dynamic has no target");

        let bad: [&dyn Fn(&mut SshOptions); 5] = [
            &|o| {
                o.jump = Some(JumpHost {
                    host: " ".into(),
                    port: None,
                    username: None,
                })
            },
            &|o| {
                o.proxy = Some(ProxyConfig {
                    kind: ProxyKind::Http,
                    host: "p".into(),
                    port: 0,
                })
            },
            &|o| o.forwards = vec![fwd(ForwardKind::Local, 0, Some(80))],
            &|o| o.forwards = vec![fwd(ForwardKind::Remote, 9000, None)],
            &|o| o.forwards = vec![fwd(ForwardKind::Dynamic, 1, None); MAX_FORWARDS + 1],
        ];
        for (n, f) in bad.iter().enumerate() {
            let err = with(*f).unwrap_err();
            assert_eq!(err.code, ErrorCode::Invalid, "case {n}");
        }
    }

    #[test]
    fn ssh_input_is_normalized() {
        let v = session(&ssh()).unwrap();
        assert_eq!(v.name, "core-sw1");
        assert_eq!(v.host.as_deref(), Some("10.0.0.1"));
        assert_eq!(v.port, Some(22));
        assert_eq!(v.username.as_deref(), Some("admin"));
        assert_eq!(v.serial, None, "network sessions drop serial settings");
        assert_eq!(v.env_tag.as_deref(), Some("prod"));
        assert_eq!(v.tags, vec!["core", "dc1"]);
        assert!(v.favorite);
    }

    #[test]
    fn serial_input_is_normalized() {
        let v = session(&serial_input()).unwrap();
        assert_eq!(v.host, None);
        assert_eq!(v.port, None);
        let s = v.serial.unwrap();
        assert_eq!(s.port, "COM4");
        assert_eq!(s.baud, Some(9600));
    }

    #[test]
    fn name_rules() {
        for bad in ["", "   ", "a\nb", "tab\there"] {
            assert!(name(bad, "session").is_err(), "{bad:?}");
        }
        assert_eq!(
            name(&"é".repeat(200), "folder").unwrap().chars().count(),
            200
        );
        assert!(name(&"x".repeat(201), "folder").is_err());
        assert_eq!(name("  Lab  ", "folder").unwrap(), "Lab");
    }

    #[test]
    fn serial_requires_settings_and_port() {
        let mut i = serial_input();
        i.serial = None;
        assert!(invalid(&i).contains("serial settings"));
        let mut i = serial_input();
        i.serial.as_mut().unwrap().port = "   ".into();
        assert!(invalid(&i).contains("serial port"));
    }

    #[test]
    fn serial_line_parameter_ranges() {
        let with = |f: &dyn Fn(&mut SerialSettings)| {
            let mut i = serial_input();
            f(i.serial.as_mut().unwrap());
            session(&i)
        };
        assert!(with(&|s| s.baud = None).is_ok(), "None = autobaud");
        assert!(with(&|s| s.baud = Some(50)).is_ok());
        assert!(with(&|s| s.baud = Some(4_000_000)).is_ok());
        assert!(with(&|s| s.baud = Some(49)).is_err());
        assert!(with(&|s| s.baud = Some(4_000_001)).is_err());
        assert!(with(&|s| s.baud = Some(0)).is_err());
        assert!(with(&|s| s.data_bits = 5).is_ok());
        assert!(with(&|s| s.data_bits = 4).is_err());
        assert!(with(&|s| s.data_bits = 9).is_err());
        assert!(with(&|s| s.stop_bits = 2).is_ok());
        assert!(with(&|s| s.stop_bits = 0).is_err());
        assert!(with(&|s| s.stop_bits = 3).is_err());
    }

    #[test]
    fn network_protocols_require_host() {
        for protocol in [
            Protocol::Ssh,
            Protocol::Telnet,
            Protocol::Rlogin,
            Protocol::Raw,
        ] {
            let mut i = ssh();
            i.protocol = protocol;
            i.host = None;
            assert!(invalid(&i).contains("host"), "{protocol:?}");
            i.host = Some("  ".into());
            assert!(invalid(&i).contains("host"), "{protocol:?}");
        }
        let mut i = ssh();
        i.host = Some("bad host".into());
        assert!(invalid(&i).contains("whitespace"));
    }

    #[test]
    fn port_is_optional_but_nonzero() {
        let mut i = ssh();
        i.port = None;
        assert_eq!(session(&i).unwrap().port, None);
        i.port = Some(65535);
        assert!(session(&i).is_ok());
        i.port = Some(0);
        assert!(invalid(&i).contains("port"));
    }

    #[test]
    fn empty_optional_text_becomes_none() {
        let mut i = ssh();
        i.username = Some("  ".into());
        i.env_tag = Some("".into());
        let v = session(&i).unwrap();
        assert_eq!(v.username, None);
        assert_eq!(v.env_tag, None);
    }

    #[test]
    fn tag_limits() {
        let mut i = ssh();
        i.tags = vec!["x".repeat(65)];
        assert!(invalid(&i).contains("tag"));
        i.tags = (0..51).map(|n| format!("t{n}")).collect();
        assert!(invalid(&i).contains("tags"));
        // Duplicates do not count towards the limit.
        i.tags = (0..200).map(|n| format!("t{}", n % 10)).collect();
        assert_eq!(session(&i).unwrap().tags.len(), 10);
    }

    #[test]
    fn env_tag_limit() {
        let mut i = ssh();
        i.env_tag = Some("x".repeat(33));
        assert!(invalid(&i).contains("env tag"));
    }

    #[test]
    fn setting_key_rules() {
        assert_eq!(setting_key("ui.theme").unwrap(), "ui.theme");
        assert!(setting_key("").is_err());
        assert!(setting_key(" theme").is_err());
        assert!(setting_key("a\nb").is_err());
        assert!(setting_key(&"k".repeat(129)).is_err());
    }

    #[test]
    fn auto_commands_are_trimmed_and_bounded() {
        let ok =
            auto_commands(&[" terminal length 0 ".into(), "".into(), "show clock".into()]).unwrap();
        assert_eq!(ok, ["terminal length 0", "show clock"]);
        assert!(auto_commands(&vec!["x".to_string(); 21]).is_err());
        assert!(auto_commands(&["a\u{1b}[2J".into()]).is_err());
        assert!(auto_commands(&["y".repeat(513)]).is_err());
    }
}
