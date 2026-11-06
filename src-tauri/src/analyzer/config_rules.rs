//! Offline config analysis (P4-T07): best-practice rules from `src/data/config-rules.json`
//! (shared data file, extensible without code) run over a running-config.
//!
//! Findings reuse the diagnostics shape; the UI words them as `cfgRule.<id>`.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};
use serde::Deserialize;

use super::diagnostics::{Finding, Severity};

#[derive(Deserialize)]
struct RawFile {
    rules: Vec<RawRule>,
}

#[derive(Deserialize)]
struct RawRule {
    id: String,
    families: Vec<String>,
    severity: String,
    kind: String,
    pattern: Option<String>,
    block: Option<String>,
    forbid: Option<String>,
    require: Option<String>,
}

enum Kind {
    Required(Regex),
    Forbidden(Regex),
    Block {
        header: Regex,
        forbid: Option<Regex>,
        require: Option<Regex>,
    },
    Interfaces,
}

struct Rule {
    id: String,
    families: Vec<String>,
    severity: Severity,
    kind: Kind,
}

fn compile(p: &str) -> Regex {
    RegexBuilder::new(p)
        .multi_line(true)
        .size_limit(1 << 20)
        .build()
        .unwrap_or_else(|e| panic!("config rule pattern {p}: {e}"))
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let raw: RawFile = serde_json::from_str(include_str!("../../../src/data/config-rules.json"))
        .expect("config-rules.json");
    raw.rules
        .into_iter()
        .map(|r| {
            let severity = match r.severity.as_str() {
                "critical" => Severity::Critical,
                "warning" => Severity::Warning,
                _ => Severity::Info,
            };
            let kind = match r.kind.as_str() {
                "required" => Kind::Required(compile(r.pattern.as_deref().expect("pattern"))),
                "forbidden" => Kind::Forbidden(compile(r.pattern.as_deref().expect("pattern"))),
                "block" => Kind::Block {
                    header: compile(r.block.as_deref().expect("block")),
                    forbid: r.forbid.as_deref().map(compile),
                    require: r.require.as_deref().map(compile),
                },
                "interfaces" => Kind::Interfaces,
                other => panic!("unknown config rule kind {other}"),
            };
            Rule {
                id: r.id,
                families: r.families,
                severity,
                kind,
            }
        })
        .collect()
});

/// Physical port types whose missing description and open state is worth a note.
static PHYSICAL: LazyLock<Regex> = LazyLock::new(|| {
    compile(
        r"(?i)^interface\s+((?:fast|gigabit|tengigabit|twentyfivegig|fortygigabit|hundredgig)\S*\s?\d\S*)",
    )
});

/// A config split into blocks: a top-level line and its indented children.
fn blocks(config: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in config.lines() {
        let l = line.trim_end();
        if l.is_empty() || l.starts_with('!') {
            continue;
        }
        if l.starts_with(char::is_whitespace) {
            if let Some(b) = out.last_mut() {
                b.1.push(l);
            }
        } else {
            out.push((l, Vec::new()));
        }
    }
    out
}

fn finding(rule: &Rule, params: &[(&str, String)]) -> Finding {
    Finding {
        severity: rule.severity,
        code: rule.id.clone(),
        params: params
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
        command: "show running-config".into(),
    }
}

/// Findings for `config`, most severe first; an `allGood` when nothing is wrong. Unknown
/// devices get the Cisco rules.
pub fn audit(family: Option<&str>, config: &str) -> Vec<Finding> {
    let config = &config[..config.len().min(super::MAX_INPUT)];
    // Ruijie has its own rules; every other device gets the Cisco-like set.
    let fam = if family == Some("ruijie") {
        "ruijie"
    } else {
        "cisco"
    };
    let top: String = config
        .lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .collect::<Vec<_>>()
        .join("\n");
    let blocks = blocks(config);
    let mut out = Vec::new();
    for rule in RULES.iter().filter(|r| r.families.iter().any(|f| f == fam)) {
        match &rule.kind {
            Kind::Required(re) => {
                if !re.is_match(&top) {
                    out.push(finding(rule, &[]));
                }
            }
            Kind::Forbidden(re) => {
                for line in top.lines().filter(|l| re.is_match(l)) {
                    out.push(finding(rule, &[("line", mask(line))]));
                }
            }
            Kind::Block {
                header,
                forbid,
                require,
            } => {
                for (head, body) in blocks.iter().filter(|(h, _)| header.is_match(h)) {
                    let text = body.join("\n");
                    let bad = forbid.as_ref().is_some_and(|f| f.is_match(&text))
                        || require.as_ref().is_some_and(|r| !r.is_match(&text));
                    if bad {
                        out.push(finding(rule, &[("block", (*head).to_owned())]));
                    }
                }
            }
            Kind::Interfaces => {
                let open: Vec<String> = blocks
                    .iter()
                    .filter_map(|(head, body)| {
                        let name = PHYSICAL.captures(head)?.get(1)?.as_str().trim().to_owned();
                        let shut = body.iter().any(|l| l.trim() == "shutdown");
                        let desc = body
                            .iter()
                            .any(|l| l.trim_start().starts_with("description "));
                        (!shut && !desc).then_some(name)
                    })
                    .collect();
                if !open.is_empty() {
                    let mut list = open.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
                    if open.len() > 8 {
                        list.push_str(", …");
                    }
                    out.push(finding(
                        rule,
                        &[("count", open.len().to_string()), ("list", list)],
                    ));
                }
            }
        }
    }
    if !out
        .iter()
        .any(|f| matches!(f.severity, Severity::Critical | Severity::Warning))
    {
        out.push(Finding {
            severity: Severity::Ok,
            code: "allGood".into(),
            params: BTreeMap::new(),
            command: String::new(),
        });
    }
    out.sort_by_key(|f| f.severity);
    out
}

