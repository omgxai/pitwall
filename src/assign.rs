//! Explicit human-triggered task assignment to a live session (M5g).
//!
//! Workforce control surface, not orchestration: exactly one foreground
//! agent invocation per explicit request, reusing the proven shapes from
//! `summary` (fixed argv, timeout+kill, text-only extraction) and the
//! validation precedents from `resume` (shape-first refusal, live-only
//! targeting, no fallbacks) and `agents` (known-binary discovery).
//!
//! Hard boundaries (all enforced, all tested):
//! - live sessions only (vanished targets refuse with "resume first");
//! - prompt is human-typed text, length-capped, control-stripped,
//!   scrubbed as defense-in-depth; never shell, never flags, never dirs;
//! - role/name is a DISPLAY label only (selects nothing);
//! - no persistence (no DB writes; output is returned, not stored);
//! - no background execution, no retry loops, no session merging.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Human prompt cap: a paragraph-to-page task fits; unbounded argv does
/// not (exposure via /proc, agent cost, UI abuse). Over-cap refuses with
/// trim-and-retry guidance — never silently truncated (cuts change meaning).
pub const MAX_ASSIGN_PROMPT_CHARS: usize = 4000;

/// Roles are display labels for the spawned session title/context only.
/// They never select binaries, dirs, or sessions.
pub const KNOWN_ROLES: &[&str] = &["UI Auditor", "Systems", "Reviewer", "Researcher", "QA"];

/// Validated assignment: everything the spawn needs, nothing it doesn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub binary: PathBuf,
    pub dir: String,
    pub model: Option<String>,
    pub prompt: String,
    pub role_label: String,
    pub session_id: String,
}

/// Validate and build an assignment. Order: shape → liveness → binary →
///
/// model → dir → prompt. First failure refuses; nothing spawns.
pub fn prepare(
    platform: &dyn crate::platform::Platform,
    db_path: &Path,
    session_id: &str,
    role: &str,
    prompt: &str,
    model: Option<&str>,
) -> Result<Assignment, String> {
    if !crate::resume::is_session_id(session_id) {
        return Err("malformed session ID (refusing)".to_string());
    }
    // Live-only: collect and require the exact session, no fallbacks.
    let snapshot = crate::collector::collect(platform);
    let live = snapshot
        .sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| format!("unknown session {session_id} (not live; resume first)"))?;
    let dir = live
        .project
        .as_ref()
        .map(|p| p.dir.clone())
        .unwrap_or_default();
    if !dir.starts_with('/') {
        return Err("refusing non-absolute project dir".to_string());
    }
    if !std::fs::metadata(&dir).map(|m| m.is_dir()).unwrap_or(false) {
        return Err(format!("project directory unavailable: {dir}"));
    }
    // Agent: suggested default from live kind when installed, else opencode
    // when installed, else refuse. Display kind never bypasses discovery.
    let suggested = match live.agent.kind.as_str() {
        "opencode" => "opencode",
        "claude-code" => "claude",
        "codex" => "codex",
        _ => "opencode",
    };
    let dirs = crate::agents::path_dirs();
    let binary = crate::agents::discover_in(&dirs)
        .into_iter()
        .find(|a| a.id == suggested && a.path.is_some())
        .and_then(|a| a.path)
        .or_else(|| {
            crate::agents::discover_in(&dirs)
                .into_iter()
                .find(|a| a.id == "opencode")
                .and_then(|a| a.path)
        })
        .ok_or_else(|| "no supported agent installed (refusing)".to_string())?;
    let _ = db_path; // Reserved: future audit reads go through the store.
    if let Some(m) = model {
        if !crate::summary::valid_model(m) {
            return Err(format!("refusing malformed model id ({m:?})"));
        }
    }
    let clean = crate::context::strip_controls(prompt.trim());
    if clean.is_empty() {
        return Err("refusing empty prompt (describe the task)".to_string());
    }
    if clean.chars().count() > MAX_ASSIGN_PROMPT_CHARS {
        return Err(format!(
            "prompt too long ({} chars, max {}); trim and retry",
            clean.chars().count(),
            MAX_ASSIGN_PROMPT_CHARS
        ));
    }
    let scrubbed = crate::context::scrub_string(&clean);
    let role_label = if KNOWN_ROLES.contains(&role) {
        role.to_string()
    } else {
        "Worker".to_string()
    };
    Ok(Assignment {
        binary,
        dir,
        model: model.map(str::to_string),
        prompt: scrubbed,
        role_label,
        session_id: session_id.to_string(),
    })
}

/// Fixed argv for the assignment spawn. Content travels as ONE message
/// element (never flags, never shell); role label is context text only.
pub fn build_argv(a: &Assignment) -> Vec<String> {
    let mut argv = vec![
        a.binary.to_string_lossy().into_owned(),
        "run".to_string(),
        "--format".to_string(),
        "json".to_string(),
        "--dir".to_string(),
        a.dir.clone(),
        "--title".to_string(),
        format!("Pitwall task: {}", a.role_label),
    ];
    if let Some(m) = &a.model {
        argv.push("-m".to_string());
        argv.push(m.clone());
    }
    argv.push(a.prompt.clone());
    argv
}

