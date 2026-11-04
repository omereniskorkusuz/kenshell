//! Session import / export (P2-T13). Parsers are pure (text in → sessions out) and
//! unit-tested with realistic samples; reading files/registry is a thin layer on top.
//!
//! * PuTTY: Windows registry (`reg query …\PuTTY\Sessions /s`) or `~/.putty/sessions/*`
//! * SecureCRT: `Config/Sessions/**/*.ini` (folders = sub-directories)
//! * MobaXterm: `MobaXterm.ini` `[Bookmarks*]` (format undocumented → best effort)
//! * CSV: `name,protocol,host,port,username,folder,env,tags,serial_port,baud`
//!
//! Exports never contain secrets (the DB has none).

pub mod csv;

use std::collections::HashMap;

use serde::Serialize;

use crate::model::{FlowControl, Parity, Protocol, SerialSettings, SessionInput};

/// One parsed session plus the folder path it should live in (relative to the
/// import's root folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub folders: Vec<String>,
    pub input: SessionInput,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub created: usize,
    /// Already present (same name in the same folder) — left untouched.
    pub skipped: usize,
    /// Entries that could not be imported (short reason, never secrets).
    pub errors: Vec<String>,
    /// Root folder that received the sessions.
    pub folder: Option<String>,
}

pub(crate) fn blank_input(name: &str, protocol: Protocol) -> SessionInput {
    SessionInput {
        folder_id: None,
        name: name.trim().to_owned(),
        protocol,
        host: None,
        port: None,
        username: None,
        serial: None,
        env_tag: None,
        tags: vec![],
        favorite: false,
        ssh: None,
        credential_profile_id: None,
    }
}

fn opt(s: Option<&String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// `user@host` → (Some(user), host)
fn split_user(host: &str) -> (Option<String>, String) {
    match host.rsplit_once('@') {
        Some((u, h)) if !u.is_empty() => (Some(u.to_owned()), h.to_owned()),
        _ => (None, host.to_owned()),
    }
}

// ---------------------------------------------------------------- PuTTY

/// PuTTY stores session names percent-encoded (`core%20sw`).
pub fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(&h), Some(&l)) = (bytes.get(i + 1), bytes.get(i + 2))
            && let (Some(h), Some(l)) = (hex(h), hex(l))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Registry values are shown as `0x16` (DWORD) or decimal in session files.
fn putty_num(v: Option<&String>) -> Option<u32> {
    let v = v?.trim();
    match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => v.parse().ok(),
    }
}

/// One PuTTY session from its key/value map. `None` for "Default Settings" and
/// protocols we do not support.
pub fn putty_session(raw_name: &str, kv: &HashMap<String, String>) -> Option<Imported> {
    let name = percent_decode(raw_name);
    if name.trim().is_empty() || name == "Default Settings" {
        return None;
    }
    let protocol = match kv.get("Protocol").map(|p| p.trim().to_ascii_lowercase()) {
        Some(p) if p == "ssh" => Protocol::Ssh,
        Some(p) if p == "telnet" => Protocol::Telnet,
        Some(p) if p == "rlogin" => Protocol::Rlogin,
        Some(p) if p == "raw" => Protocol::Raw,
        Some(p) if p == "serial" => Protocol::Serial,
        None => Protocol::Ssh,
        Some(_) => return None,
    };
    let mut input = blank_input(&name, protocol);
    if protocol == Protocol::Serial {
        let port = opt(kv.get("SerialLine"))?;
        input.serial = Some(SerialSettings {
            port,
            baud: putty_num(kv.get("SerialSpeed")).filter(|b| *b > 0),
            data_bits: putty_num(kv.get("SerialDataBits"))
                .and_then(|b| u8::try_from(b).ok())
                .filter(|b| (5..=8).contains(b))
                .unwrap_or(8),
            // PuTTY stores stop bits in halves: 2 = 1, 3 = 1.5, 4 = 2.
            stop_bits: if putty_num(kv.get("SerialStopHalfbits")) == Some(4) {
                2
            } else {
                1
            },
            parity: match putty_num(kv.get("SerialParity")) {
                Some(1) => Parity::Odd,
                Some(2) => Parity::Even,
                _ => Parity::None,
            },
            flow_control: match putty_num(kv.get("SerialFlowControl")) {
                Some(1) => FlowControl::XonXoff,
                Some(2) => FlowControl::RtsCts,
                _ => FlowControl::None,
            },
            dtr: None,
            rts: None,
        });
    } else {
        let (user_in_host, host) = split_user(&opt(kv.get("HostName"))?);
        input.host = Some(host);
        input.username = opt(kv.get("UserName")).or(user_in_host);
        let port = putty_num(kv.get("PortNumber")).and_then(|p| u16::try_from(p).ok());
        input.port = port.filter(|p| Some(*p) != protocol.default_port() && *p > 0);
    }
    Some(Imported {
        folders: vec![],
        input,
    })
}

