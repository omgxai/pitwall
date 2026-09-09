//! Minimal user configuration (M5f settings persistence).
//!
//! Deliberately NOT a generic framework: three known keys for the Pitwall
//! settings popup (agent, model, summary toggle). Stored as `key=value`
//! lines in `~/.config/pitwall/config` (0600). Unknown keys are preserved
//! verbatim so forward-compatible readers never lose data.
//!
//! QML never touches this file directly: it goes through
//! `pitwall config get|set` (fixed argv) or the state.json `config` echo.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Known keys. Anything else passes through untouched.
pub const KEY_AGENT: &str = "agent";
pub const KEY_MODEL: &str = "model";
pub const KEY_SUMMARY_ENABLED: &str = "summary_enabled";

/// Defaults when no config exists: opencode, agent default model, on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub agent: String,
    pub model: String,
    pub summary_enabled: bool,
    extra: HashMap<String, String>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            agent: "opencode".to_string(),
            model: String::new(),
            summary_enabled: true,
            extra: HashMap::new(),
        }
    }
}

impl Config {
    /// Parse key=value lines (ignores blanks and `#` comments).
    pub fn parse(text: &str) -> Config {
        let mut cfg = Config::default();
        let mut extra = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                KEY_AGENT => cfg.agent = value.trim().to_string(),
                KEY_MODEL => cfg.model = value.trim().to_string(),
                KEY_SUMMARY_ENABLED => {
                    cfg.summary_enabled = !matches!(
                        value.trim().to_lowercase().as_str(),
                        "0" | "false" | "no" | "off"
                    )
                }
                other => {
                    extra.insert(other.to_string(), value.trim().to_string());
                }
            }
        }
        cfg.extra = extra;
        if cfg.agent.is_empty() {
            cfg.agent = "opencode".to_string();
        }
        cfg
    }

    /// Validate-and-set one known key. Unknown keys and empty agent/model
    /// values are refused (empty model means "agent default" via clear).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            KEY_AGENT => {
                if !crate::agents::KNOWN.iter().any(|a| a.id == value) {
                    return Err(format!("unknown agent '{value}'"));
                }
                self.agent = value.to_string();
                Ok(())
            }
            KEY_MODEL => {
                if value.is_empty() {
                    self.model.clear();
                    return Ok(());
                }
                if !crate::summary::valid_model(value) {
                    return Err("malformed model id (refusing)".to_string());
                }
                self.model = value.to_string();
                Ok(())
            }
            KEY_SUMMARY_ENABLED => {
                self.summary_enabled = !matches!(
                    value.trim().to_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                );
                Ok(())
            }
            _ => Err(format!("unknown config key '{key}'")),
        }
    }

    /// Serialize back (known keys first, extras preserved).
    pub fn serialize(&self) -> String {
        let mut out = String::from("# Pitwall user configuration (managed by `pitwall config`).\n");
        out.push_str(&format!("{}={}\n", KEY_AGENT, self.agent));
        out.push_str(&format!("{}={}\n", KEY_MODEL, self.model));
        out.push_str(&format!(
            "{}={}\n",
            KEY_SUMMARY_ENABLED,
            if self.summary_enabled {
                "true"
            } else {
                "false"
            }
        ));
        let mut extra: Vec<(&String, &String)> = self.extra.iter().collect();
        extra.sort();
        for (k, v) in extra {
            out.push_str(&format!("{k}={v}\n"));
        }
        out
    }
}

/// Config file path: `$XDG_CONFIG_HOME/pitwall/config`, else
/// `~/.config/pitwall/config`.
pub fn config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("pitwall/config");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".config/pitwall/config")
}

/// Load config from `path` (missing file = defaults). Never fails hard:
/// unreadable files yield defaults so the panel always works.
pub fn load_from(path: &Path) -> Config {
    std::fs::read_to_string(path).map_or_else(|_| Config::default(), |t| Config::parse(&t))
}

/// Save config atomically (temp + rename) with 0600 permissions.
pub fn save_to(path: &Path, cfg: &Config) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create config dir: {e}"))?;
        }
    }
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true).mode(0o600);
        use std::io::Write as _;
        let mut file = opts
            .open(&tmp)
            .map_err(|e| format!("cannot write config: {e}"))?;
        file.write_all(cfg.serialize().as_bytes())
            .map_err(|e| format!("cannot write config: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot install config: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_known_and_extra_keys() {
        let cfg = Config::parse("agent=codex\nmodel=a/b\nsummary_enabled=false\nfuture=x\n");
        assert_eq!(cfg.agent, "codex");
        assert_eq!(cfg.model, "a/b");
        assert!(!cfg.summary_enabled);
        let back = Config::parse(&cfg.serialize());
        assert_eq!(back, cfg);
    }

    #[test]
    fn defaults_and_garbage_are_safe() {
        let cfg = Config::parse("");
        assert_eq!(cfg, Config::default());
        let cfg = Config::parse("# comment\n\nno-equals-here\nagent=\n");
        assert_eq!(cfg.agent, "opencode");
        assert!(cfg.summary_enabled);
    }

    #[test]
    fn set_validates_against_known_agents_and_models() {
        let mut cfg = Config::default();
        assert!(cfg.set("agent", "opencode").is_ok());
        assert!(cfg.set("agent", "evil;id").is_err());
        assert!(cfg.set("model", "prov/model").is_ok());
        assert!(cfg.set("model", "").is_ok());
        assert_eq!(cfg.model, "");
        assert!(cfg.set("model", "bad model").is_err());
        assert!(cfg.set("summary_enabled", "false").is_ok());
        assert!(!cfg.summary_enabled);
        assert!(cfg.set("whatever", "1").is_err());
    }

    #[test]
    fn save_and_load_round_trip_with_private_perms() {
        let dir = std::env::temp_dir().join("pitwall-m5f-config-test");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config");
        let mut cfg = Config::default();
        cfg.set("agent", "codex").unwrap();
        save_to(&path, &cfg).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(load_from(&path), cfg);
        // Missing file reads as defaults.
        assert_eq!(load_from(&dir.join("nope")), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
