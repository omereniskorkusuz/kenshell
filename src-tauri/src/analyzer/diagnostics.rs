//! System Diagnostics (P4-T06): a vendor's read-only command set and the rules that turn
//! its output into findings with a severity ("Gi1/0/5: 12 403 CRC errors").
//!
//! The commands are only ever `show` commands (the session refuses anything else,
//! `device::capture::read_only`). Findings carry a code and parameters; the UI words them
//! (`diag.<code>` in TR/EN).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Critical,
    Warning,
    Info,
    Ok,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    pub code: String,
    pub params: BTreeMap<String, String>,
    /// The command whose output it came from.
    pub command: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutput {
    pub command: String,
    /// The output, or None when the command could not run (time-out, busy …).
    pub text: Option<String>,
}

const CISCO: &[&str] = &[
    "show version",
    "show environment all",
    "show processes cpu sorted",
    "show processes memory sorted",
    "show interfaces",
    "show interfaces status err-disabled",
    "show logging",
];

const RUIJIE: &[&str] = &[
    "show version",
    "show power",
    "show fan",
    "show cpu",
    "show memory",
    "show interfaces",
    "show interfaces status",
    "show logging",
];

/// The command set for a vendor family (Cisco-like for unknown devices).
pub fn commands(family: Option<&str>) -> &'static [&'static str] {
    match family {
        Some("ruijie") => RUIJIE,
        _ => CISCO,
    }
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("diagnostics pattern")
}

static UNSUPPORTED: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?m)^\s*%\s*(Invalid input|Unknown command|Incomplete command|Ambiguous command)")
});
static CPU_CISCO: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)CPU utilization for five seconds:\s*(\d+)%.*?one minute:\s*(\d+)%.*?five minutes:\s*(\d+)%",
    )
});
static CPU_RUIJIE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)CPU utilization in five minutes\s*:\s*([\d.]+)%"));
static MEM_CISCO: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)Processor Pool Total:\s*(\d+)\s+Used:\s*(\d+)"));
static MEM_RUIJIE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)Used rate\s*:\s*([\d.]+)%"));
static IFACE_HEAD: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)^(\S+(?: \d\S*)?) is (?:administratively )?(up|down)\b"));
static INPUT_ERR: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+) input errors?,\s*(\d+) CRC"));
static OUTPUT_ERR: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+) output errors?"));
static ENV_PART: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\b(fan|power|psu|ps\d|supply|temp)"));
static ENV_BAD: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(not ok|fail(ed|ure)?|faulty|bad|abnormal|critical|shutdown|not work)")
});
static LOG_SEV: LazyLock<Regex> = LazyLock::new(|| re(r"%[A-Z0-9_]+-([0-3])-[A-Z0-9_]+"));
static UPTIME_CISCO: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)uptime is (.+)"));
static UPTIME_RUIJIE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)System uptime\s*:\s*(\d+):(\d+):(\d+):(\d+)"));
static RELOAD_REASON: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)Last reload reason:\s*(.+)"));

struct Out<'a> {
    list: Vec<Finding>,
    command: &'a str,
}

impl Out<'_> {
    fn add(&mut self, severity: Severity, code: &str, params: &[(&str, String)]) {
        self.list.push(Finding {
            severity,
            code: code.into(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            command: self.command.to_owned(),
        });
    }
}

fn pct_level(v: f64, warn: f64, crit: f64) -> Option<Severity> {
    if v >= crit {
        Some(Severity::Critical)
    } else if v >= warn {
        Some(Severity::Warning)
    } else {
        None
    }
}

