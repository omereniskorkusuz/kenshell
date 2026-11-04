//! CSV import/export of sessions. Header-based, so column order does not matter.
//! Columns: `name,protocol,host,port,username,folder,env,tags,serial_port,baud`
//! (`folder` = `A/B` path, `tags` separated by `;`). Never contains secrets.

use crate::model::{Folder, Protocol, SerialSettings, SessionRecord};

use super::{Imported, blank_input};

pub const HEADER: [&str; 10] = [
    "name",
    "protocol",
    "host",
    "port",
    "username",
    "folder",
    "env",
    "tags",
    "serial_port",
    "baud",
];

/// RFC 4180-ish: quotes, doubled quotes, commas and newlines inside quotes; `;` is
/// also accepted as the separator (Excel in Turkish locale saves CSV with `;`).
pub fn parse_rows(text: &str) -> Vec<Vec<String>> {
    let first_line = text.lines().next().unwrap_or("");
    let sep = if first_line.matches(';').count() > first_line.matches(',').count() {
        ';'
    } else {
        ','
    };
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') if field.is_empty() => quoted = true,
            (false, c) if c == sep => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.trim().is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        if row.iter().any(|f| !f.trim().is_empty()) {
            rows.push(row);
        }
    }
    rows
}

/// Parsed sessions + per-row errors (`row N: reason`).
pub fn import(text: &str) -> (Vec<Imported>, Vec<String>) {
    let rows = parse_rows(text);
    let Some((header, body)) = rows.split_first() else {
        return (vec![], vec!["empty file".into()]);
    };
    let idx = |name: &str| {
        header
            .iter()
            .position(|h| h.trim().eq_ignore_ascii_case(name))
    };
    let Some(name_col) = idx("name") else {
        return (vec![], vec!["missing column: name".into()]);
    };
    let col = |row: &[String], name: &str| -> Option<String> {
        idx(name)
            .and_then(|i| row.get(i))
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (n, row) in body.iter().enumerate() {
        let line = n + 2;
        let Some(name) = row
            .get(name_col)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        else {
            errors.push(format!("row {line}: no name"));
            continue;
        };
        let protocol = match col(row, "protocol")
            .map(|p| p.to_ascii_lowercase())
            .as_deref()
        {
            None | Some("ssh") => Protocol::Ssh,
            Some("telnet") => Protocol::Telnet,
            Some("rlogin") => Protocol::Rlogin,
            Some("raw") => Protocol::Raw,
            Some("serial") => Protocol::Serial,
            Some(other) => {
                errors.push(format!("row {line}: unknown protocol `{other}`"));
                continue;
            }
        };
        let mut input = blank_input(name, protocol);
        if protocol == Protocol::Serial {
            let Some(port) = col(row, "serial_port") else {
                errors.push(format!("row {line}: serial_port missing"));
                continue;
            };
            input.serial = Some(SerialSettings {
                port,
                baud: col(row, "baud").and_then(|b| b.parse().ok()),
                ..SerialSettings::default()
            });
        } else {
            input.host = col(row, "host");
            input.port = match col(row, "port") {
                None => None,
                Some(p) => match p.parse::<u16>() {
                    Ok(p) => Some(p),
                    Err(_) => {
                        errors.push(format!("row {line}: bad port `{p}`"));
                        continue;
                    }
                },
            };
            input.username = col(row, "username");
        }
        input.env_tag = col(row, "env");
        input.tags = col(row, "tags")
            .map(|t| {
                t.split(';')
                    .map(|x| x.trim().to_owned())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        let folders = col(row, "folder")
            .map(|f| {
                f.split(['/', '\\'])
                    .map(|p| p.trim().to_owned())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        out.push(Imported { folders, input });
    }
    (out, errors)
}

fn quote(v: &str) -> String {
    if v.contains([',', '"', '\n', '\r', ';']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_owned()
    }
}

/// `A/B` path of a folder.
pub fn folder_path(folders: &[Folder], mut id: Option<i64>) -> String {
    let mut parts = Vec::new();
    let mut guard = 0;
    while let Some(fid) = id {
        guard += 1;
        let Some(f) = folders.iter().find(|f| f.id == fid) else {
            break;
        };
        if guard > 64 {
            break;
        }
        parts.push(f.name.clone());
        id = f.parent_id;
    }
    parts.reverse();
    parts.join("/")
}

pub fn export(folders: &[Folder], sessions: &[SessionRecord]) -> String {
    let mut out = HEADER.join(",");
    out.push_str("\r\n");
    for s in sessions {
        let proto = match s.protocol {
            Protocol::Serial => "serial",
            Protocol::Ssh => "ssh",
            Protocol::Telnet => "telnet",
            Protocol::Rlogin => "rlogin",
            Protocol::Raw => "raw",
        };
        let cells = [
            s.name.clone(),
            proto.into(),
            s.host.clone().unwrap_or_default(),
            s.port.map(|p| p.to_string()).unwrap_or_default(),
            s.username.clone().unwrap_or_default(),
            folder_path(folders, s.folder_id),
            s.env_tag.clone().unwrap_or_default(),
            s.tags.join(";"),
            s.serial
                .as_ref()
                .map(|x| x.port.clone())
                .unwrap_or_default(),
            s.serial
                .as_ref()
                .and_then(|x| x.baud)
                .map(|b| b.to_string())
                .unwrap_or_default(),
        ];
        out.push_str(&cells.iter().map(|c| quote(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_mixed_rows_with_quotes_and_errors() {
        let text = "\u{feff}name,protocol,host,port,username,folder,env,tags,serial_port,baud\r\n\
core-1,ssh,10.0.0.1,,admin,DC1/Core,prod,core;dc1,,\r\n\
\"fw, edge\",telnet,10.0.0.5,2001,,DC1,,,,\r\n\
console,serial,,,,,lab,,COM4,9600\r\n\
bad,ssh,10.0.0.9,99999,,,,,,\r\n\
,ssh,10.0.0.10,,,,,,,\r\n\
odd,carrier-pigeon,x,,,,,,,\r\n";
        let (ok, errors) = import(text);
        assert_eq!(ok.len(), 3);
        assert_eq!(ok[0].folders, vec!["DC1", "Core"]);
        assert_eq!(ok[0].input.tags, vec!["core", "dc1"]);
        assert_eq!(ok[1].input.name, "fw, edge");
        assert_eq!(ok[1].input.port, Some(2001));
        assert_eq!(ok[2].input.serial.as_ref().unwrap().baud, Some(9600));
        assert_eq!(errors.len(), 3, "{errors:?}");
    }

    #[test]
    fn semicolon_separated_excel_csv() {
        let (ok, errors) = import("name;host;protocol\nsw1;10.0.0.1;ssh\n");
        assert!(errors.is_empty());
        assert_eq!(ok[0].input.host.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn export_round_trips() {
        let folders = vec![Folder {
            id: 1,
            parent_id: None,
            name: "DC1".into(),
            sort_order: 0,
        }];
        let (parsed, _) = import(
            "name,protocol,host,port,username,folder,env,tags\n\"a,b\",ssh,h,2222,u,DC1,prod,x;y\n",
        );
        let i = &parsed[0].input;
        let rec = SessionRecord {
            id: 1,
            folder_id: Some(1),
            name: i.name.clone(),
            protocol: i.protocol,
            host: i.host.clone(),
            port: i.port,
            username: i.username.clone(),
            serial: None,
            env_tag: i.env_tag.clone(),
            tags: i.tags.clone(),
            favorite: false,
            ssh: None,
            credential_profile_id: None,
            sort_order: 0,
            created_at: 0,
            updated_at: 0,
            last_connected_at: None,
        };
        let text = export(&folders, &[rec]);
        let (again, errors) = import(&text);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(again[0].input, parsed[0].input);
        assert_eq!(again[0].folders, vec!["DC1"]);
    }
}
