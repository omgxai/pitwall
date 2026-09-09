//! Snapshot rendering: machine JSON (stable API boundary) + human text.
//!
//! `status --json` output is the contract the future panel/runtime will
//! consume: `schema_version` bump on breaking change, fixed key order,
//! explicit `null` (never omitted) for unknown values.

use crate::collector::WorkspaceSnapshot;
use crate::store::Checkpoint;

/// Escape a string for JSON double-quote embedding.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn q(s: &str) -> String {
    format!("\"{}\"", escape(s))
}

fn opt_q(v: Option<&str>) -> String {
    v.map_or("null".to_string(), q)
}

fn opt_bool(v: Option<bool>) -> String {
    v.map_or("null".to_string(), |b| b.to_string())
}

/// Render a snapshot as compact machine-readable JSON (schema v1).
pub fn snapshot_to_json(s: &WorkspaceSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"schema_version\":{},\"collected_at\":{},\"hostname\":{},\"session_count\":{},\"sessions\":[",
        s.schema_version,
        s.collected_at_epoch,
        q(&s.hostname),
        s.sessions.len()
    ));
    for (i, sess) in s.sessions.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let window = sess.window.as_ref().map_or("null".to_string(), |w| {
            format!(
                "{{\"address\":{},\"class\":{},\"initial_class\":{},\"title\":{},\"workspace\":{},\"pid\":{}}}",
                q(&w.address),
                q(&w.class),
                q(&w.initial_class),
                q(&w.title),
                q(&w.workspace),
                w.pid
            )
        });
        let project = sess.project.as_ref().map_or("null".to_string(), |p| {
            format!(
                "{{\"id\":{},\"dir\":{},\"name\":{},\"is_git_repo\":{},\"branch\":{},\"git_clean\":{}}}",
                q(&p.id),
                q(&p.dir),
                q(&p.name),
                p.is_git_repo,
                opt_q(p.branch.as_deref()),
                opt_bool(p.git_clean)
            )
        });
        let mut evidence = String::from("[");
        for (j, e) in sess.agent.evidence.iter().enumerate() {
            if j > 0 {
                evidence.push(',');
            }
            evidence.push_str(&q(e));
        }
        evidence.push(']');
        let mut processes = String::from("[");
        for (j, p) in sess.processes.iter().enumerate() {
            if j > 0 {
                processes.push(',');
            }
            processes.push_str(&format!(
                "{{\"pid\":{},\"ppid\":{},\"name\":{},\"command\":{},\"cwd\":{},\"state\":{},\"started_at\":{}}}",
                p.pid,
                p.ppid,
                q(&p.name),
                q(&p.command),
                q(&p.cwd),
                q(p.state.as_str()),
                p.started_at_epoch
            ));
        }
        processes.push(']');
        out.push_str(&format!(
            "{{\"id\":{},\"root_pid\":{},\"state\":{},\"process_count\":{},\"window\":{},\"project\":{},\"agent\":{{\"kind\":{},\"confidence\":{},\"evidence\":{}}},\"last_activity\":{{\"epoch\":{},\"kind\":{}}},\"summary\":{},\"processes\":{},\"role\":{}}}",
            q(&sess.id),
            sess.root_pid,
            q(sess.state.as_str()),
            sess.process_count,
            window,
            project,
            q(sess.agent.kind.as_str()),
            q(sess.agent.confidence.as_str()),
            evidence,
            sess.last_activity_epoch,
            q(sess.last_activity_kind),
            q(&sess.summary),
            processes,
            q(sess.role.as_str())
        ));
    }
    out.push_str("]}");
    out
}

