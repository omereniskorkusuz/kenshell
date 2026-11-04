//! Transport-independent CLI state machine. Bytes in → bytes out; no I/O, fully testable.
//!
//! Supports: login, enable, configure terminal, interface/sub-modes, exit/end/Ctrl-Z,
//! `hostname`, `do`, `terminal length`, `--More--` paging, abbreviated commands,
//! ambiguous/invalid-input messages and `reload`.

use std::collections::VecDeque;

use crate::profile::{MinMode, Profile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    User,
    Privileged,
    Config,
    Interface(String),
    Sub(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    LoginUser,
    LoginPass { user: String },
    EnablePass,
    Shell,
    Paging(VecDeque<String>),
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Enable,
    Disable,
    ConfigureTerminal,
    Exit,
    End,
    TerminalLength,
    Reload,
    Interface,
    SubMode(usize),
    Hostname,
    Do,
    Output(usize),
}

struct Candidate {
    words: Vec<String>,
    action: Action,
}

enum Match {
    Found(Action, usize),
    Ambiguous,
    Incomplete,
    Unknown { bad_word: usize },
}

pub struct Session<'p> {
    profile: &'p Profile,
    hostname: String,
    stage: Stage,
    mode: Mode,
    line: String,
    term_length: usize,
    last_was_cr: bool,
}

impl<'p> Session<'p> {
    pub fn new(profile: &'p Profile) -> Self {
        Self {
            profile,
            hostname: profile.hostname.clone(),
            stage: Stage::LoginUser,
            mode: Mode::User,
            line: String::new(),
            term_length: profile.terminal_length,
            last_was_cr: false,
        }
    }

    /// A session that skips the CLI login (SSH already authenticated the user).
    pub fn new_authenticated(profile: &'p Profile) -> Self {
        let mut s = Self::new(profile);
        s.stage = Stage::Shell;
        s
    }

    pub fn is_closed(&self) -> bool {
        self.stage == Stage::Closed
    }

    #[cfg(test)]
    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Initial output after connect: banner + username prompt.
    pub fn start(&mut self) -> Vec<u8> {
        let mut out = String::new();
        if self.stage == Stage::Shell {
            if !self.profile.motd.is_empty() {
                out.push_str(&crlf(&self.profile.motd));
                out.push_str("\r\n");
            }
            out.push_str("\r\n");
            out.push_str(&self.prompt());
            return out.into_bytes();
        }
        if !self.profile.banner.is_empty() {
            out.push_str(&crlf(&self.profile.banner));
            out.push_str("\r\n");
        }
        out.push_str(&self.profile.username_prompt);
        out.into_bytes()
    }

    /// Simulates a BREAK (serial break / telnet IAC BRK). Skeleton: just a new prompt.
    pub fn on_break(&mut self) -> Vec<u8> {
        b"\r\n".to_vec()
    }

    pub fn feed(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = String::new();
        for &b in input {
            if self.is_closed() {
                break;
            }
            let after_cr = self.last_was_cr;
            self.last_was_cr = b == b'\r';
            if let Stage::Paging(_) = self.stage {
                if (b == b'\n' || b == 0) && after_cr {
                    continue;
                }
                self.page_key(b, &mut out);
                continue;
            }
            match b {
                b'\n' | 0 if after_cr => {}
                b'\r' | b'\n' => {
                    out.push_str("\r\n");
                    let line = std::mem::take(&mut self.line);
                    self.process_line(&line, &mut out);
                }
                0x08 | 0x7f => {
                    if self.line.pop().is_some() && self.echoes() {
                        out.push_str("\x08 \x08");
                    }
                }
                0x15 => {
                    // Ctrl-U: kill line
                    if self.echoes() {
                        for _ in 0..self.line.chars().count() {
                            out.push_str("\x08 \x08");
                        }
                    }
                    self.line.clear();
                }
                0x1a if self.in_config() => {
                    // Ctrl-Z
                    self.line.clear();
                    self.mode = Mode::Privileged;
                    out.push_str("^Z\r\n");
                    out.push_str(&self.prompt());
                }
                0x03 => {
                    self.line.clear();
                    out.push_str("^C\r\n");
                    self.prompt_for_stage(&mut out);
                }
                0x20..=0x7e => {
                    self.line.push(b as char);
                    if self.echoes() {
                        out.push(b as char);
                    }
                }
                _ => {}
            }
        }
        out.into_bytes()
    }

