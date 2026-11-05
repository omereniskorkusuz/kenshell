//! Vendor CLI profiles: prompt → mode and vendor identification (P3-T02).
//!
//! The table is `src/data/modes.json` (extensible data, not code). The DeviceObserver
//! (P3-T03) matches the current last output line against the session's profile and the
//! banner / `show version` text against every profile's identification clues.

use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

const DATA: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../src/data/modes.json"
));

/// CLI mode shown by the mode colour (frontend `CliMode` minus `unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CliMode {
    User,
    Privileged,
    Config,
    Boot,
    Shell,
}

#[derive(Deserialize)]
struct RawPrompt {
    mode: CliMode,
    re: String,
}

#[derive(Deserialize)]
struct RawClue {
    re: String,
    weight: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPaging {
    more: String,
    disable: String,
}

#[derive(Deserialize)]
struct RawIdentity {
    command: String,
    fields: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct RawProfile {
    id: String,
    label: String,
    family: String,
    identify: Vec<RawClue>,
    prompts: Vec<RawPrompt>,
    paging: RawPaging,
    identity: RawIdentity,
}

/// What `show version` (or the vendor's equivalent) told about the device.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceFacts {
    pub hostname: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub serial: Option<String>,
    pub uptime: Option<String>,
}

impl DeviceFacts {
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.version.is_none() && self.serial.is_none()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawData {
    min_score: u32,
    default_profile: String,
    reload: Vec<String>,
    profiles: Vec<RawProfile>,
}

/// One vendor CLI profile with compiled patterns.
pub struct Profile {
    pub id: String,
    pub label: String,
    /// `cisco`, `ruijie`, … (`generic` for the fallback).
    pub family: String,
    /// Paging prompt (`--More--`) and the command that disables paging (P3-T04/T14).
    #[cfg_attr(not(test), allow(dead_code))]
    pub more: String,
    #[cfg_attr(not(test), allow(dead_code))]
    pub disable_paging: String,
    /// Read-only command that prints the device identity.
    pub identity_command: String,
    prompts: Vec<(CliMode, Regex)>,
    clues: Vec<(Regex, u32)>,
    /// Field name → patterns (`v` = value, optional `h` = host name).
    fields: Vec<(String, Vec<Regex>)>,
}

/// A recognised prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptMatch {
    pub mode: CliMode,
    pub host: Option<String>,
    /// Sub-mode inside the parentheses, e.g. `config-if-GigabitEthernet 0/24`.
    pub sub: Option<String>,
}

struct Table {
    profiles: Vec<Profile>,
    default: usize,
    min_score: u32,
    reload: regex::RegexSet,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let raw: RawData = serde_json::from_str(DATA).expect("src/data/modes.json is valid");
        let profiles: Vec<Profile> = raw
            .profiles
            .into_iter()
            .map(|p| Profile {
                prompts: p
                    .prompts
                    .into_iter()
                    .map(|x| (x.mode, Regex::new(&x.re).expect("prompt pattern")))
                    .collect(),
                clues: p
                    .identify
                    .into_iter()
                    .map(|c| {
                        let re = RegexBuilder::new(&c.re)
                            .case_insensitive(true)
                            .build()
                            .expect("identify pattern");
                        (re, c.weight)
                    })
                    .collect(),
                identity_command: p.identity.command,
                fields: p
                    .identity
                    .fields
                    .into_iter()
                    .map(|(name, pats)| {
                        let res = pats
                            .iter()
                            .map(|re| Regex::new(re).expect("identity field pattern"))
                            .collect();
                        (name, res)
                    })
                    .collect(),
                more: p.paging.more,
                disable_paging: p.paging.disable,
                id: p.id,
                label: p.label,
                family: p.family,
            })
            .collect();
        let default = profiles
            .iter()
            .position(|p| p.id == raw.default_profile)
            .expect("default profile exists");
        Table {
            profiles,
            default,
            min_score: raw.min_score,
            reload: regex::RegexSet::new(&raw.reload).expect("reload patterns"),
        }
    })
}

impl Profile {
    /// Mode of `line` if it is a prompt of this profile (`line` without ANSI codes).
    pub fn match_prompt(&self, line: &str) -> Option<PromptMatch> {
        self.prompts.iter().find_map(|(mode, re)| {
            let caps = re.captures(line)?;
            let text = |name: &str| caps.name(name).map(|m| m.as_str().to_owned());
            Some(PromptMatch {
                mode: *mode,
                host: text("host"),
                sub: text("sub"),
            })
        })
    }