/// Findings for the outputs, most severe first.
pub fn evaluate(family: Option<&str>, outputs: &[CommandOutput]) -> Vec<Finding> {
    let mut all = Vec::new();
    for o in outputs {
        let mut out = Out {
            list: Vec::new(),
            command: &o.command,
        };
        let Some(text) = o.text.as_deref() else {
            out.add(
                Severity::Info,
                "commandFailed",
                &[("command", o.command.clone())],
            );
            all.extend(out.list);
            continue;
        };
        let text = &text[..text.len().min(super::MAX_INPUT)];
        if UNSUPPORTED.is_match(text) {
            out.add(
                Severity::Info,
                "commandUnsupported",
                &[("command", o.command.clone())],
            );
            all.extend(out.list);
            continue;
        }
        let cmd = o.command.as_str();
        if cmd == "show version" {
            version(&mut out, text);
        } else if cmd.starts_with("show environment") || cmd == "show power" || cmd == "show fan" {
            // `show power` / `show fan` rows need not name the part themselves.
            environment(&mut out, text, cmd == "show power" || cmd == "show fan");
        } else if cmd.starts_with("show processes cpu") || cmd == "show cpu" {
            cpu(&mut out, text);
        } else if cmd.starts_with("show processes memory") || cmd == "show memory" {
            memory(&mut out, text);
        } else if cmd == "show interfaces" {
            interface_errors(&mut out, text);
        } else if cmd.starts_with("show interfaces status") {
            err_disabled(&mut out, text, family);
        } else if cmd == "show logging" {
            logging(&mut out, text);
        }
        all.extend(out.list);
    }
    if !all
        .iter()
        .any(|f| matches!(f.severity, Severity::Critical | Severity::Warning))
    {
        all.push(Finding {
            severity: Severity::Ok,
            code: "allGood".into(),
            params: BTreeMap::new(),
            command: String::new(),
        });
    }
    all.sort_by_key(|f| f.severity);
    all
}

fn version(out: &mut Out, text: &str) {
    let reason = RELOAD_REASON
        .captures(text)
        .map(|c| c[1].trim().to_owned())
        .unwrap_or_default();
    if let Some(c) = UPTIME_CISCO.captures(text) {
        let up = c[1].trim().to_owned();
        let l = up.to_ascii_lowercase();
        if !l.contains("year") && !l.contains("week") && !l.contains("day") {
            out.add(
                Severity::Warning,
                "recentReload",
                &[("uptime", up), ("reason", reason)],
            );
        }
    } else if let Some(c) = UPTIME_RUIJIE.captures(text)
        && &c[1] == "0"
    {
        out.add(
            Severity::Warning,
            "recentReload",
            &[
                ("uptime", format!("{}:{}:{}", &c[2], &c[3], &c[4])),
                ("reason", reason),
            ],
        );
    }
}

fn environment(out: &mut Out, text: &str, any_row: bool) {
    for line in text.lines() {
        let t = line.trim();
        if (any_row || ENV_PART.is_match(t)) && ENV_BAD.is_match(t) {
            out.add(
                Severity::Critical,
                "envFault",
                &[("line", t.chars().take(160).collect())],
            );
        }
    }
}

fn cpu(out: &mut Out, text: &str) {
    let five = CPU_CISCO
        .captures(text)
        .and_then(|c| c[3].parse::<f64>().ok())
        .or_else(|| {
            CPU_RUIJIE
                .captures(text)
                .and_then(|c| c[1].parse::<f64>().ok())
        });
    if let Some(v) = five
        && let Some(sev) = pct_level(v, 70.0, 90.0)
    {
        out.add(sev, "highCpu", &[("percent", format!("{v:.0}"))]);
    }
}

fn memory(out: &mut Out, text: &str) {
    let used = MEM_CISCO
        .captures(text)
        .and_then(|c| {
            let total: f64 = c[1].parse().ok()?;
            let used: f64 = c[2].parse().ok()?;
            (total > 0.0).then(|| used * 100.0 / total)
        })
        .or_else(|| {
            MEM_RUIJIE
                .captures(text)
                .and_then(|c| c[1].parse::<f64>().ok())
        });
    if let Some(v) = used
        && let Some(sev) = pct_level(v, 80.0, 90.0)
    {
        out.add(sev, "highMemory", &[("percent", format!("{v:.0}"))]);
    }
}