    fn echoes(&self) -> bool {
        !matches!(self.stage, Stage::LoginPass { .. } | Stage::EnablePass)
    }

    fn in_config(&self) -> bool {
        matches!(self.mode, Mode::Config | Mode::Interface(_) | Mode::Sub(_))
    }

    fn prompt(&self) -> String {
        let p = &self.profile.prompts;
        let tpl = match &self.mode {
            Mode::User => p.user.clone(),
            Mode::Privileged => p.privileged.clone(),
            Mode::Config => p.config.clone(),
            Mode::Interface(i) => p.interface.replace("{iface}", i),
            Mode::Sub(s) => p.submode.replace("{sub}", s),
        };
        tpl.replace("{host}", &self.hostname)
    }

    fn prompt_for_stage(&self, out: &mut String) {
        match self.stage {
            Stage::LoginUser => out.push_str(&self.profile.username_prompt),
            Stage::LoginPass { .. } | Stage::EnablePass => {
                out.push_str(&self.profile.password_prompt)
            }
            Stage::Shell => out.push_str(&self.prompt()),
            Stage::Paging(_) | Stage::Closed => {}
        }
    }

    fn process_line(&mut self, line: &str, out: &mut String) {
        match std::mem::replace(&mut self.stage, Stage::Shell) {
            Stage::LoginUser => {
                let user = line.trim().to_string();
                self.stage = if user.is_empty() {
                    Stage::LoginUser
                } else {
                    Stage::LoginPass { user }
                };
                self.prompt_for_stage(out);
            }
            Stage::LoginPass { user } => {
                if user == self.profile.username && line == self.profile.password {
                    if !self.profile.motd.is_empty() {
                        out.push_str(&crlf(&self.profile.motd));
                        out.push_str("\r\n");
                    }
                    self.mode = Mode::User;
                    self.stage = Stage::Shell;
                } else {
                    out.push_str(&crlf(&self.profile.messages.login_failed));
                    out.push_str("\r\n\r\n");
                    self.stage = Stage::LoginUser;
                }
                self.prompt_for_stage(out);
            }
            Stage::EnablePass => {
                if Some(line) == self.profile.enable_password.as_deref() {
                    self.mode = Mode::Privileged;
                } else {
                    out.push_str(&crlf(&self.profile.messages.enable_failed));
                    out.push_str("\r\n");
                }
                self.prompt_for_stage(out);
            }
            Stage::Shell => self.execute(line, out),
            s @ (Stage::Paging(_) | Stage::Closed) => self.stage = s,
        }
    }

    fn candidates(&self, privileged_view: bool) -> Vec<Candidate> {
        fn add(c: &mut Vec<Candidate>, words: &str, action: Action) {
            c.push(Candidate {
                words: words.split(' ').map(str::to_string).collect(),
                action,
            });
        }
        let mut c = Vec::new();
        let exec_mode = privileged_view || matches!(self.mode, Mode::User | Mode::Privileged);
        if exec_mode {
            let level = if privileged_view || self.mode == Mode::Privileged {
                MinMode::Privileged
            } else {
                MinMode::User
            };
            for (i, cmd) in self.profile.commands.iter().enumerate() {
                if cmd.min_mode <= level {
                    c.push(Candidate {
                        words: cmd.words.clone(),
                        action: Action::Output(i),
                    });
                }
            }
        }
        if privileged_view {
            return c;
        }
        match self.mode {
            Mode::User => {
                add(&mut c, "enable", Action::Enable);
                add(&mut c, "exit", Action::Exit);
                add(&mut c, "logout", Action::Exit);
                add(&mut c, "terminal length", Action::TerminalLength);
            }
            Mode::Privileged => {
                add(&mut c, "disable", Action::Disable);
                add(&mut c, "configure terminal", Action::ConfigureTerminal);
                add(&mut c, "exit", Action::Exit);
                add(&mut c, "logout", Action::Exit);
                add(&mut c, "terminal length", Action::TerminalLength);
                add(&mut c, "reload", Action::Reload);
            }
            Mode::Config | Mode::Interface(_) | Mode::Sub(_) => {
                add(&mut c, "end", Action::End);
                add(&mut c, "exit", Action::Exit);
                add(&mut c, "interface", Action::Interface);
                add(&mut c, "hostname", Action::Hostname);
                add(&mut c, "do", Action::Do);
                for (i, s) in self.profile.submodes.iter().enumerate() {
                    add(&mut c, &s.keyword, Action::SubMode(i));
                }
            }
        }
        c
    }

