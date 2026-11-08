//! Serial port access on Linux (P5-T12): devices like `/dev/ttyUSB0` belong to a group
//! (`dialout` on Debian/Ubuntu, `uucp` on Arch/Fedora); a user outside it cannot open
//! them. The app explains this once instead of failing with "access denied". Other OSes
//! need nothing (always ok).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialAccess {
    pub ok: bool,
    /// Group that owns the serial devices, when the user is not in it.
    pub group: Option<String>,
}

// Used by the Linux check; the parsers are tested on every OS.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Group name for `gid` from `/etc/group` text (`name:x:gid:members`).
pub fn group_name(etc_group: &str, gid: u32) -> Option<String> {
    etc_group.lines().find_map(|l| {
        let mut f = l.split(':');
        let name = f.next()?;
        let _pw = f.next()?;
        (f.next()?.parse::<u32>().ok()? == gid).then(|| name.to_owned())
    })
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
/// Group ids from `id -G` output.
pub fn parse_ids(out: &str) -> Vec<u32> {
    out.split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect()
}

#[cfg(target_os = "linux")]
pub fn check() -> SerialAccess {
    use std::os::unix::fs::MetadataExt;
    // The group of an existing serial device, else the usual one of the distribution.
    let devices = ["/dev/ttyUSB0", "/dev/ttyACM0", "/dev/ttyS0"];
    let gid = devices
        .iter()
        .find_map(|d| std::fs::metadata(d).ok().map(|m| m.gid()));
    let etc = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let group = gid.and_then(|g| group_name(&etc, g)).or_else(|| {
        ["dialout", "uucp"]
            .iter()
            .find(|n| etc.lines().any(|l| l.starts_with(&format!("{n}:"))))
            .map(|n| (*n).to_owned())
    });
    let Some(group) = group else {
        return SerialAccess {
            ok: true,
            group: None,
        };
    };
    let gid = gid.or_else(|| {
        etc.lines().find_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            (f.first() == Some(&group.as_str())).then(|| f.get(2)?.parse().ok())?
        })
    });
    let ids = std::process::Command::new("id")
        .arg("-G")
        .output()
        .ok()
        .map(|o| parse_ids(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    // Unknown (no `id`): do not nag.
    let ok = ids.is_empty() || gid.is_none_or(|g| ids.contains(&g));
    SerialAccess {
        ok,
        group: (!ok).then_some(group),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn check() -> SerialAccess {
    SerialAccess {
        ok: true,
        group: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups() {
        let etc = "root:x:0:\ndialout:x:20:alice\nuucp:x:14:\n";
        assert_eq!(group_name(etc, 20).as_deref(), Some("dialout"));
        assert_eq!(group_name(etc, 14).as_deref(), Some("uucp"));
        assert_eq!(group_name(etc, 99), None);
        assert_eq!(parse_ids("1000 4 20 27\n"), vec![1000, 4, 20, 27]);
        assert!(check().ok || check().group.is_some());
    }
}