/// Hides secrets in a reported config line (`password 7 0822…` → `password 7 ****`).
fn mask(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    let mut out = Vec::with_capacity(words.len());
    let mut hide_next = false;
    for (i, w) in words.iter().enumerate() {
        if hide_next {
            // `password 7 <hash>` / `secret 5 <hash>`: keep the type digit, hide the value.
            if w.len() == 1 && w.chars().all(|c| c.is_ascii_digit()) && i + 1 < words.len() {
                out.push((*w).to_owned());
                continue;
            }
            out.push("****".to_owned());
            hide_next = false;
            continue;
        }
        out.push((*w).to_owned());
        hide_next = matches!(
            *w,
            "password" | "secret" | "community" | "key" | "key-string"
        );
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const CISCO_BAD: &str = "\
hostname core-sw1
!
enable password cisco123
username admin privilege 15 password 7 0822455D0A16
snmp-server community public RO
ip http server
!
interface GigabitEthernet1/0/1
 description Uplink
 switchport mode trunk
!
interface GigabitEthernet1/0/2
 switchport access vlan 10
!
interface GigabitEthernet1/0/3
 shutdown
!
interface Vlan10
 ip address 10.0.0.2 255.255.255.0
!
line con 0
 exec-timeout 0 0
line vty 0 4
 password 7 1234
 transport input telnet ssh
line vty 5 15
 access-class 10 in
 transport input ssh
!
end
";

    const CISCO_GOOD: &str = "\
hostname core-sw1
service password-encryption
enable secret 9 $9$abc
username admin privilege 15 secret 9 $9$def
ip ssh version 2
ntp server 10.0.0.1
logging host 10.0.0.5
interface GigabitEthernet1/0/1
 description Uplink
interface GigabitEthernet1/0/2
 shutdown
line con 0
 exec-timeout 10 0
line vty 0 15
 access-class 10 in
 transport input ssh
end
";

    fn codes(f: &[Finding]) -> Vec<&str> {
        f.iter().map(|x| x.code.as_str()).collect()
    }

    #[test]
    fn cisco_bad_config() {
        let f = audit(Some("cisco"), CISCO_BAD);
        let c = codes(&f);
        for want in [
            "enablePasswordPlain",
            "noPasswordEncryption",
            "userPasswordWeak",
            "snmpDefaultCommunity",
            "noNtp",
            "noSyslog",
            "httpServer",
            "sshV2Missing",
            "telnetEnabled",
            "vtyNoAcl",
            "noExecTimeout",
            "portsWithoutDescription",
        ] {
            assert!(c.contains(&want), "{want} missing in {c:?}");
        }
        assert_eq!(f[0].severity, Severity::Critical);
        // Only the first vty block lacks an ACL and allows telnet.
        let telnet: Vec<_> = f.iter().filter(|x| x.code == "telnetEnabled").collect();
        assert_eq!(telnet.len(), 1);
        assert_eq!(telnet[0].params["block"], "line vty 0 4");
        let ports = f
            .iter()
            .find(|x| x.code == "portsWithoutDescription")
            .unwrap();
        assert_eq!(ports.params["list"], "GigabitEthernet1/0/2");
        // Secrets never appear in findings.
        let all = format!("{f:?}");
        assert!(!all.contains("public RO"));
        assert!(!all.contains("cisco123"));
        assert!(!all.contains("0822455D0A16"));
    }

    #[test]
    fn cisco_good_config_is_clean() {
        assert_eq!(codes(&audit(Some("cisco"), CISCO_GOOD)), ["allGood"]);
        assert_eq!(
            codes(&audit(None, CISCO_GOOD)),
            ["allGood"],
            "unknown device: Cisco rules"
        );
    }

    #[test]
    fn ruijie_rules() {
        let cfg = "\
hostname access-sw-02
enable service web-server
snmp-server community private rw
interface GigabitEthernet 0/1
 description Kamera
interface GigabitEthernet 0/2
 switchport access vlan 10
line vty 0 4
 login local
end
";
        let c = codes(&audit(Some("ruijie"), cfg)).join(",");
        for want in [
            "httpServer",
            "snmpDefaultCommunity",
            "sshDisabled",
            "noSyslog",
            "noNtp",
            "vtyNoAcl",
            "portsWithoutDescription",
        ] {
            assert!(c.contains(want), "{want} in {c}");
        }
        assert!(!c.contains("sshV2Missing"), "Cisco-only rule");
        let f = audit(Some("ruijie"), cfg);
        let p = f
            .iter()
            .find(|x| x.code == "portsWithoutDescription")
            .unwrap();
        assert_eq!(p.params["list"], "GigabitEthernet 0/2");
    }

    #[test]
    fn masking_and_garbage() {
        assert_eq!(
            mask("username admin password 7 0822455D0A16"),
            "username admin password 7 ****"
        );
        assert_eq!(
            mask("snmp-server community public RO"),
            "snmp-server community **** RO"
        );
        assert_eq!(mask("enable password cisco"), "enable password ****");
        for g in [
            "",
            "\n\n",
            " indented only",
            "interface",
            "line vty",
            &"x\n".repeat(10_000),
        ] {
            let _ = audit(Some("cisco"), g);
        }
    }
}