/// Parses `reg query HKCU\Software\SimonTatham\PuTTY\Sessions /s` output.
#[cfg_attr(not(windows), allow(dead_code, reason = "used on Windows; tested everywhere"))]
pub fn parse_reg_query(text: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut out: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("HKEY_") {
            if let Some((_, name)) = trimmed.rsplit_once("\\Sessions\\") {
                out.push((name.to_owned(), HashMap::new()));
            }
            continue;
        }
        let Some((_, map)) = out.last_mut() else {
            continue;
        };
        // "    Name    REG_TYPE    value" (value may contain spaces, or be empty)
        let t = trimmed.trim_start();
        let mut parts = t.splitn(2, "    ");
        let (Some(key), Some(rest)) = (parts.next(), parts.next()) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix("REG_").map(|r| {
            r.split_once("    ")
                .map(|(_, v)| v.to_owned())
                .unwrap_or_default()
        }) else {
            continue;
        };
        map.insert(key.to_owned(), rest);
    }
    out
}

/// One `~/.putty/sessions/<name>` file (`Key=Value` lines).
pub fn parse_putty_file(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.to_owned()))
        .collect()
}

// ---------------------------------------------------------------- SecureCRT

/// One SecureCRT session `.ini`. `folders` = sub-directories below `Sessions`.
pub fn securecrt_session(name: &str, folders: Vec<String>, text: &str) -> Option<Imported> {
    if name.eq_ignore_ascii_case("Default") || name.starts_with("__") {
        return None;
    }
    let mut s: HashMap<String, String> = HashMap::new();
    let mut d: HashMap<String, u32> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let (kind, rest) = match line.split_at_checked(2) {
            Some((k @ ("S:" | "D:"), r)) => (k, r),
            _ => continue,
        };
        let Some((key, value)) = rest.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"').to_owned();
        if kind == "S:" {
            s.insert(key, value.to_owned());
        } else if let Ok(v) = u32::from_str_radix(value.trim(), 16) {
            d.insert(key, v);
        }
    }
    let proto = s
        .get("Protocol Name")
        .map(|p| p.trim().to_ascii_lowercase());
    let protocol = match proto.as_deref() {
        Some("ssh2" | "ssh1") | None => Protocol::Ssh,
        Some("telnet" | "telnet/ssl") => Protocol::Telnet,
        Some("rlogin") => Protocol::Rlogin,
        Some("raw") => Protocol::Raw,
        Some("serial") => Protocol::Serial,
        Some(_) => return None,
    };
    let mut input = blank_input(name, protocol);
    if protocol == Protocol::Serial {
        let port = opt(s.get("Com Port"))?;
        input.serial = Some(SerialSettings {
            port,
            baud: d.get("Baud Rate").copied().filter(|b| *b > 0),
            data_bits: d
                .get("Data Bits")
                .and_then(|b| u8::try_from(*b).ok())
                .filter(|b| (5..=8).contains(b))
                .unwrap_or(8),
            parity: match d.get("Parity") {
                Some(1) => Parity::Odd,
                Some(2) => Parity::Even,
                _ => Parity::None,
            },
            stop_bits: if d.get("Stop Bits") == Some(&2) { 2 } else { 1 },
            flow_control: if d.get("RTS Flow") == Some(&1) || d.get("CTS Flow") == Some(&1) {
                FlowControl::RtsCts
            } else if d.get("XON Flow") == Some(&1) {
                FlowControl::XonXoff
            } else {
                FlowControl::None
            },
            dtr: None,
            rts: None,
        });
    } else {
        input.host = Some(opt(s.get("Hostname"))?);
        input.username = opt(s.get("Username"));
        let port = match protocol {
            Protocol::Ssh => d.get("[SSH2] Port").or_else(|| d.get("[SSH1] Port")),
            _ => d.get("Port"),
        }
        .and_then(|p| u16::try_from(*p).ok());
        input.port = port.filter(|p| Some(*p) != protocol.default_port() && *p > 0);
    }
    Some(Imported { folders, input })
}

// ---------------------------------------------------------------- MobaXterm