    fn find(cands: &[Candidate], words: &[&str]) -> Match {
        let lower: Vec<String> = words.iter().map(|w| w.to_ascii_lowercase()).collect();
        let mut best: Vec<(&Candidate, bool)> = Vec::new();
        let mut best_len = 0;
        let mut furthest_ok = 0;
        let mut partial: Vec<&str> = Vec::new();
        for cand in cands {
            let n = cand.words.len();
            let ok = lower
                .iter()
                .zip(&cand.words)
                .take_while(|(i, c)| c.starts_with(i.as_str()))
                .count();
            furthest_ok = furthest_ok.max(ok);
            if ok == lower.len() && lower.len() < n {
                partial.push(&cand.words[lower.len() - 1]);
                continue;
            }
            if ok < n {
                continue;
            }
            let exact = lower.iter().zip(&cand.words).all(|(i, c)| i == c);
            if n > best_len {
                best_len = n;
                best.clear();
            }
            if n == best_len {
                best.push((cand, exact));
            }
        }
        match best.as_slice() {
            [] if !partial.is_empty() => {
                partial.sort_unstable();
                partial.dedup();
                if partial.len() > 1 {
                    Match::Ambiguous
                } else {
                    Match::Incomplete
                }
            }
            [] => Match::Unknown {
                bad_word: furthest_ok,
            },
            [(c, _)] => Match::Found(c.action, best_len),
            many => {
                let exact: Vec<_> = many.iter().filter(|(_, e)| *e).collect();
                match exact.as_slice() {
                    [(c, _)] => Match::Found(c.action, best_len),
                    _ => Match::Ambiguous,
                }
            }
        }
    }

    fn execute(&mut self, line: &str, out: &mut String) {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.is_empty() {
            out.push_str(&self.prompt());
            return;
        }
        let cands = self.candidates(false);
        match Self::find(&cands, &words) {
            Match::Found(action, used) => self.run(action, &words[used..], line, out),
            Match::Ambiguous => {
                out.push_str(
                    &self
                        .profile
                        .messages
                        .ambiguous
                        .replace("{cmd}", line.trim()),
                );
                out.push_str("\r\n");
                out.push_str(&self.prompt());
            }
            Match::Incomplete => {
                out.push_str(&self.profile.messages.incomplete);
                out.push_str("\r\n");
                out.push_str(&self.prompt());
            }
            Match::Unknown { .. } if self.in_config() => {
                // Unknown config lines are accepted silently (we only simulate the CLI).
                out.push_str(&self.prompt());
            }
            Match::Unknown { bad_word } => {
                self.invalid(line, bad_word, out);
                out.push_str(&self.prompt());
            }
        }
    }

    fn invalid(&self, line: &str, bad_word: usize, out: &mut String) {
        // Caret under the first character of the first word that failed to match.
        let mut col = 0;
        let mut seen = 0;
        let mut in_word = false;
        for (i, ch) in line.char_indices() {
            if ch.is_whitespace() {
                in_word = false;
            } else if !in_word {
                in_word = true;
                if seen == bad_word {
                    col = i;
                    break;
                }
                seen += 1;
            }
        }
        let pad = self.prompt().chars().count() + col;
        out.push_str(&" ".repeat(pad));
        out.push_str("^\r\n");
        out.push_str(&self.profile.messages.invalid_input);
        out.push_str("\r\n\r\n");
    }

