//! DeviceObserver (P3-T03): watches a session's output and reports the CLI mode, the
//! device identity and reloads, without keeping the output.
//!
//! It keeps only the current last line (≤ 512 characters, ANSI removed) and, until the
//! vendor is known, up to 16 KB of completed lines for identification. After every chunk
//! the current line is matched against the session's vendor profile: a prompt such as
//! `sw1(config)#` gives the mode immediately (`conf t` → config, `end` → privileged).

use serde::{Deserialize, Serialize};

use super::modes::{self, CliMode, Profile};

const MAX_LINE: usize = 512;
const MAX_BANNER: usize = 16 * 1024;

/// What a session learned about its device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The prompt changed mode or sub-mode.
    Mode { mode: CliMode, sub: Option<String> },
    /// Host name from the prompt and/or vendor from banner / `show version`.
    Identity {
        hostname: Option<String>,
        vendor: Option<VendorInfo>,
    },
    /// Boot messages: the device restarts (a serial session stays up).
    Reload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorInfo {
    /// Profile id, e.g. `ruijie-rgos`.
    pub id: String,
    pub label: String,
    /// `cisco`, `ruijie`, …
    pub family: String,
}

impl VendorInfo {
    fn of(p: &Profile) -> Self {
        Self {
            id: p.id.clone(),
            label: p.label.clone(),
            family: p.family.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Esc {
    None,
    /// After ESC.
    Start,
    /// CSI `ESC [` … final byte.
    Csi,
    /// OSC `ESC ]` … BEL or ST.
    Osc,
    OscEnd,
}

pub struct Observer {
    profile: &'static Profile,
    identified: bool,
    esc: Esc,
    /// A CR was seen; a following LF ends the line, anything else overwrites it.
    cr: bool,
    line: String,
    banner: String,
    mode: Option<CliMode>,
    sub: Option<String>,
    host: Option<String>,
    reloading: bool,
}

impl Default for Observer {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer {
    pub fn new() -> Self {
        Self {
            profile: modes::default_profile(),
            identified: false,
            esc: Esc::None,
            cr: false,
            line: String::new(),
            banner: String::new(),
            mode: None,
            sub: None,
            host: None,
            reloading: false,
        }
    }

    /// The vendor profile in use (`cisco-like` until identified; identity command P3-T04).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn profile(&self) -> &'static Profile {
        self.profile
    }

    /// A new connection: the mode is unknown again until the next prompt (the device
    /// and its vendor stay the same).
    pub fn reconnected(&mut self) {
        self.esc = Esc::None;
        self.cr = false;
        self.line.clear();
        self.mode = None;
        self.sub = None;
        self.reloading = false;
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Observation> {
        let mut out = Vec::new();
        let text = String::from_utf8_lossy(bytes);
        let banner_before = self.banner.len();
        for ch in text.chars() {
            self.feed_char(ch, &mut out);
        }
        if !self.identified && self.banner.len() > banner_before {
            if let Some(p) = modes::identify(&self.banner) {
                self.profile = p;
                self.identified = true;
                self.banner = String::new();
                out.push(Observation::Identity {
                    hostname: None,
                    vendor: Some(VendorInfo::of(p)),
                });
            } else if self.banner.len() >= MAX_BANNER {
                // Nothing conclusive in the first 16 KB: stop collecting (stay cisco-like).
                self.identified = true;
                self.banner = String::new();
            }
        }
        self.check_prompt(&mut out);
        out
    }

    fn feed_char(&mut self, ch: char, out: &mut Vec<Observation>) {
        match self.esc {
            Esc::Start => {
                self.esc = match ch {
                    '[' => Esc::Csi,
                    ']' => Esc::Osc,
                    _ => Esc::None, // two-character escape
                };
                return;
            }
            Esc::Csi => {
                if ('@'..='~').contains(&ch) {
                    self.esc = Esc::None;
                }
                return;
            }
            Esc::Osc => {
                match ch {
                    '\x07' => self.esc = Esc::None,
                    '\x1b' => self.esc = Esc::OscEnd,
                    _ => {}
                }
                return;
            }
            Esc::OscEnd => {
                self.esc = if ch == '\\' { Esc::None } else { Esc::Osc };
                return;
            }
            Esc::None => {}
        }
        if self.cr {
            self.cr = false;
            if ch != '\n' {
                self.line.clear(); // carriage return without line feed: overwrite
            }
        }
        match ch {
            '\x1b' => self.esc = Esc::Start,
            '\n' => self.end_line(out),
            '\r' => self.cr = true,
            '\x08' => {
                self.line.pop();
            }
            c if c.is_control() => {}
            c => {
                if self.line.len() < MAX_LINE {
                    self.line.push(c);
                }
            }
        }
    }

    fn end_line(&mut self, out: &mut Vec<Observation>) {
        let line = std::mem::take(&mut self.line);
        if !self.reloading && modes::is_reload_line(&line) {
            self.reloading = true;
            // The next prompt must be reported even if it shows the same mode as before.
            self.mode = None;
            self.sub = None;
            out.push(Observation::Reload);
        }
        if !self.identified && self.banner.len() < MAX_BANNER {
            self.banner.push_str(&line);
            self.banner.push('\n');
        }
    }

    fn check_prompt(&mut self, out: &mut Vec<Observation>) {
        let candidate = self.line.trim_end();
        if candidate.is_empty() {
            return;
        }
        let Some(m) = self.profile.match_prompt(candidate) else {
            return;
        };
        if self.mode != Some(m.mode) || self.sub != m.sub {
            self.mode = Some(m.mode);
            self.sub.clone_from(&m.sub);
            self.reloading = false;
            out.push(Observation::Mode {
                mode: m.mode,
                sub: m.sub,
            });
        }
        if m.host.is_some() && m.host != self.host {
            self.host.clone_from(&m.host);
            out.push(Observation::Identity {
                hostname: m.host,
                vendor: None,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes_of(obs: &[Observation]) -> Vec<(CliMode, Option<String>)> {
        obs.iter()
            .filter_map(|o| match o {
                Observation::Mode { mode, sub } => Some((*mode, sub.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn conf_t_and_end_switch_modes_at_once() {
        let mut o = Observer::new();
        let first = o.feed(b"\r\ncore-sw-01>");
        assert_eq!(modes_of(&first), [(CliMode::User, None)]);
        assert!(first.contains(&Observation::Identity {
            hostname: Some("core-sw-01".into()),
            vendor: None
        }));
        // Echo of the typed command: no change while typing.
        assert!(o.feed(b"en").is_empty());
        assert!(o.feed(b"able\r\nPassword:").is_empty());
        assert_eq!(
            modes_of(&o.feed(b"\r\ncore-sw-01#")),
            [(CliMode::Privileged, None)]
        );
        o.feed(b"conf t\r\nEnter configuration commands, one per line.\r\n");
        assert_eq!(
            modes_of(&o.feed(b"core-sw-01(config)#")),
            [(CliMode::Config, Some("config".into()))]
        );
        o.feed(b"int gi 0/24\r\n");
        assert_eq!(
            modes_of(&o.feed(b"core-sw-01(config-if)#")),
            [(CliMode::Config, Some("config-if".into()))]
        );
        assert_eq!(
            modes_of(&o.feed(b"end\r\ncore-sw-01#")),
            [(CliMode::Privileged, None)]
        );
        // Same prompt again: nothing new.
        assert!(o.feed(b"\r\ncore-sw-01#").is_empty());
    }

    #[test]
    fn colours_split_escapes_and_overwrites_do_not_hide_the_prompt() {
        let mut o = Observer::new();
        // Prompt in bold, split inside the escape sequence.
        assert!(o.feed(b"\x1b[1").is_empty());
        let obs = o.feed(b"msw1\x1b[0m#");
        assert_eq!(modes_of(&obs), [(CliMode::Privileged, None)]);
        // --More-- erased with CR + spaces + CR, then a prompt.
        o.feed(b"\r\n --More-- ");
        let obs = o.feed(b"\r          \rsw1(config)#");
        assert_eq!(modes_of(&obs), [(CliMode::Config, Some("config".into()))]);
        // Backspaces.
        let obs = o.feed(b"\r\nsw1(config)#x\x08");
        assert!(modes_of(&obs).is_empty(), "same mode");
    }

    #[test]
    fn ruijie_banner_switches_the_profile_and_keeps_spaced_submodes() {
        let mut o = Observer::new();
        let obs = o.feed(
            b"System description      : Ruijie Full Gigabit Access Switch By Ruijie Networks\r\n\
              System software version : S5750_RGOS 11.4(1)B12P10S2\r\naccess-sw-02#",
        );
        assert!(obs.iter().any(|x| matches!(x,
            Observation::Identity { vendor: Some(v), .. } if v.family == "ruijie")));
        assert_eq!(o.profile().id, "ruijie-rgos");
        o.feed(b"int gi 0/24\r\n");
        assert_eq!(
            modes_of(&o.feed(b"access-sw-02(config-if-GigabitEthernet 0/24)#")),
            [(
                CliMode::Config,
                Some("config-if-GigabitEthernet 0/24".into())
            )]
        );
    }

    #[test]
    fn prompts_alone_never_claim_a_vendor() {
        let mut o = Observer::new();
        o.feed(b"User Access Verification\r\n\r\nUsername: admin\r\nPassword:\r\nsw1>");
        assert_eq!(o.profile().id, "cisco-like");
    }

    #[test]
    fn reload_then_prompt() {
        let mut o = Observer::new();
        o.feed(b"sw1#");
        let obs = o.feed(b"reload\r\nProceed with reload? [confirm]\r\n");
        assert_eq!(obs, [Observation::Reload]);
        assert!(
            o.feed(b"System Bootstrap, Version 15.2\r\n").is_empty(),
            "one reload event per restart"
        );
        o.feed(b"rommon 1 >\r\n"); // completed line: not a prompt at the end
        let obs = o.feed(b"Press RETURN to get started!\r\n\r\nsw1>");
        assert_eq!(modes_of(&obs), [(CliMode::User, None)]);
        o.reconnected();
        assert_eq!(
            modes_of(&o.feed(b"\r\nsw1>")),
            [(CliMode::User, None)],
            "mode reported again after a reconnect"
        );
    }

    #[test]
    fn huge_lines_and_binary_noise_stay_bounded() {
        let mut o = Observer::new();
        o.feed(&vec![b'x'; 100_000]);
        assert!(o.line.len() <= MAX_LINE);
        let noise: Vec<u8> = (0..=255u8).cycle().take(50_000).collect();
        o.feed(&noise);
        assert!(o.banner.len() <= MAX_BANNER + MAX_LINE + 1);
    }
}
