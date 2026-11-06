//! `show tech`, crashinfo and log file analysis (P4-T09). The file is read line by line
//! (never held in memory): sections, device facts, the most frequent syslog messages and
//! findings (tracebacks, crashes, memory allocation failures, CPU hogs, severe logs …).

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use super::diagnostics::{Finding, Severity};
use crate::device::modes;

/// Longest line kept (a binary or one-line file must not use unbounded memory).
const MAX_LINE: usize = 64 * 1024;
const MAX_SECTIONS: usize = 5_000;
const TOP_MESSAGES: usize = 30;
/// Text kept for device facts.
const FACTS_PREFIX: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileKind {
    ShowTech,
    Crashinfo,
    Log,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub title: String,
    /// 1-based, inclusive.
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageCount {
    /// `LINK-3-UPDOWN`
    pub mnemonic: String,
    pub severity: u8,
    pub count: u64,
    /// Line of the first occurrence.
    pub first_line: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSummary {
    pub kind: FileKind,
    pub lines: u64,
    pub bytes: u64,
    pub sections: Vec<Section>,
    pub facts: Vec<(String, String)>,
    pub messages: Vec<MessageCount>,
    pub findings: Vec<Finding>,
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("logfile pattern")
}

static SECTION: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^\s*(?:-{5,}|={5,})\s*((?:show|display|dir|more)\s.*?)\s*(?:-{5,}|={5,})\s*$|^`((?:show|display)\s[^`]*)`\s*$",
    )
});
static SYSLOG: LazyLock<Regex> =
    LazyLock::new(|| re(r"%([A-Z][A-Z0-9_]*(?:-[A-Z0-9_]+)?)-([0-7])-([A-Z0-9_]+)"));
static TRACEBACK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\btraceback\b|-Traceback="));
static CRASH: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?i)unexpected exception|crashinfo|system returned to rom by (?:bus error|address error|watchdog|.*exception|.*crash|s/w reset)|kernel panic|core dump",
    )
});

#[derive(Default)]
struct Acc {
    lines: u64,
    bytes: u64,
    sections: Vec<Section>,
    messages: HashMap<String, MessageCount>,
    tracebacks: (u64, u64, String),
    crash: Option<(u64, String)>,
    head: String,
}

fn sample(line: &str) -> String {
    line.trim().chars().take(200).collect()
}

/// Reads `reader` to the end. I/O errors stop the analysis with an error; bad UTF-8 is
/// read lossily.
pub fn analyze<R: BufRead>(mut reader: R) -> std::io::Result<FileSummary> {
    let mut acc = Acc::default();
    let mut buf = Vec::with_capacity(4096);
    loop {
        buf.clear();
        // Read one line, keeping at most MAX_LINE bytes of it.
        let mut over = false;
        let n = loop {
            let avail = reader.fill_buf()?;
            if avail.is_empty() {
                break buf.len();
            }
            let (chunk, done) = match avail.iter().position(|&b| b == b'\n') {
                Some(i) => (&avail[..=i], true),
                None => (avail, false),
            };
            let take = chunk.len();
            if buf.len() < MAX_LINE {
                let room = MAX_LINE - buf.len();
                buf.extend_from_slice(&chunk[..chunk.len().min(room)]);
            } else {
                over = true;
            }
            acc.bytes += take as u64;
            reader.consume(take);
            if done {
                break buf.len().max(1);
            }
        };
        if n == 0 && !over {
            break;
        }
        acc.lines += 1;
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches(['\n', '\r']);
        feed(&mut acc, line);
    }
    Ok(finish(acc))
}

fn feed(acc: &mut Acc, line: &str) {
    let no = acc.lines;
    if acc.head.len() < FACTS_PREFIX {
        acc.head.push_str(line);
        acc.head.push('\n');
    }
    if let Some(c) = SECTION.captures(line) {
        let title = c
            .get(1)
            .or_else(|| c.get(2))
            .map_or("", |m| m.as_str())
            .trim();
        if let Some(last) = acc.sections.last_mut() {
            last.end = no.saturating_sub(1).max(last.start);
        }
        if acc.sections.len() < MAX_SECTIONS {
            acc.sections.push(Section {
                title: title.chars().take(200).collect(),
                start: no,
                end: no,
            });
        }
    }
    if let Some(c) = SYSLOG.captures(line) {
        let mnemonic = format!("{}-{}-{}", &c[1], &c[2], &c[3]);
        let severity = c[2].parse().unwrap_or(7);
        if acc.messages.len() < 20_000 || acc.messages.contains_key(&mnemonic) {
            acc.messages
                .entry(mnemonic.clone())
                .or_insert(MessageCount {
                    mnemonic,
                    severity,
                    count: 0,
                    first_line: no,
                })
                .count += 1;
        }
    }
    if TRACEBACK.is_match(line) {
        if acc.tracebacks.0 == 0 {
            acc.tracebacks.1 = no;
            acc.tracebacks.2 = sample(line);
        }
        acc.tracebacks.0 += 1;
    }
    if acc.crash.is_none() && CRASH.is_match(line) {
        acc.crash = Some((no, sample(line)));
    }
}

fn finish(mut acc: Acc) -> FileSummary {
    if let Some(last) = acc.sections.last_mut() {
        last.end = acc.lines.max(last.start);
    }
    let mut messages: Vec<MessageCount> = acc.messages.into_values().collect();
    messages.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.count.cmp(&a.count))
            .then(a.mnemonic.cmp(&b.mnemonic))
    });

    let mut findings = Vec::new();
    fn add(out: &mut Vec<Finding>, severity: Severity, code: &str, params: &[(&str, String)]) {
        out.push(Finding {
            severity,
            code: code.into(),
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect(),
            command: String::new(),
        });
    }
    if let Some((line, s)) = &acc.crash {
        add(
            &mut findings,
            Severity::Critical,
            "fileCrash",
            &[("line", line.to_string()), ("sample", s.clone())],
        );
    }
    if acc.tracebacks.0 > 0 {
        add(
            &mut findings,
            Severity::Critical,
            "fileTraceback",
            &[
                ("count", acc.tracebacks.0.to_string()),
                ("line", acc.tracebacks.1.to_string()),
                ("sample", acc.tracebacks.2.clone()),
            ],
        );
    }
    let count = |pred: &dyn Fn(&MessageCount) -> bool| {
        messages
            .iter()
            .filter(|m| pred(m))
            .map(|m| m.count)
            .sum::<u64>()
    };
    let named = |name: &str| {
        messages
            .iter()
            .filter(|m| m.mnemonic.ends_with(name))
            .map(|m| m.count)
            .sum::<u64>()
    };
    let mallocfail = named("-MALLOCFAIL");
    if mallocfail > 0 {
        add(
            &mut findings,
            Severity::Critical,
            "fileMallocFail",
            &[("count", mallocfail.to_string())],
        );
    }
    let sev012 = count(&|m| m.severity <= 2);
    if sev012 > 0 {
        add(
            &mut findings,
            Severity::Critical,
            "fileSevere",
            &[("count", sev012.to_string())],
        );
    }
    let cpuhog = named("-CPUHOG");
    if cpuhog > 0 {
        add(
            &mut findings,
            Severity::Warning,
            "fileCpuHog",
            &[("count", cpuhog.to_string())],
        );
    }
    let errdis = named("-ERR_DISABLE");
    if errdis > 0 {
        add(
            &mut findings,
            Severity::Warning,
            "fileErrDisable",
            &[("count", errdis.to_string())],
        );
    }
    let flaps = named("-UPDOWN");
    if flaps >= 10 {
        add(
            &mut findings,
            Severity::Warning,
            "fileLinkFlaps",
            &[("count", flaps.to_string())],
        );
    }
    let sev3 = count(&|m| m.severity == 3);
    if sev3 > 0 {
        add(
            &mut findings,
            Severity::Warning,
            "fileErrors",
            &[("count", sev3.to_string())],
        );
    }
    if !findings
        .iter()
        .any(|f| matches!(f.severity, Severity::Critical | Severity::Warning))
    {
        add(&mut findings, Severity::Ok, "allGood", &[]);
    }
    findings.sort_by_key(|f| f.severity);

    let facts = modes::identify(&acc.head)
        .map(|p| p.parse_facts(&acc.head))
        .map(|f| {
            [
                ("hostname", f.hostname),
                ("model", f.model),
                ("version", f.version),
                ("serial", f.serial),
                ("uptime", f.uptime),
            ]
            .into_iter()
            .filter_map(|(k, v)| v.map(|v| (k.to_owned(), v)))
            .collect()
        })
        .unwrap_or_default();
    let lower_head = acc.head.to_ascii_lowercase();
    let kind = if acc.crash.is_some() && lower_head.contains("crashinfo") {
        FileKind::Crashinfo
    } else if acc.sections.len() >= 3 {
        FileKind::ShowTech
    } else if !messages.is_empty() {
        FileKind::Log
    } else {
        FileKind::Text
    };
    messages.truncate(TOP_MESSAGES);
    FileSummary {
        kind,
        lines: acc.lines,
        bytes: acc.bytes,
        sections: acc.sections,
        facts,
        messages,
        findings,
    }
}

/// Lines `start..=end` (1-based) of a file, at most `max_lines` / `max_bytes`.
pub fn read_lines<R: BufRead>(
    reader: R,
    start: u64,
    end: u64,
    max_lines: u64,
    max_bytes: usize,
) -> std::io::Result<String> {
    let mut out = String::new();
    let last = end.min(start.saturating_add(max_lines).saturating_sub(1));
    for (i, line) in reader.split(b'\n').enumerate() {
        let no = i as u64 + 1;
        if no > last || out.len() > max_bytes {
            break;
        }
        let line = line?;
        if no >= start {
            out.push_str(
                String::from_utf8_lossy(&line[..line.len().min(MAX_LINE)]).trim_end_matches('\r'),
            );
            out.push('\n');
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const SHOW_TECH: &str = "\
------------------ show version ------------------

Cisco IOS XE Software, Version 17.09.04a
core-sw1 uptime is 2 hours, 3 minutes
System returned to ROM by bus error at PC 0x0
Model Number                       : C9300-48P
System Serial Number               : FOC2345X1YZ

------------------ show logging ------------------

*Nov  6 10:00: %LINK-3-UPDOWN: Interface Gi1/0/2, changed state to down
*Nov  6 10:00: %LINK-3-UPDOWN: Interface Gi1/0/2, changed state to up
*Nov  6 10:01: %SYS-2-MALLOCFAIL: Memory allocation of 65536 bytes failed
-Traceback= 1A2B3C 4D5E6F
*Nov  6 10:02: %SYS-3-CPUHOG: Task is running for (2004)msecs
*Nov  6 10:03: %PM-4-ERR_DISABLE: bpduguard error detected on Gi1/0/3
*Nov  6 10:04: %SYS-5-CONFIG_I: Configured from console

------------------ show interfaces ------------------

GigabitEthernet1/0/1 is up, line protocol is up
";

    fn codes(s: &FileSummary) -> Vec<&str> {
        s.findings.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn show_tech_summary() {
        let s = analyze(Cursor::new(SHOW_TECH)).unwrap();
        assert_eq!(s.kind, FileKind::ShowTech);
        assert_eq!(s.lines, SHOW_TECH.lines().count() as u64);
        assert_eq!(s.bytes, SHOW_TECH.len() as u64);
        assert_eq!(
            s.sections
                .iter()
                .map(|x| x.title.as_str())
                .collect::<Vec<_>>(),
            ["show version", "show logging", "show interfaces"]
        );
        assert_eq!((s.sections[0].start, s.sections[0].end), (1, 8));
        assert_eq!(s.sections[2].end, s.lines);
        let c = codes(&s);
        for want in [
            "fileCrash",
            "fileTraceback",
            "fileMallocFail",
            "fileSevere",
            "fileCpuHog",
            "fileErrDisable",
            "fileErrors",
        ] {
            assert!(c.contains(&want), "{want} in {c:?}");
        }
        assert_eq!(s.findings[0].severity, Severity::Critical);
        let top = &s.messages[0];
        assert_eq!(
            (top.mnemonic.as_str(), top.severity, top.count),
            ("SYS-2-MALLOCFAIL", 2, 1)
        );
        let link = s
            .messages
            .iter()
            .find(|m| m.mnemonic == "LINK-3-UPDOWN")
            .unwrap();
        assert_eq!(link.count, 2);
        assert!(s.facts.contains(&("serial".into(), "FOC2345X1YZ".into())));
        let sec = read_lines(
            Cursor::new(SHOW_TECH),
            s.sections[1].start,
            s.sections[1].end,
            1000,
            1 << 20,
        )
        .unwrap();
        assert!(sec.starts_with("------------------ show logging"));
        assert!(sec.contains("CONFIG_I"));
        assert!(!sec.contains("show interfaces"));
    }

    #[test]
    fn quiet_log_and_plain_text() {
        let log = "Nov 6 %SYS-5-CONFIG_I: Configured\nNov 6 %SYS-6-LOGGINGHOST_STARTSTOP: Logging to host 10.0.0.5\n";
        let s = analyze(Cursor::new(log)).unwrap();
        assert_eq!(s.kind, FileKind::Log);
        assert_eq!(codes(&s), ["allGood"]);
        let t = analyze(Cursor::new("hello\nworld")).unwrap();
        assert_eq!((t.kind, t.lines), (FileKind::Text, 2));
        assert_eq!(analyze(Cursor::new("")).unwrap().lines, 0);
    }

    #[test]
    fn huge_lines_binary_and_volume_stay_bounded() {
        let mut big = vec![b'x'; 3 * MAX_LINE];
        big.extend_from_slice(b"\n\xff\xfe\x00 %LINK-3-UPDOWN: x\n");
        let s = analyze(Cursor::new(big.clone())).unwrap();
        assert_eq!(s.lines, 2);
        assert_eq!(s.bytes, big.len() as u64);
        // ~5 MB of log lines stream through quickly.
        let line = "*Nov  6 10:00: %LINK-3-UPDOWN: Interface Gi1/0/2, changed state to down\n";
        let many = line.repeat(70_000);
        let t = std::time::Instant::now();
        let s = analyze(Cursor::new(many.as_bytes())).unwrap();
        assert_eq!(s.messages[0].count, 70_000);
        assert!(codes(&s).contains(&"fileLinkFlaps"));
        assert!(t.elapsed() < std::time::Duration::from_secs(20));
    }
}
