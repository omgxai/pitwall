//! Agent discovery (M5a).
//!
//! Which AI agents exist here and how Pitwall may use them. Detection is
//! PATH existence plus a *validated* integration surface per agent —
//! validated means the non-interactive invocation and model discovery
//! were actually observed (see module docs), never assumed from a name.
//!
//! Deliberately small: exactly the agents M5 supports. hermes is not an
//! executable (user project dir), aider is absent on this machine, and
//! gemini/crush/omp/pi/copilot/grok have unvalidated CLIs — all excluded
//! until their surfaces are verified, not merely present.

use std::path::{Path, PathBuf};

/// How confidently Pitwall may use an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// Binary + non-interactive path + model discovery all observed.
    High,
    /// Binary + non-interactive flags observed; model list unvalidated
    /// (falls back to the agent's own default).
    Medium,
}

/// How Pitwall learns an agent's models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelDiscovery {
    /// Run this subcommand (argv, no shell) and read `provider/model` lines.
    Command(&'static str),
    /// No verified list surface: use the agent's configured default.
    AgentDefault,
}

/// Static integration record for one agent. `binary` is the executable
/// name; `non_interactive` documents the verified invocation shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentStatic {
    pub id: &'static str,
    pub binary: &'static str,
    pub availability: Availability,
    pub non_interactive: &'static str,
    pub models: ModelDiscovery,
}

/// Exactly the M5-supported set. Add rows only with observed evidence.
pub const KNOWN: &[AgentStatic] = &[
    AgentStatic {
        id: "opencode",
        binary: "opencode",
        availability: Availability::High,
        non_interactive: "opencode run --format json",
        models: ModelDiscovery::Command("models"),
    },
    AgentStatic {
        id: "claude",
        binary: "claude",
        availability: Availability::Medium,
        non_interactive: "claude -p",
        models: ModelDiscovery::AgentDefault,
    },
    AgentStatic {
        id: "codex",
        binary: "codex",
        availability: Availability::Medium,
        non_interactive: "codex exec",
        models: ModelDiscovery::AgentDefault,
    },
];

/// A known agent plus where (if anywhere) it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub id: &'static str,
    pub path: Option<PathBuf>,
    pub availability: Availability,
    pub non_interactive: &'static str,
    pub models: ModelDiscovery,
}

/// Directories to search, split from `dirs` (production passes PATH).
/// Hermetic by construction: tests pass temp dirs, never the live PATH.
pub fn discover_in(dirs: &[PathBuf]) -> Vec<AgentInfo> {
    KNOWN
        .iter()
        .map(|a| AgentInfo {
            id: a.id,
            path: find_binary(dirs, a.binary),
            availability: a.availability,
            non_interactive: a.non_interactive,
            models: a.models,
        })
        .collect()
}

/// Directories from the live `PATH` environment variable.
pub fn path_dirs() -> Vec<PathBuf> {
    std::env::var_os("PATH").map_or(Vec::new(), |paths| std::env::split_paths(&paths).collect())
}

fn find_binary(dirs: &[PathBuf], binary: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join(binary))
        .find(|p| is_executable_file(p))
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|md| md.is_file() && md.permissions().mode() & 0o111 != 0)
}

/// List models for an agent id. Only `Command`-level agents execute
/// anything (fixed argv, captured output, capped lines); the rest report
/// the honest fallback. Unknown ids are refused.
pub fn models_for(agent_id: &str, dirs: &[PathBuf]) -> Result<Vec<String>, String> {
    const MAX_MODELS: usize = 128;
    let known = KNOWN
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("unknown agent '{agent_id}'"))?;
    let subcommand = match known.models {
        ModelDiscovery::Command(sub) => sub,
        ModelDiscovery::AgentDefault => {
            return Err(format!(
                "agent '{agent_id}' exposes no verified model list; use the agent default"
            ));
        }
    };
    let bin = find_binary(dirs, known.binary)
        .ok_or_else(|| format!("agent '{agent_id}' is not installed"))?;
    let output = std::process::Command::new(&bin)
        .arg(subcommand)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("failed to run model discovery: {e}"))?;
    if !output.status.success() {
        return Err(format!("model discovery exited {}", output.status));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let models: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_MODELS)
        .map(str::to_string)
        .collect();
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn fake_bin_dir(names_with_script: &[(&str, Option<&str>)]) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m5a-agents-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, body) in names_with_script {
            let path = dir.join(name);
            std::fs::write(&path, body.unwrap_or("#!/bin/sh\nexit 0\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // A non-executable decoy and an unknown binary must both be ignored.
        std::fs::write(dir.join("notexec"), "x").unwrap();
        std::fs::write(dir.join("mystery-ai"), "#!/bin/sh\n").unwrap();
        dir
    }

    #[test]
    fn discovery_reports_known_agents_only() {
        let dir = fake_bin_dir(&[("opencode", None), ("claude", None)]);
        let found = discover_in(std::slice::from_ref(&dir));
        assert_eq!(found.len(), 3, "known table is fixed");
        let oc = found.iter().find(|a| a.id == "opencode").unwrap();
        assert_eq!(oc.path.as_deref(), Some(dir.join("opencode").as_path()));
        assert_eq!(oc.availability, Availability::High);
        let cl = found.iter().find(|a| a.id == "claude").unwrap();
        assert!(cl.path.is_some());
        assert_eq!(cl.availability, Availability::Medium);
        let cx = found.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(cx.path, None, "absent here");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_executables_and_unknown_names_are_ignored() {
        let dir = fake_bin_dir(&[]);
        // `mystery-ai` is executable but not KNOWN; `notexec` is not executable.
        let found = discover_in(std::slice::from_ref(&dir));
        assert!(found.iter().all(|a| a.path.is_none()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_parses_command_output() {
        let dir = fake_bin_dir(&[(
            "opencode",
            Some("#!/bin/sh\necho 'prov/a'\necho ''\necho 'prov/b'\n"),
        )]);
        let models = models_for("opencode", std::slice::from_ref(&dir)).unwrap();
        assert_eq!(models, vec!["prov/a".to_string(), "prov/b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn models_refuses_gracefully() {
        let dir = fake_bin_dir(&[("claude", None)]);
        // Agent-default agents have no list surface.
        let err = models_for("claude", std::slice::from_ref(&dir)).unwrap_err();
        assert!(err.contains("agent default"));
        // Unknown ids are refused, not executed.
        let err = models_for("evil;id", std::slice::from_ref(&dir)).unwrap_err();
        assert!(err.contains("unknown agent"));
        // Known-with-command but absent binary.
        let err = models_for("opencode", std::slice::from_ref(&dir)).unwrap_err();
        assert!(err.contains("not installed"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