fn interface_errors(out: &mut Out, text: &str) {
    let mut iface = String::new();
    for line in text.lines() {
        if let Some(c) = IFACE_HEAD.captures(line) {
            iface = c[1].to_owned();
            continue;
        }
        if iface.is_empty() {
            continue;
        }
        if let Some(c) = INPUT_ERR.captures(line) {
            let input: u64 = c[1].parse().unwrap_or(0);
            let crc: u64 = c[2].parse().unwrap_or(0);
            if crc > 0 {
                let sev = if crc >= 1000 {
                    Severity::Critical
                } else {
                    Severity::Warning
                };
                out.add(
                    sev,
                    "crcErrors",
                    &[("iface", iface.clone()), ("count", crc.to_string())],
                );
            } else if input > 0 {
                out.add(
                    Severity::Warning,
                    "inputErrors",
                    &[("iface", iface.clone()), ("count", input.to_string())],
                );
            }
        } else if let Some(c) = OUTPUT_ERR.captures(line) {
            let n: u64 = c[1].parse().unwrap_or(0);
            if n > 0 {
                out.add(
                    Severity::Warning,
                    "outputErrors",
                    &[("iface", iface.clone()), ("count", n.to_string())],
                );
            }
        }
    }
}

fn err_disabled(out: &mut Out, text: &str, family: Option<&str>) {
    let def = super::parsers::ALL.iter().find(|p| {
        p.command == "show interfaces status"
            && (family == Some("ruijie")) == p.id.starts_with("ruijie")
    });
    let rows = def.map(|p| (p.parse)(text)).unwrap_or_default();
    for r in rows {
        let status = r.get(2).map(String::as_str).unwrap_or("");
        if status.to_ascii_lowercase().contains("err") {
            // Cisco `… err-disabled` lists the reason in the Vlan column.
            let reason = r.get(3).cloned().unwrap_or_default();
            out.add(
                Severity::Critical,
                "errDisabled",
                &[("iface", r[0].clone()), ("reason", reason)],
            );
        }
    }
    // `show interfaces status err-disabled` (Cisco) has its own layout: Port Name Status Reason.
    if out.list.is_empty() {
        for line in text.lines() {
            let w: Vec<&str> = line.split_whitespace().collect();
            if let Some(i) = w
                .iter()
                .position(|t| t.eq_ignore_ascii_case("err-disabled"))
                && i > 0
            {
                let reason = w.get(i + 1..).map(|x| x.join(" ")).unwrap_or_default();
                out.add(
                    Severity::Critical,
                    "errDisabled",
                    &[("iface", w[0].to_owned()), ("reason", reason)],
                );
            }
        }
    }
}