    /// Parses the identity command's output (lines matched one by one, first hit wins).
    pub fn parse_facts(&self, text: &str) -> DeviceFacts {
        let mut f = DeviceFacts::default();
        for (name, res) in &self.fields {
            let hit = text.lines().find_map(|line| {
                let line = line.trim_end_matches('\r');
                res.iter().find_map(|re| re.captures(line))
            });
            let Some(caps) = hit else { continue };
            let v = caps.name("v").map(|m| m.as_str().trim().to_owned());
            if let Some(h) = caps.name("h") {
                f.hostname.get_or_insert_with(|| h.as_str().to_owned());
            }
            match name.as_str() {
                "model" => f.model = v,
                "version" => f.version = v,
                "serial" => f.serial = v,
                "uptime" => f.uptime = v,
                "hostname" => f.hostname = v.or(f.hostname.take()),
                _ => {}
            }
        }
        f
    }

    fn score(&self, text: &str) -> u32 {
        self.clues
            .iter()
            .filter(|(re, _)| re.is_match(text))
            .map(|(_, w)| w)
            .sum()
    }
}

/// The fallback profile ("Cisco-like CLI") used until the vendor is known.
pub fn default_profile() -> &'static Profile {
    let t = table();
    &t.profiles[t.default]
}

/// Profile by id (stored device vendors, P3-T04).
#[cfg_attr(not(test), allow(dead_code))]
pub fn profile(id: &str) -> Option<&'static Profile> {
    table().profiles.iter().find(|p| p.id == id)
}

/// True when a completed output line means the device is restarting.
pub fn is_reload_line(line: &str) -> bool {
    table().reload.is_match(line)
}

/// The vendor profile best supported by `text` (banner, login, `show version`), or `None`
/// when no profile has enough evidence — Ruijie is never guessed as Cisco from prompts.
pub fn identify(text: &str) -> Option<&'static Profile> {
    let t = table();
    t.profiles
        .iter()
        .map(|p| (p.score(text), p))
        .filter(|(s, _)| *s >= t.min_score)
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/data/mode-cases.json"
    ));

    #[derive(Deserialize)]
    struct PromptCase {
        profile: String,
        line: String,
        mode: Option<CliMode>,
        host: Option<String>,
        sub: Option<String>,
    }

    #[derive(Deserialize)]
    struct IdentifyCase {
        text: String,
        profile: Option<String>,
    }

    #[derive(Deserialize)]
    struct ReloadCase {
        line: String,
        reload: bool,
    }

    #[derive(Deserialize)]
    struct FactsCase {
        profile: String,
        text: String,
        facts: DeviceFacts,
    }

    #[derive(Deserialize)]
    struct Cases {
        prompts: Vec<PromptCase>,
        identify: Vec<IdentifyCase>,
        reload: Vec<ReloadCase>,
        facts: Vec<FactsCase>,
    }

    fn cases() -> Cases {
        serde_json::from_str(CASES).unwrap()
    }

    #[test]
    fn prompts_map_to_modes() {
        for c in cases().prompts {
            let p = profile(&c.profile).unwrap_or_else(|| panic!("profile {}", c.profile));
            let got = p.match_prompt(&c.line);
            assert_eq!(got.as_ref().map(|m| m.mode), c.mode, "{:?}", c.line);
            if let Some(m) = got {
                assert_eq!(m.host, c.host, "{:?}", c.line);
                assert_eq!(m.sub, c.sub, "{:?}", c.line);
            }
        }
    }

    #[test]
    fn vendors_are_identified_from_text_only() {
        for c in cases().identify {
            assert_eq!(
                identify(&c.text).map(|p| p.id.clone()),
                c.profile,
                "{:?}",
                c.text
            );
        }
    }

    #[test]
    fn show_version_outputs_parse() {
        for c in cases().facts {
            let p = profile(&c.profile).unwrap();
            assert_eq!(p.parse_facts(&c.text), c.facts, "{}", c.profile);
        }
        assert_eq!(
            profile("cisco-ios").unwrap().identity_command,
            "show version"
        );
    }

    #[test]
    fn reload_lines() {
        for c in cases().reload {
            assert_eq!(is_reload_line(&c.line), c.reload, "{:?}", c.line);
        }
    }

    #[test]
    fn ruijie_is_not_cisco_and_default_is_cisco_like() {
        let ruijie = identify("System description : Ruijie ... RGOS 11.4").unwrap();
        assert_eq!(ruijie.family, "ruijie");
        assert_eq!(default_profile().id, "cisco-like");
        assert_eq!(default_profile().disable_paging, "terminal length 0");
        assert_eq!(default_profile().more, "--More--");
        assert_eq!(default_profile().label, "Cisco-like CLI");
        // Every profile's patterns compile and the generic prompt shapes work everywhere.
        for p in &table().profiles {
            assert_eq!(
                p.match_prompt("sw1#").map(|m| m.mode),
                Some(CliMode::Privileged),
                "{}",
                p.id
            );
        }
    }
}
