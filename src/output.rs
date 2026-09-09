//! Snapshot rendering: machine JSON (stable API boundary) + human text.
//!
//! `status --json` output is the contract the future panel/runtime will
//! consume: `schema_version` bump on breaking change, fixed key order,
//! explicit `null` (never omitted) for unknown values.

use crate::collector::WorkspaceSnapshot;

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
            "{{\"id\":{},\"root_pid\":{},\"state\":{},\"process_count\":{},\"window\":{},\"project\":{},\"agent\":{{\"kind\":{},\"confidence\":{},\"evidence\":{}}},\"last_activity\":{{\"epoch\":{},\"kind\":{}}},\"summary\":{},\"processes\":{}}}",
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
            processes
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{
        AgentIdentity, AgentKind, Confidence, ProcessInfo, ProcessState, ProjectInfo, SessionState,
        TerminalSession, WorkspaceSnapshot, LAST_ACTIVITY_KIND,
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
}