fn logging(out: &mut Out, text: &str) {
    let mut count = 0usize;
    let mut sample = String::new();
    for line in text.lines() {
        if let Some(c) = LOG_SEV.captures(line) {
            count += 1;
            if sample.is_empty() || &c[1] < "2" {
                sample = line.trim().chars().take(160).collect();
            }
        }
    }
    if count > 0 {
        out.add(
            Severity::Warning,
            "logErrors",
            &[("count", count.to_string()), ("sample", sample)],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(command: &str, text: &str) -> CommandOutput {
        CommandOutput {
            command: command.into(),
            text: Some(text.into()),
        }
    }

    fn codes(f: &[Finding]) -> Vec<(Severity, &str)> {
        f.iter().map(|x| (x.severity, x.code.as_str())).collect()
    }

    #[test]
    fn cisco_findings_most_severe_first() {
        let f = evaluate(
            Some("cisco"),
            &[
                o(
                    "show version",
                    "sw1 uptime is 3 hours, 12 minutes\nLast reload reason: power-on\n",
                ),
                o(
                    "show environment all",
                    "FAN 1 is OK\nSW  PID                 Serial#     Status           Sys Pwr  PoE Pwr  Watts\n1A  PWR-C1-715WAC       LIT1       OK               Good     Good     715\nPower supply B: NOT OK\n",
                ),
                o(
                    "show processes cpu sorted",
                    "CPU utilization for five seconds: 95%/3%; one minute: 93%; five minutes: 91%\n",
                ),
                o(
                    "show processes memory sorted",
                    "Processor Pool Total:  1000000 Used:  850000 Free:  150000\n",
                ),
                o(
                    "show interfaces",
                    "GigabitEthernet1/0/5 is up, line protocol is up\n     12403 input errors, 12403 CRC, 0 frame, 0 overrun, 0 ignored\n     0 output errors, 0 collisions\nGigabitEthernet1/0/6 is up, line protocol is up\n     3 input errors, 0 CRC, 3 frame\n     7 output errors, 0 collisions\nVlan1 is administratively down, line protocol is down\n     0 input errors, 0 CRC\n",
                ),
                o(
                    "show interfaces status err-disabled",
                    "\nPort      Name               Status       Reason               Err-disabled Vlans\nGi1/0/3   Printer            err-disabled bpduguard\n",
                ),
                o(
                    "show logging",
                    "*Nov  6 10:00: %LINK-3-UPDOWN: Interface Gi1/0/2, changed state to down\n*Nov  6 10:01: %SYS-5-CONFIG_I: Configured\n*Nov  6 10:02: %PLATFORM_PM-6-MODULE_INSERTED: x\n",
                ),
            ],
        );
        let c = codes(&f);
        assert_eq!(c[0].0, Severity::Critical);
        assert!(c.contains(&(Severity::Critical, "envFault")));
        assert!(c.contains(&(Severity::Critical, "highCpu")));
        assert!(c.contains(&(Severity::Critical, "crcErrors")));
        assert!(c.contains(&(Severity::Critical, "errDisabled")));
        assert!(c.contains(&(Severity::Warning, "recentReload")));
        assert!(c.contains(&(Severity::Warning, "highMemory")));
        assert!(c.contains(&(Severity::Warning, "inputErrors")));
        assert!(c.contains(&(Severity::Warning, "outputErrors")));
        assert!(c.contains(&(Severity::Warning, "logErrors")));
        let crc = f.iter().find(|x| x.code == "crcErrors").unwrap();
        assert_eq!(crc.params["iface"], "GigabitEthernet1/0/5");
        assert_eq!(crc.params["count"], "12403");
        let ed = f.iter().find(|x| x.code == "errDisabled").unwrap();
        assert_eq!(
            (ed.params["iface"].as_str(), ed.params["reason"].as_str()),
            ("Gi1/0/3", "bpduguard")
        );
        assert!(!c.iter().any(|x| x.1 == "allGood"));
    }

    #[test]
    fn ruijie_findings_and_healthy_device() {
        let f = evaluate(
            Some("ruijie"),
            &[
                o("show version", "System uptime           : 0:02:10:05\n"),
                o("show cpu", "CPU utilization in five minutes: 75.10%\n"),
                o(
                    "show memory",
                    "Total memory: 975.7MB, Free memory: 146.3MB, Used memory: 829.4MB, Used rate: 85%\n",
                ),
                o(
                    "show power",
                    "Power-id  Power-type   Status\n1         AC           LinkAndPowerOn\n2         AC           Abnormal\n",
                ),
                o(
                    "show interfaces status",
                    include_str!("fixtures/ruijie_show_interfaces_status.txt"),
                ),
            ],
        );
        let c = codes(&f);
        assert!(c.contains(&(Severity::Warning, "recentReload")));
        assert!(c.contains(&(Severity::Warning, "highCpu")));
        assert!(c.contains(&(Severity::Warning, "highMemory")));
        assert!(c.contains(&(Severity::Critical, "envFault")));

        let healthy = evaluate(
            Some("cisco"),
            &[
                o("show version", "sw1 uptime is 2 years, 3 weeks\n"),
                o(
                    "show processes cpu sorted",
                    "CPU utilization for five seconds: 5%/0%; one minute: 4%; five minutes: 4%",
                ),
                o("show fantasy", "x"),
                CommandOutput {
                    command: "show logging".into(),
                    text: None,
                },
                o(
                    "show environment all",
                    "% Invalid input detected at '^' marker.",
                ),
            ],
        );
        assert_eq!(
            codes(&healthy),
            [
                (Severity::Info, "commandFailed"),
                (Severity::Info, "commandUnsupported"),
                (Severity::Ok, "allGood"),
            ]
        );
    }

    #[test]
    fn command_sets_are_read_only() {
        for fam in [None, Some("cisco"), Some("ruijie"), Some("huawei")] {
            for c in commands(fam) {
                assert!(crate::device::capture::read_only(c), "{c}");
            }
        }
    }
}