    fn run(&mut self, action: Action, args: &[&str], line: &str, out: &mut String) {
        match action {
            Action::Enable => {
                if self.profile.enable_password.is_some() {
                    self.stage = Stage::EnablePass;
                    out.push_str(&self.profile.password_prompt);
                    return;
                }
                self.mode = Mode::Privileged;
            }
            Action::Disable => self.mode = Mode::User,
            Action::ConfigureTerminal => {
                out.push_str("Enter configuration commands, one per line.  End with CNTL/Z.\r\n");
                self.mode = Mode::Config;
            }
            Action::Exit => match self.mode {
                Mode::User | Mode::Privileged => {
                    self.stage = Stage::Closed;
                    return;
                }
                Mode::Config => self.mode = Mode::Privileged,
                Mode::Interface(_) | Mode::Sub(_) => self.mode = Mode::Config,
            },
            Action::End => self.mode = Mode::Privileged,
            Action::TerminalLength => match args.first().and_then(|a| a.parse().ok()) {
                Some(n) => self.term_length = n,
                None => self.invalid(line, 2, out),
            },
            Action::Reload => {
                out.push_str(&crlf(&self.profile.reload_text));
                out.push_str("\r\n");
                self.mode = Mode::User;
                self.hostname = self.profile.hostname.clone();
                self.term_length = self.profile.terminal_length;
                self.stage = Stage::LoginUser;
                out.push_str(&String::from_utf8_lossy(&self.start()));
                return;
            }
            Action::Interface => match normalize_iface(self.profile, args) {
                Some(name) => self.mode = Mode::Interface(name),
                None => self.invalid(line, 1, out),
            },
            Action::SubMode(i) => self.mode = Mode::Sub(self.profile.submodes[i].suffix.clone()),
            Action::Hostname => match args.first() {
                Some(h) => self.hostname = (*h).to_string(),
                None => self.invalid(line, 1, out),
            },
            Action::Do => {
                let cands = self.candidates(true);
                match Self::find(&cands, args) {
                    Match::Found(Action::Output(i), _) => {
                        return self.emit(&self.profile.commands[i].output.clone(), out);
                    }
                    _ => self.invalid(line, 1, out),
                }
            }
            Action::Output(i) => return self.emit(&self.profile.commands[i].output.clone(), out),
        }
        out.push_str(&self.prompt());
    }

    /// Writes command output, paging with `--More--` when `terminal length` > 0.
    fn emit(&mut self, text: &str, out: &mut String) {
        let mut lines: VecDeque<String> = text
            .trim_end_matches('\n')
            .split('\n')
            .map(|l| l.trim_end_matches('\r').replace("{host}", &self.hostname))
            .collect();
        let page = self.term_length.saturating_sub(1);
        if page == 0 || lines.len() <= page {
            for l in lines {
                out.push_str(&l);
                out.push_str("\r\n");
            }
            out.push_str(&self.prompt());
            return;
        }
        for l in lines.drain(..page) {
            out.push_str(&l);
            out.push_str("\r\n");
        }
        out.push_str(&self.profile.more_prompt);
        self.stage = Stage::Paging(lines);
    }

    fn page_key(&mut self, key: u8, out: &mut String) {
        let Stage::Paging(mut rest) = std::mem::replace(&mut self.stage, Stage::Shell) else {
            return;
        };
        // Erase the --More-- prompt the way IOS does (backspace, blank, backspace).
        let n = self.profile.more_prompt.chars().count();
        out.push_str(&"\x08".repeat(n));
        out.push_str(&" ".repeat(n));
        out.push_str(&"\x08".repeat(n));
        let take = match key {
            b' ' => self.term_length.saturating_sub(1).max(1),
            b'\r' | b'\n' => 1,
            _ => {
                out.push_str("\r\n");
                out.push_str(&self.prompt());
                return;
            }
        };
        for l in rest.drain(..take.min(rest.len())) {
            out.push_str(&l);
            out.push_str("\r\n");
        }
        if rest.is_empty() {
            out.push_str(&self.prompt());
        } else {
            out.push_str(&self.profile.more_prompt);
            self.stage = Stage::Paging(rest);
        }
    }
}

