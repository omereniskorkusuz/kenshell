//! Device profile: declarative description of a vendor CLI (prompts, messages, show outputs).
//! Profiles live in `profiles/*.toml`; built-ins are embedded at compile time.

use serde::Deserialize;

/// Minimum CLI mode a profile command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MinMode {
    #[default]
    User,
    Privileged,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Prompts {
    /// e.g. `{host}>`
    pub user: String,
    /// e.g. `{host}#`
    pub privileged: String,
    /// e.g. `{host}(config)#`
    pub config: String,
    /// Interface sub-mode; `{iface}` = normalized interface name.
    pub interface: String,
    /// Generic sub-mode; `{sub}` = sub-mode suffix such as `config-vlan`.
    pub submode: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Messages {
    /// Printed below the caret line for unknown commands.
    pub invalid_input: String,
    /// `{cmd}` is replaced with the typed text.
    pub ambiguous: String,
    pub incomplete: String,
    pub login_failed: String,
    pub enable_failed: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubMode {
    /// First keyword, e.g. `vlan`.
    pub keyword: String,
    /// Prompt suffix, e.g. `config-vlan`.
    pub suffix: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    /// Full command words, e.g. `["show", "ip", "interface", "brief"]`.
    pub words: Vec<String>,
    #[serde(default)]
    pub min_mode: MinMode,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    pub vendor: String,
    pub hostname: String,
    #[serde(default)]
    pub banner: String,
    #[serde(default)]
    pub motd: String,
    pub username: String,
    pub password: String,
    pub enable_password: Option<String>,
    pub username_prompt: String,
    pub password_prompt: String,
    pub more_prompt: String,
    /// Default `terminal length` (0 = no paging).
    pub terminal_length: usize,
    /// `true` → Ruijie style `GigabitEthernet 0/1`, `false` → Cisco style `GigabitEthernet0/1`.
    pub iface_space: bool,
    /// Full interface type names in abbreviation priority order.
    pub iface_types: Vec<String>,
    pub reload_text: String,
    pub prompts: Prompts,
    pub messages: Messages,
    #[serde(default)]
    pub submodes: Vec<SubMode>,
    #[serde(default)]
    pub commands: Vec<Command>,
}

/// Built-in profiles: (name, TOML source).
pub const BUILTIN: &[(&str, &str)] = &[
    ("cisco-ios", include_str!("../profiles/cisco-ios.toml")),
    ("ruijie-rgos", include_str!("../profiles/ruijie-rgos.toml")),
];

impl Profile {
    pub fn parse(src: &str) -> Result<Self, String> {
        toml::from_str(src).map_err(|e| e.to_string())
    }

    /// Loads a built-in profile by name (or unique prefix, e.g. `cisco`, `ruijie`).
    pub fn builtin(name: &str) -> Result<Self, String> {
        let hits: Vec<_> = BUILTIN
            .iter()
            .filter(|(n, _)| n.starts_with(name))
            .collect();
        match hits.as_slice() {
            [(_, src)] => Self::parse(src),
            [] => Err(format!("unknown profile '{name}'")),
            _ => Err(format!("ambiguous profile '{name}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_profiles_parse() {
        for (name, src) in BUILTIN {
            let p = Profile::parse(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(&p.name, name);
            assert!(!p.commands.is_empty());
        }
    }

    #[test]
    fn builtin_by_prefix() {
        assert_eq!(Profile::builtin("ruijie").unwrap().vendor, "Ruijie");
        assert!(Profile::builtin("nope").is_err());
    }
}
