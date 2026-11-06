//! Ping and traceroute from this computer (P5-T05) through the OS tools: they work
//! without privileges everywhere (raw ICMP would not on Windows, nor for traceroute's
//! TTL handling). Only the argument lists differ per OS; they are built for every OS
//! here so all of them are tested.

use std::net::IpAddr;

// Only the running OS's variant is built outside tests; all are tested everywhere.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

#[cfg(target_os = "macos")]
pub const OS: Os = Os::Mac;
#[cfg(target_os = "windows")]
pub const OS: Os = Os::Windows;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub const OS: Os = Os::Linux;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub program: &'static str,
    pub args: Vec<String>,
}

fn v6(target: &str) -> bool {
    matches!(target.parse::<IpAddr>(), Ok(IpAddr::V6(_)))
}

/// `count` None = until stopped. `timeout_ms` per reply.
pub fn ping(os: Os, target: &str, count: Option<u32>, timeout_ms: u32) -> Probe {
    let six = v6(target);
    let mut args: Vec<String> = Vec::new();
    let program = match os {
        Os::Mac => {
            if let Some(c) = count {
                args.extend(["-c".into(), c.to_string()]);
            }
            if !six {
                // macOS: wait time per reply in milliseconds (ping6 has no -W).
                args.extend(["-W".into(), timeout_ms.to_string()]);
            }
            if six { "/sbin/ping6" } else { "/sbin/ping" }
        }
        Os::Linux => {
            if six {
                args.push("-6".into());
            }
            if let Some(c) = count {
                args.extend(["-c".into(), c.to_string()]);
            }
            args.extend(["-W".into(), timeout_ms.div_ceil(1000).max(1).to_string()]);
            "ping"
        }
        Os::Windows => {
            if six {
                args.push("-6".into());
            }
            match count {
                Some(c) => args.extend(["-n".into(), c.to_string()]),
                None => args.push("-t".into()),
            }
            args.extend(["-w".into(), timeout_ms.to_string()]);
            "ping"
        }
    };
    args.push(target.to_owned());
    Probe { program, args }
}

/// Traceroute candidates in order (Linux falls back to `tracepath`, which needs no
/// extra package).
pub fn trace(os: Os, target: &str, max_hops: u32, resolve: bool) -> Vec<Probe> {
    let six = v6(target);
    let hops = max_hops.to_string();
    match os {
        Os::Mac | Os::Linux => {
            let mut args: Vec<String> = Vec::new();
            if !resolve {
                args.push("-n".into());
            }
            args.extend(["-m".into(), hops.clone(), "-w".into(), "2".into()]);
            if six && os == Os::Linux {
                args.insert(0, "-6".into());
            }
            args.push(target.to_owned());
            let traceroute = match (os, six) {
                (Os::Mac, true) => "/usr/sbin/traceroute6",
                (Os::Mac, false) => "/usr/sbin/traceroute",
                _ => "traceroute",
            };
            let mut list = vec![Probe {
                program: traceroute,
                args,
            }];
            if os == Os::Linux {
                let mut args: Vec<String> = Vec::new();
                if !resolve {
                    args.push("-n".into());
                }
                if six {
                    args.push("-6".into());
                }
                args.extend(["-m".into(), hops, target.to_owned()]);
                list.push(Probe {
                    program: "tracepath",
                    args,
                });
            }
            list
        }
        Os::Windows => {
            let mut args: Vec<String> = Vec::new();
            if !resolve {
                args.push("-d".into());
            }
            if six {
                args.push("-6".into());
            }
            args.extend([
                "-h".into(),
                hops,
                "-w".into(),
                "2000".into(),
                target.to_owned(),
            ]);
            vec![Probe {
                program: "tracert",
                args,
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(p: &Probe) -> String {
        format!("{} {}", p.program, p.args.join(" "))
    }

    #[test]
    fn ping_arguments_per_os() {
        assert_eq!(
            line(&ping(Os::Mac, "10.0.0.1", Some(4), 2000)),
            "/sbin/ping -c 4 -W 2000 10.0.0.1"
        );
        assert_eq!(
            line(&ping(Os::Mac, "fe80::1", None, 2000)),
            "/sbin/ping6 fe80::1"
        );
        assert_eq!(
            line(&ping(Os::Linux, "host.lan", Some(10), 1500)),
            "ping -c 10 -W 2 host.lan"
        );
        assert_eq!(
            line(&ping(Os::Windows, "10.0.0.1", None, 2000)),
            "ping -t -w 2000 10.0.0.1"
        );
        assert_eq!(
            line(&ping(Os::Windows, "2001:db8::1", Some(4), 1000)),
            "ping -6 -n 4 -w 1000 2001:db8::1"
        );
    }

    #[test]
    fn trace_arguments_per_os() {
        let mac = trace(Os::Mac, "8.8.8.8", 30, false);
        assert_eq!(line(&mac[0]), "/usr/sbin/traceroute -n -m 30 -w 2 8.8.8.8");
        let linux = trace(Os::Linux, "2001:db8::1", 20, true);
        assert_eq!(line(&linux[0]), "traceroute -6 -m 20 -w 2 2001:db8::1");
        assert_eq!(line(&linux[1]), "tracepath -6 -m 20 2001:db8::1");
        let win = trace(Os::Windows, "8.8.8.8", 30, false);
        assert_eq!(line(&win[0]), "tracert -d -h 30 -w 2000 8.8.8.8");
    }
}