/// Expands `gi0/1`, `Gi 0/1`, `ten 0/49`, `ag1` to the vendor's full interface name.
pub fn normalize_iface(profile: &Profile, args: &[&str]) -> Option<String> {
    let joined = args.join(" ");
    let joined = joined.trim();
    let split = joined
        .find(|c: char| c.is_ascii_digit() || c.is_whitespace())
        .unwrap_or(joined.len());
    let (kind, num) = joined.split_at(split);
    let num = num.trim();
    if kind.is_empty() || num.is_empty() {
        return None;
    }
    let kind_l = kind.to_ascii_lowercase();
    let full = profile
        .iface_types
        .iter()
        .find(|t| t.to_ascii_lowercase().starts_with(&kind_l))?;
    let sep = if profile.iface_space { " " } else { "" };
    Some(format!("{full}{sep}{num}"))
}

fn crlf(s: &str) -> String {
    s.trim_end_matches('\n')
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cisco() -> Profile {
        Profile::builtin("cisco").unwrap()
    }
    fn ruijie() -> Profile {
        Profile::builtin("ruijie").unwrap()
    }

    fn send(s: &mut Session, input: &str) -> String {
        String::from_utf8(s.feed(input.as_bytes())).unwrap()
    }

    fn logged_in(p: &Profile) -> Session<'_> {
        let mut s = Session::new(p);
        s.start();
        send(&mut s, &format!("{}\r\n{}\r\n", p.username, p.password));
        assert_eq!(s.mode(), &Mode::User);
        s
    }

    fn enabled(p: &Profile) -> Session<'_> {
        let mut s = logged_in(p);
        send(&mut s, "enable\r\n");
        if let Some(pw) = &p.enable_password {
            send(&mut s, &format!("{pw}\r\n"));
        }
        assert_eq!(s.mode(), &Mode::Privileged);
        s
    }

    #[test]
    fn login_rejects_bad_password_and_hides_it() {
        let p = cisco();
        let mut s = Session::new(&p);
        s.start();
        let out = send(&mut s, "admin\r\nwrong\r\n");
        assert!(!out.contains("wrong"));
        assert!(out.contains(&p.messages.login_failed));
        assert!(out.ends_with(&p.username_prompt));
    }

    #[test]
    fn cisco_mode_transitions_and_prompts() {
        let p = cisco();
        let mut s = enabled(&p);
        let out = send(&mut s, "conf t\r\n");
        assert!(out.ends_with("core-sw-01(config)#"), "{out:?}");
        let out = send(&mut s, "int gi1/0/24\r\n");
        assert_eq!(s.mode(), &Mode::Interface("GigabitEthernet1/0/24".into()));
        assert!(out.ends_with("core-sw-01(config-if)#"));
        let out = send(&mut s, "exit\r\n");
        assert!(out.ends_with("(config)#"));
        let out = send(&mut s, "vlan 10\r\n");
        assert!(out.ends_with("core-sw-01(config-vlan)#"), "{out:?}");
        let out = send(&mut s, "\x1a");
        assert!(out.ends_with("core-sw-01#"));
        let out = send(&mut s, "disable\r\n");
        assert!(out.ends_with("core-sw-01>"));
    }

    #[test]
    fn ruijie_interface_prompt_keeps_space() {
        let p = ruijie();
        let mut s = enabled(&p);
        send(&mut s, "configure terminal\r\n");
        let out = send(&mut s, "interface gi 0/1\r\n");
        assert!(
            out.ends_with("access-sw-02(config-if-GigabitEthernet 0/1)#"),
            "{out:?}"
        );
        let out = send(&mut s, "int ag1\r\n");
        assert!(out.ends_with("(config-if-AggregatePort 1)#"), "{out:?}");
        let out = send(&mut s, "end\r\n");
        assert!(out.ends_with("access-sw-02#"));
    }

    #[test]
    fn abbreviations_and_longest_match() {
        let p = cisco();
        let mut s = enabled(&p);
        send(&mut s, "terminal length 0\r\n");
        let out = send(&mut s, "sh ip int br\r\n");
        assert!(out.contains("Vlan1"), "{out:?}");
        let out = send(&mut s, "sh int status\r\n");
        assert!(out.contains("Gi1/0/1"), "{out:?}");
    }

    #[test]
    fn ambiguous_and_invalid_input() {
        let p = cisco();
        let mut s = logged_in(&p);
        let out = send(&mut s, "e\r\n");
        assert!(out.contains("Ambiguous"), "{out:?}");
        let out = send(&mut s, "sh\r\n");
        assert!(out.contains(&p.messages.incomplete), "{out:?}");
        let mut s = enabled(&p);
        let out = send(&mut s, "show bogus\r\n");
        let caret_line = out.lines().nth(1).unwrap(); // line 0 is the echo
        assert_eq!(caret_line.find('^'), Some("core-sw-01#show ".len()));
        assert!(out.contains("% Invalid input detected at '^' marker."));
    }

    #[test]
    fn paging_with_more_prompt() {
        let p = cisco();
        let mut s = enabled(&p);
        let out = send(&mut s, "show running-config\r\n");
        assert!(out.ends_with(&p.more_prompt), "{out:?}");
        assert!(matches!(s.stage, Stage::Paging(_)));
        // Enter → one more line, still paging (config is long).
        let out = send(&mut s, "\r");
        assert_eq!(out.matches("\r\n").count(), 1);
        // q → abort to prompt.
        let out = send(&mut s, "q");
        assert!(out.ends_with("core-sw-01#"));
        // terminal length 0 → no paging.
        send(&mut s, "terminal length 0\r\n");
        let out = send(&mut s, "show running-config\r\n");
        assert!(!out.contains(&p.more_prompt));
        assert!(out.ends_with("core-sw-01#"));
    }

    #[test]
    fn hostname_and_do_in_config() {
        let p = ruijie();
        let mut s = enabled(&p);
        send(&mut s, "terminal length 0\r\n");
        send(&mut s, "conf t\r\n");
        let out = send(&mut s, "hostname LAB-SW\r\n");
        assert!(out.ends_with("LAB-SW(config)#"), "{out:?}");
        let out = send(&mut s, "do show version\r\n");
        assert!(out.contains("RGOS"));
        assert!(out.ends_with("LAB-SW(config)#"));
    }

    #[test]
    fn crlf_and_cr_nul_are_one_enter() {
        let p = cisco();
        let mut s = logged_in(&p);
        let out = send(&mut s, "\r\n\r\0\n");
        assert_eq!(out.matches("core-sw-01>").count(), 3, "{out:?}");
    }

    #[test]
    fn exit_from_exec_closes_and_reload_returns_to_login() {
        let p = cisco();
        let mut s = enabled(&p);
        let out = send(&mut s, "reload\r\n");
        assert!(out.ends_with(&p.username_prompt));
        assert_eq!(s.mode(), &Mode::User);
        let mut s = logged_in(&p);
        send(&mut s, "exit\r\n");
        assert!(s.is_closed());
    }

    #[test]
    fn iface_normalization() {
        let c = cisco();
        let r = ruijie();
        assert_eq!(
            normalize_iface(&c, &["Gi1/0/1"]).unwrap(),
            "GigabitEthernet1/0/1"
        );
        assert_eq!(normalize_iface(&c, &["po", "1"]).unwrap(), "Port-channel1");
        assert_eq!(
            normalize_iface(&r, &["te0/49"]).unwrap(),
            "TenGigabitEthernet 0/49"
        );
        assert_eq!(
            normalize_iface(&r, &["GigabitEthernet", "0/24"]).unwrap(),
            "GigabitEthernet 0/24"
        );
        assert!(normalize_iface(&r, &["xyz0/1"]).is_none());
        assert!(normalize_iface(&r, &["gi"]).is_none());
    }
}