/// `MobaXterm.ini`: `[Bookmarks]`, `[Bookmarks_1]` … sections with `SubRep=Folder\Sub`
/// and `Name=#icon#type%host%port%user%…` entries. Type 0 SSH, 1 Telnet, 2 Rlogin,
/// 8 Serial (others — RDP, VNC, FTP … — are skipped). Undocumented → best effort.
pub fn parse_mobaxterm(text: &str) -> Vec<Imported> {
    let mut out = Vec::new();
    let mut in_bookmarks = false;
    let mut folders: Vec<String> = vec![];
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_bookmarks = line.to_ascii_lowercase().starts_with("[bookmarks");
            folders.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key == "SubRep" {
            folders = value
                .split('\\')
                .filter(|p| !p.trim().is_empty())
                .map(|p| p.trim().to_owned())
                .collect();
            continue;
        }
        if key == "ImgNum" || !value.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = value.split('%').collect();
        let head: Vec<&str> = fields[0].split('#').collect(); // ["", icon, type]
        let Some(kind) = head.get(2).and_then(|t| t.parse::<u32>().ok()) else {
            continue;
        };
        let field = |i: usize| fields.get(i).map(|f| f.trim()).filter(|f| !f.is_empty());
        let protocol = match kind {
            0 => Protocol::Ssh,
            1 => Protocol::Telnet,
            2 => Protocol::Rlogin,
            8 => Protocol::Serial,
            _ => continue,
        };
        let mut input = blank_input(key, protocol);
        if protocol == Protocol::Serial {
            // "COM4  (Prolific USB-to-Serial Comm Port (COM4))" → "COM4"
            let Some(port) = field(1).and_then(|p| p.split_whitespace().next()) else {
                continue;
            };
            let baud = field(2)
                .and_then(|b| b.parse::<u32>().ok())
                .filter(|b| (300..=4_000_000).contains(b));
            input.serial = Some(SerialSettings {
                port: port.to_owned(),
                baud,
                ..SerialSettings::default()
            });
        } else {
            let Some(host) = field(1) else { continue };
            input.host = Some(host.to_owned());
            input.port = field(2)
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|p| Some(*p) != protocol.default_port() && *p > 0);
            input.username = field(3).map(str::to_owned);
        }
        out.push(Imported {
            folders: folders.clone(),
            input,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn putty_registry_dump() {
        let text = r"
HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings
    HostName    REG_SZ

HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\core%20sw%2001
    HostName    REG_SZ    admin@10.0.0.1
    PortNumber    REG_DWORD    0x16
    Protocol    REG_SZ    ssh
    UserName    REG_SZ

HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\rack-term
    HostName    REG_SZ    10.0.0.5
    PortNumber    REG_DWORD    0x7d1
    Protocol    REG_SZ    telnet

HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\console
    Protocol    REG_SZ    serial
    SerialLine    REG_SZ    COM4
    SerialSpeed    REG_DWORD    0x2580
    SerialStopHalfbits    REG_DWORD    0x2
    SerialParity    REG_DWORD    0x0
    SerialFlowControl    REG_DWORD    0x1
";
        let entries = parse_reg_query(text);
        assert_eq!(entries.len(), 4);
        let sessions: Vec<_> = entries
            .iter()
            .filter_map(|(n, kv)| putty_session(n, kv))
            .collect();
        assert_eq!(sessions.len(), 3, "Default Settings skipped");
        let core = &sessions[0].input;
        assert_eq!(core.name, "core sw 01");
        assert_eq!(core.host.as_deref(), Some("10.0.0.1"));
        assert_eq!(core.username.as_deref(), Some("admin"));
        assert_eq!(core.port, None, "default 22 is implicit");
        let rack = &sessions[1].input;
        assert_eq!((rack.protocol, rack.port), (Protocol::Telnet, Some(2001)));
        let s = sessions[2].input.serial.as_ref().unwrap();
        assert_eq!((s.port.as_str(), s.baud), ("COM4", Some(9600)));
        assert_eq!(s.flow_control, FlowControl::XonXoff);
    }

    #[test]
    fn putty_unix_file() {
        let kv =
            parse_putty_file("HostName=fw-edge\nPortNumber=2222\nProtocol=ssh\nUserName=root\n");
        let s = putty_session("fw-edge", &kv).unwrap().input;
        assert_eq!((s.port, s.username.as_deref()), (Some(2222), Some("root")));
    }

    #[test]
    fn securecrt_ini() {
        let ssh = r#"S:"Protocol Name"=SSH2
S:"Hostname"=10.10.10.1
S:"Username"=netadmin
D:"[SSH2] Port"=00000016
"#;
        let s = securecrt_session("core-1", vec!["DC1".into()], ssh).unwrap();
        assert_eq!(s.folders, vec!["DC1"]);
        assert_eq!(s.input.host.as_deref(), Some("10.10.10.1"));
        assert_eq!(s.input.port, None);
        let serial = r#"S:"Protocol Name"=Serial
S:"Com Port"=COM3
D:"Baud Rate"=0001c200
D:"Data Bits"=00000008
D:"Parity"=00000000
D:"Stop Bits"=00000000
"#;
        let s = securecrt_session("console", vec![], serial).unwrap().input;
        assert_eq!(s.serial.unwrap().baud, Some(115_200));
        let telnet = "S:\"Protocol Name\"=Telnet\nS:\"Hostname\"=ts1\nD:\"Port\"=000007d2\n";
        assert_eq!(
            securecrt_session("ts", vec![], telnet).unwrap().input.port,
            Some(2002)
        );
        assert!(securecrt_session("Default", vec![], ssh).is_none());
        assert!(
            securecrt_session("x", vec![], "S:\"Protocol Name\"=SSH2\n").is_none(),
            "no host"
        );
    }

    #[test]
    fn mobaxterm_bookmarks() {
        let ini = r"[Misc]
Foo=bar
[Bookmarks]
SubRep=
ImgNum=42
core-sw=#109#0%10.0.0.1%22%admin%%-1%-1%%%22%%0%0%0%%%-1%0%0%0%%1080%%0%0%1
[Bookmarks_1]
SubRep=Sites\Izmir
ImgNum=41
izmir-fw=#109#0%192.168.10.1%2222%root%%-1%-1
term=#98#1%10.0.0.5%2001%%%2%%
console=#131#8%COM4  (Prolific USB-to-Serial Comm Port (COM4))%115200%3%0%0%1%2%
rdp-host=#91#4%10.0.0.9%3389%%
";
        let v = parse_mobaxterm(ini);
        assert_eq!(v.len(), 4, "RDP skipped");
        assert_eq!(v[0].input.username.as_deref(), Some("admin"));
        assert_eq!(v[1].folders, vec!["Sites", "Izmir"]);
        assert_eq!(v[1].input.port, Some(2222));
        assert_eq!(
            (v[2].input.protocol, v[2].input.port),
            (Protocol::Telnet, Some(2001))
        );
        let s = v[3].input.serial.as_ref().unwrap();
        assert_eq!((s.port.as_str(), s.baud), ("COM4", Some(115_200)));
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let junk = "\u{0}%%%\n[Bookmarks]\nx=#\ny=#a#b%\nSubRep=\\\\\\\n";
        let _ = parse_mobaxterm(junk);
        let _ = parse_reg_query(junk);
        let _ = securecrt_session("x", vec![], junk);
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}

// ---------------------------------------------------------------- applying to the DB

use crate::storage::Db;

/// Max size of an import file (protects against picking a huge wrong file).
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

fn find_or_create_folder(db: &Db, parent: Option<i64>, name: &str) -> crate::error::Result<i64> {
    let existing = db
        .list_folders()?
        .into_iter()
        .find(|f| f.parent_id == parent && f.name.eq_ignore_ascii_case(name.trim()));
    match existing {
        Some(f) => Ok(f.id),
        None => Ok(db.create_folder(name, parent)?.id),
    }
}

/// Creates the sessions under a top-level folder `root` (reused if it exists),
/// mirroring their folder paths. Same name in the same folder → skipped.
pub fn apply(db: &Db, root: &str, items: Vec<Imported>, mut report: ImportReport) -> ImportReport {
    if items.is_empty() {
        return report;
    }
    let root_id = match find_or_create_folder(db, None, root) {
        Ok(id) => id,
        Err(e) => {
            report.errors.push(format!("{root}: {e}"));
            return report;
        }
    };
    report.folder = Some(root.to_owned());
    for item in items {
        let mut parent = root_id;
        let mut failed = false;
        for part in &item.folders {
            match find_or_create_folder(db, Some(parent), part) {
                Ok(id) => parent = id,
                Err(e) => {
                    report.errors.push(format!("{}: {e}", item.input.name));
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            continue;
        }
        let exists = db.list_sessions().is_ok_and(|all| {
            all.iter().any(|s| {
                s.folder_id == Some(parent) && s.name.eq_ignore_ascii_case(item.input.name.trim())
            })
        });
        if exists {
            report.skipped += 1;
            continue;
        }
        let mut input = item.input;
        input.folder_id = Some(parent);
        match db.create_session(&input) {
            Ok(_) => report.created += 1,
            Err(e) => report.errors.push(format!("{}: {e}", input.name)),
        }
    }
    report
}

#[cfg(test)]
mod apply_tests {
    use super::*;

    #[test]
    fn apply_mirrors_folders_and_skips_duplicates() {
        let db = Db::open_in_memory().unwrap();
        let (items, _) = csv::import(
            "name,protocol,host,folder\ncore,ssh,10.0.0.1,DC1/Core\nedge,ssh,10.0.0.2,DC1\nbad,ssh,,DC1\n",
        );
        let r = apply(&db, "CSV import", items.clone(), ImportReport::default());
        assert_eq!(
            (r.created, r.skipped, r.errors.len()),
            (2, 0, 1),
            "{:?}",
            r.errors
        );
        let again = apply(&db, "CSV import", items, ImportReport::default());
        assert_eq!((again.created, again.skipped), (0, 2));
        let folders = db.list_folders().unwrap();
        assert_eq!(
            folders.len(),
            3,
            "CSV import / DC1 / Core, reused the second time"
        );
    }
}