/// Render a snapshot for humans (`pitwall status`).
pub fn snapshot_to_text(s: &WorkspaceSnapshot) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "pitwall status — {} session(s) on {}\n",
        s.sessions.len(),
        s.hostname
    ));
    for sess in &s.sessions {
        out.push_str(&format!("  [{}] {}\n", sess.id, sess.summary));
        if let Some(p) = &sess.project {
            out.push_str(&format!("    project: {} ({})\n", p.dir, p.id));
        }
        if let Some(w) = &sess.window {
            out.push_str(&format!(
                "    window:  {} {} pid={}\n",
                w.address, w.class, w.pid
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// state.json artifact (M2): SEPARATE interface from `status --json`.
// ---------------------------------------------------------------------------
//
// `status --json` is the live CLI read API (schema v1, includes ephemeral
// per-process detail for debugging). `state.json` is the small, versioned,
// machine-readable artifact for future M3 panel consumption. Both are
// generated from the same [`WorkspaceSnapshot`], but neither is derived
// from the other's bytes: the panel must never depend on CLI formatting.
//
// state.json contract (state_version 2):
// - everything v1 had, byte-identical in shape (v1 readers ignore `resumable`)
// - plus `resumable`: newest-first checkpoints for vanished sessions
//   (cap RESUMABLE_CAP), each scrubbed like the rest: checkpoint_id,
//   session_id (opaque local id, needed for resume-by-id), project_id,
//   project_dir, project_name (basename), branch, git_clean,
//   agent_kind, state (normalized), last_activity_epoch, created_at,
//   note, trigger. No PIDs, cmdlines, evidence, or secrets.
pub const STATE_SCHEMA_VERSION: u32 = 3;

/// Display status of the `summary` object in state.json. The panel uses
/// this to distinguish no-summary-yet (`None` object), ready, failed, and
/// disabled states. Error messages are fixed short strings — never agent
/// internals, terminal content, or command lines.
pub mod summary_status {
    pub const READY: &str = "ready";
    pub const ERROR: &str = "error";
    pub const UNAVAILABLE: &str = "unavailable";
}

/// A persistable-ready AI summary for state.json. Only final text plus
/// minimal metadata — the same boundary as the `summaries` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateSummary {
    pub text: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub input_hash: String,
    pub status: &'static str,
    pub message: Option<String>,
}

impl StateSummary {
    pub fn ready(
        text: String,
        model: Option<String>,
        created_at: i64,
        input_hash: String,
    ) -> StateSummary {
        StateSummary {
            text,
            model,
            created_at,
            input_hash,
            status: summary_status::READY,
            message: None,
        }
    }

    pub fn error(message: &'static str) -> StateSummary {
        StateSummary {
            text: String::new(),
            model: None,
            created_at: 0,
            input_hash: String::new(),
            status: summary_status::ERROR,
            message: Some(message.to_string()),
        }
    }
}

/// Cap on exposed resumable entries (panel shows a compact list, not history).
pub const RESUMABLE_CAP: usize = 10;

/// Normalize a stored state string to the known session-state vocabulary;
/// corrupt/foreign values become "unknown" rather than breaking readers.
pub fn normalize_state(state: &str) -> &'static str {
    match state {
        "running" => "running",
        "sleeping" => "sleeping",
        "stopped" => "stopped",
        _ => "unknown",
    }
}

fn project_basename(dir: &str) -> &str {
    dir.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(dir)
}

/// Render a snapshot as the `state.json` artifact (state schema v2).
pub fn snapshot_to_state_json(
    s: &WorkspaceSnapshot,
    resumable: &[Checkpoint],
    summary: Option<&StateSummary>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"state_version\":{},\"collected_at\":{},\"hostname\":{},\"session_count\":{},\"sessions\":[",
        STATE_SCHEMA_VERSION,
        s.collected_at_epoch,
        q(&s.hostname),
        s.sessions.len()
    ));
    for (i, sess) in s.sessions.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let project = sess.project.as_ref().map_or("null".to_string(), |p| {
            format!(
                "{{\"id\":{},\"dir\":{},\"name\":{},\"is_git_repo\":{},\"branch\":{},\"git_clean\":{}}}",
                q(&p.id),
                q(&p.dir),
                q(&p.name),
                p.is_git_repo,
                opt_q(p.branch.as_deref()),
                opt_bool(p.git_clean)
            )
        });
        let window = sess.window.as_ref().map_or("null".to_string(), |w| {
            format!(
                "{{\"address\":{},\"class\":{},\"title\":{},\"workspace\":{}}}",
                q(&w.address),
                q(&w.class),
                q(&w.title),
                q(&w.workspace)
            )
        });
        out.push_str(&format!(
            "{{\"id\":{},\"state\":{},\"process_count\":{},\"project\":{},\"agent\":{{\"kind\":{},\"confidence\":{}}},\"window\":{},\"last_activity\":{{\"epoch\":{},\"kind\":{}}},\"summary\":{},\"role\":{}}}",
            q(&sess.id),
            q(sess.state.as_str()),
            sess.process_count,
            project,
            q(sess.agent.kind.as_str()),
            q(sess.agent.confidence.as_str()),
            window,
            sess.last_activity_epoch,
            q(sess.last_activity_kind),
            q(&sess.summary),
            q(sess.role.as_str())
        ));
    }
    out.push_str("],\"resumable\":[");
    for (i, cp) in resumable.iter().take(RESUMABLE_CAP).enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"checkpoint_id\":{},\"session_id\":{},\"project_id\":{},\"project_dir\":{},\"project_name\":{},\"branch\":{},\"git_clean\":{},\"agent_kind\":{},\"state\":{},\"last_activity_epoch\":{},\"created_at\":{},\"note\":{},\"trigger\":{}}}",
            cp.id,
            q(&cp.session_id),
            q(&cp.project_id),
            q(&cp.project_dir),
            q(project_basename(&cp.project_dir)),
            opt_q(cp.branch.as_deref()),
            opt_bool(cp.git_clean),
            q(&cp.agent_kind),
            q(normalize_state(&cp.state)),
            cp.last_activity_epoch,
            cp.created_at,
            opt_q(cp.note.as_deref()),
            q(&cp.trigger)
        ));
    }
    out.push_str("],");
    match summary {
        Some(sum) => {
            out.push_str(&format!(
                "\"summary\":{{\"text\":{},\"model\":{},\"created_at\":{},\"input_hash\":{},\"status\":{},\"message\":{}}}",
                q(&sum.text),
                opt_q(sum.model.as_deref()),
                sum.created_at,
                q(&sum.input_hash),
                q(sum.status),
                opt_q(sum.message.as_deref())
            ));
        }
        None => out.push_str("\"summary\":null"),
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{
        AgentIdentity, AgentKind, Confidence, ProcessInfo, ProcessState, ProjectInfo, SessionState,
        TerminalSession, WindowRole, WorkspaceSnapshot, LAST_ACTIVITY_KIND,
    };
    use crate::platform::WindowInfo;

    fn sample_snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "test\"box".to_string(),
            sessions: vec![TerminalSession {
                id: "sess_abc".to_string(),
                window: Some(WindowInfo {
                    address: "0x1".to_string(),
                    class: "foot".to_string(),
                    initial_class: "foot".to_string(),
                    title: "t\nitle".to_string(),
                    workspace: "1".to_string(),
                    pid: 10,
                }),
                root_pid: 10,
                role: WindowRole::Terminal,
                project: Some(ProjectInfo {
                    id: "proj_def".to_string(),
                    dir: "/home/u/Work".to_string(),
                    name: "Work".to_string(),
                    is_git_repo: false,
                    branch: None,
                    git_clean: None,
                }),
                agent: AgentIdentity {
                    kind: AgentKind::Unknown,
                    confidence: Confidence::Unknown,
                    evidence: Vec::new(),
                },
                state: SessionState::Sleeping,
                process_count: 1,
                processes: vec![ProcessInfo {
                    pid: 10,
                    ppid: 1,
                    name: "bash".to_string(),
                    command: "/bin/bash".to_string(),
                    exe_name: "bash".to_string(),
                    cwd: "/home/u/Work".to_string(),
                    state: ProcessState::Sleeping,
                    started_at_epoch: 1_700_000_000,
                }],
                last_activity_epoch: 1_700_000_000,
                last_activity_kind: LAST_ACTIVITY_KIND,
                summary: "unknown [unknown] on Work (no branch) · sleeping · 1 proc".to_string(),
            }],
        }
    }

    #[test]
    fn json_escapes_control_chars_and_quotes() {
        let json = snapshot_to_json(&sample_snapshot());
        assert!(json.contains("\"hostname\":\"test\\\"box\""), "{json}");
        assert!(json.contains("\"title\":\"t\\nitle\""), "{json}");
        assert!(json.contains("\"branch\":null"), "{json}");
        assert!(json.contains("\"git_clean\":null"), "{json}");
        assert!(json.contains("\"kind\":\"latest_child_start\""), "{json}");
        assert!(json.starts_with("{\"schema_version\":1"), "{json}");
    }

    #[test]
    fn text_render_contains_session_line() {
        let text = snapshot_to_text(&sample_snapshot());
        assert!(text.contains("1 session(s) on"));
        assert!(text.contains("sess_abc"));
    }

    #[test]
    fn state_artifact_is_separate_versioned_and_scrubbed() {
        let mut snap = sample_snapshot();
        snap.sessions[0].processes[0].command = "opencode --token hunter2-supersecret".to_string();
        snap.sessions[0].agent.evidence = vec!["cmd:opencode (pid 10)".to_string()];
        let state = snapshot_to_state_json(&snap, &[], None);
        assert!(state.starts_with("{\"state_version\":3"), "{state}");
        assert!(state.contains("\"summary\":"), "{state}");
        assert!(state.contains("\"resumable\":[]"), "{state}");
        assert!(
            !state.contains("hunter2-supersecret"),
            "cmdline leaked: {state}"
        );
        assert!(!state.contains("--token"), "cmdline leaked: {state}");
        assert!(
            !state.contains("evidence"),
            "evidence strings excluded: {state}"
        );
        assert!(
            !state.contains("\"processes\""),
            "process detail excluded: {state}"
        );
        assert!(!state.contains("\"pid\""), "pids excluded: {state}");
    }

    #[test]
    fn state_v3_carries_ready_summary() {
        let snap = sample_snapshot();
        let sum = StateSummary::ready(
            "All quiet.".to_string(),
            Some("prov/model".to_string()),
            1_700_000_300,
            "fnv:abc123".to_string(),
        );
        let state = snapshot_to_state_json(&snap, &[], Some(&sum));
        assert!(
            state.contains("\"summary\":{\"text\":\"All quiet.\""),
            "{state}"
        );
        assert!(state.contains("\"status\":\"ready\""), "{state}");
        assert!(state.contains("\"model\":\"prov/model\""), "{state}");
        assert!(state.contains("\"input_hash\":\"fnv:abc123\""), "{state}");
    }

    #[test]
    fn state_v3_error_state_is_short_and_safe() {
        let snap = sample_snapshot();
        let sum = StateSummary::error("timeout");
        let state = snapshot_to_state_json(&snap, &[], Some(&sum));
        assert!(state.contains("\"status\":\"error\""), "{state}");
        assert!(state.contains("\"message\":\"timeout\""), "{state}");
        assert!(state.contains("\"text\":\"\""), "{state}");
    }

    /// String-aware JSON well-formedness: balanced {}[] outside strings,
    /// valid escapes, no trailing garbage. Catches brace bugs that
    /// substring assertions miss (e.g. a doubled closing brace).
    fn assert_well_formed(json: &str) {
        let mut stack: Vec<char> = Vec::new();
        let mut chars = json.chars().peekable();
        let mut in_str = false;
        while let Some(c) = chars.next() {
            if in_str {
                if c == '\\' {
                    chars.next();
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => stack.push('}'),
                '[' => stack.push(']'),
                '}' | ']' => assert_eq!(stack.pop(), Some(c), "unbalanced in {json}"),
                _ => {}
            }
        }
        assert!(!in_str, "unterminated string in {json}");
        assert!(stack.is_empty(), "unclosed brackets in {json}");
    }

    #[test]
    fn all_artifacts_are_well_formed_json() {
        let snap = sample_snapshot();
        let cps = vec![sample_checkpoint(1, "sess_x")];
        let sum = StateSummary::ready("Hi.".to_string(), None, 1, "fnv:1".to_string());
        assert_well_formed(&snapshot_to_json(&snap));
        assert_well_formed(&snapshot_to_state_json(&snap, &[], None));
        assert_well_formed(&snapshot_to_state_json(&snap, &cps, None));
        assert_well_formed(&snapshot_to_state_json(&snap, &cps, Some(&sum)));
        assert_well_formed(&snapshot_to_state_json(
            &snap,
            &cps,
            Some(&StateSummary::error("timeout")),
        ));
    }

    #[test]
    fn state_v3_absent_summary_is_null() {
        let snap = sample_snapshot();
        let state = snapshot_to_state_json(&snap, &[], None);
        assert!(state.contains("\"summary\":null"), "{state}");
    }

    fn sample_checkpoint(id: i64, session: &str) -> Checkpoint {
        Checkpoint {
            id,
            created_at: 1_700_000_200 + id,
            project_id: "proj_def".to_string(),
            session_id: session.to_string(),
            project_dir: "/home/u/Work".to_string(),
            branch: Some("main".to_string()),
            git_clean: Some(false),
            agent_kind: "opencode".to_string(),
            agent_confidence: "high".to_string(),
            state: "stopped".to_string(),
            last_activity_epoch: 1_700_000_001,
            window_address: Some("0x1".to_string()),
            window_class: Some("foot".to_string()),
            note: Some("halfway through auth".to_string()),
            trigger: "disappearance".to_string(),
            observation_id: Some(7),
        }
    }

    #[test]
    fn state_v2_preserves_v1_shape_and_adds_resumable() {
        let snap = sample_snapshot();
        let cps = vec![
            sample_checkpoint(2, "sess_old"),
            sample_checkpoint(1, "sess_older"),
        ];
        let state = snapshot_to_state_json(&snap, &cps, None);
        // v1 shape intact (newest-first resumable is purely additive).
        assert!(state.contains("\"hostname\":\"test\\\"box\""), "{state}");
        assert!(state.contains("\"sessions\":[{"), "{state}");
        let rpos = state.find("\"resumable\":[").expect("resumable key");
        let body = &state[rpos..];
        assert!(
            body.find("\"checkpoint_id\":2").unwrap() < body.find("\"checkpoint_id\":1").unwrap()
        );
        assert!(body.contains("\"project_name\":\"Work\""), "{body}");
        assert!(body.contains("\"session_id\":\"sess_old\""), "{body}");
        assert!(body.contains("\"note\":\"halfway through auth\""), "{body}");
        assert!(!body.contains("\"pid\""), "{body}");
        assert!(!body.contains("evidence"), "{body}");
    }

    #[test]
    fn state_v2_caps_and_normalizes_resumable() {
        let snap = sample_snapshot();
        let mut cps = Vec::new();
        for i in 0..(RESUMABLE_CAP + 5) {
            let mut cp = sample_checkpoint(i as i64, &format!("sess_{i}"));
            cp.state = "flying".to_string(); // corrupt/foreign state
            cps.push(cp);
        }
        let state = snapshot_to_state_json(&snap, &cps, None);
        let rpos = state.find("\"resumable\":[").unwrap();
        let body = &state[rpos..];
        assert_eq!(body.matches("\"checkpoint_id\"").count(), RESUMABLE_CAP);
        assert!(!body.contains("\"flying\""), "foreign states normalized");
        assert!(body.contains("\"state\":\"unknown\""), "{body}");
    }
}