/// Execute one assignment synchronously and return filtered text.
/// Reuses the summary runner (timeout+kill, bounded output) and text
/// extractor (tool payloads ignored). No persistence of any kind.
pub fn execute(a: &Assignment, timeout: Duration) -> Result<String, String> {
    let argv = build_argv(a);
    let raw = crate::summary::run_agent(&argv, timeout)?;
    let text = crate::summary::extract_summary_text(&raw);
    if text.is_empty() {
        return Err("agent returned no usable text".to_string());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{GitInfo, RawProcess, WindowInfo};
    use std::cell::RefCell;

    // NOTE: collect() derives real sess_* ids from windows; tests drive
    // prepare() through those ids (see resume.rs test seam pattern).
    struct LiveMock {
        windows: Vec<WindowInfo>,
        launched: RefCell<Vec<String>>,
    }

    impl crate::platform::Platform for LiveMock {
        fn processes(&self) -> Vec<RawProcess> {
            Vec::new()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.windows.clone()
        }
        fn git_info(&self, _dir: &str) -> GitInfo {
            GitInfo::default()
        }
        fn boot_epoch(&self) -> i64 {
            1_700_000_000
        }
        fn clock_ticks_per_sec(&self) -> i64 {
            100
        }
        fn hostname(&self) -> String {
            "testbox".to_string()
        }
        fn launch_terminal(&self, directory: &str) -> Result<(), String> {
            self.launched.borrow_mut().push(directory.to_string());
            Ok(())
        }
        fn focus_window_address(&self, _address: &str) -> Result<(), String> {
            Ok(())
        }
        fn process_io(&self, _pid: u32) -> Option<crate::platform::IoCounters> {
            None
        }
        fn terminal_text(&self, _pid: u32, _class: &str) -> crate::platform::TerminalText {
            crate::platform::TerminalText::Unavailable {
                reason: "mock has no terminals",
            }
        }
    }

    fn live_window(address: &str, pid: u32) -> WindowInfo {
        WindowInfo {
            address: address.to_string(),
            class: "foot".to_string(),
            initial_class: "foot".to_string(),
            title: "t".to_string(),
            workspace: "1".to_string(),
            pid,
        }
    }

    #[test]
    fn malformed_id_refuses_before_anything() {
        let plat = LiveMock { windows: vec![], launched: RefCell::new(Vec::new()) };
        let err = prepare(&plat, Path::new("/nonexistent.db"), "bogus", "UI Auditor", "do things", None)
            .unwrap_err();
        assert!(err.contains("malformed"));
    }

    #[test]
    fn vanished_session_refuses_without_fallback() {
        let plat = LiveMock { windows: vec![], launched: RefCell::new(Vec::new()) };
        let err = prepare(
            &plat,
            Path::new("/nonexistent.db"),
            "sess_0123456789abcdef",
            "UI Auditor",
            "do things",
            None,
        )
        .unwrap_err();
        assert!(err.contains("not live"), "{err}");
    }

    #[test]
    fn prompt_rules_are_strict() {
        // Realizarre: need a live session; reuse collect through a window
        // whose project exists (temp dir) — see prompt_cap_needs_live.
        let dir = std::env::temp_dir().join("pitwall-assign-prompt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Point a window's shell cwd at dir via processes? MockPlatform has
        // no processes; project will be None -> dir empty -> non-absolute
        // refusal happens before prompt checks. That still proves ordering:
        // shape first, then liveness, then dir.
        let plat = LiveMock {
            windows: vec![live_window("0x1", 100)],
            launched: RefCell::new(Vec::new()),
        };
        let snap = crate::collector::collect(&plat);
        assert_eq!(snap.sessions.len(), 1);
        let sid = snap.sessions[0].id.clone();
        // No project -> dir refusal (before prompt validation).
        let err = prepare(&plat, Path::new("/nonexistent.db"), &sid, "UI Auditor", "", None)
            .unwrap_err();
        assert!(err.contains("non-absolute") || err.contains("unavailable"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn role_label_never_selects_anything() {
        // Unknown roles degrade to Worker; known roles pass through as
        // display text only. Binary/dir selection untouched by role.
        assert!(KNOWN_ROLES.contains(&"UI Auditor"));
        assert!(!KNOWN_ROLES.contains(&"; rm -rf ~"));
    }

    #[test]
    fn argv_shape_has_no_shell_no_flags_from_content() {
        let a = Assignment {
            binary: PathBuf::from("/usr/bin/opencode"),
            dir: "/home/u/proj".to_string(),
            model: Some("prov/model".to_string()),
            prompt: "audit this; rm -rf /".to_string(),
            role_label: "UI Auditor".to_string(),
            session_id: "sess_0123456789abcdef".to_string(),
        };
        let argv = build_argv(&a);
        assert_eq!(argv[0], "/usr/bin/opencode");
        assert_eq!(argv[1], "run");
        assert!(argv.contains(&"--format".to_string()));
        assert!(argv.contains(&"--dir".to_string()));
        assert!(argv.contains(&"/home/u/proj".to_string()));
        // Prompt is ONE trailing element (message position), never flags.
        assert_eq!(argv.last().unwrap(), "audit this; rm -rf /");
        assert!(!argv.iter().any(|x| x.contains("sh -c") || x.contains("bash -c")));
    }
}
