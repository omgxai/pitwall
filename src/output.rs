//! Snapshot rendering: machine JSON (stable API boundary) + human text.
//!
//! `status --json` output is the contract the future panel/runtime will
//! consume: `schema_version` bump on breaking change, fixed key order,
//! explicit `null` (never omitted) for unknown values.

use crate::collector::WorkspaceSnapshot;
use crate::store::Checkpoint;
use std::collections::HashMap;

/// Config echo for state.json: the *effective* user choices the panel
/// displays and acts on (agent/model/summary toggle). Rendered from the
/// config file at state-write time; defaults when unconfigured.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConfigEcho {
    pub agent: String,
    pub model: String,
    pub summary_enabled: bool,
}

/// Per-session enrichment for timeline bars, computed from retained
/// observations (observed duration + state history). Absent map entries
/// render as unknown/empty — never invented.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionMeta {
    pub age_secs: Option<i64>,
    pub history: String,
}

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
/// Display tier for presentation grouping (semantic priority, NOT identity).
/// Order is fixed downstream: pitwall-native > agents > workspace > system.
/// `pitwall-native` is a documented name heuristic for this product's home
/// project (gracefully empty for other users, who start at agents).
/// Unknown agents are never promoted: they stay workspace/app-labeled.
pub fn tier_for(
    project_name: Option<&str>,
    project_dir: Option<&str>,
    agent_kind: &str,
) -> &'static str {
    let haystack = format!(
        "{} {}",
        project_name.unwrap_or_default(),
        project_dir.unwrap_or_default()
    )
    .to_lowercase();
    if haystack.contains("pitwall") {
        return "pitwall-native";
    }
    if !agent_kind.is_empty() && agent_kind != "unknown" {
        return "agents";
    }
    "workspace"
}

/// Presentation group key: shared project id when a project exists,
/// otherwise the session/checkpoint's own id (ungrouped singleton).
/// Identity untouched — grouping only.
pub fn group_for(project_id: Option<&str>, session_id: &str) -> String {
    project_id.unwrap_or(session_id).to_string()
}

pub fn snapshot_to_state_json(
    s: &WorkspaceSnapshot,
    resumable: &[Checkpoint],
    summary: Option<&StateSummary>,
    meta: &HashMap<String, SessionMeta>,
    config: &ConfigEcho,
    notifications: &[crate::store::Notification],
    unread_count: i64,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"state_version\":{},\"collected_at\":{},\"hostname\":{},\"session_count\":{},\"config\":{{\"agent\":{},\"model\":{},\"summary_enabled\":{}}},\"sessions\":[",
        STATE_SCHEMA_VERSION,
        s.collected_at_epoch,
        q(&s.hostname),
        s.sessions.len(),
        q(&config.agent),
        q(&config.model),
        config.summary_enabled
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
        // Timeline enrichment (additive; unknown when unobserved).
        let meta_entry = meta.get(&sess.id);
        out.push_str(&format!(
            "{{\"id\":{},\"state\":{},\"process_count\":{},\"project\":{},\"agent\":{{\"kind\":{},\"confidence\":{}}},\"window\":{},\"last_activity\":{{\"epoch\":{},\"kind\":{}}},\"summary\":{},\"role\":{},\"age_secs\":{},\"history\":{},\"root_pid\":{},\"tier\":{},\"group\":{}}}",
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
            q(sess.role.as_str()),
            meta_entry.and_then(|m| m.age_secs).map_or("null".to_string(), |a| a.to_string()),
            q(meta_entry.map(|m| m.history.as_str()).unwrap_or("")),
            sess.root_pid,
            q(tier_for(
                sess.project.as_ref().map(|p| p.name.as_str()),
                sess.project.as_ref().map(|p| p.dir.as_str()),
                sess.agent.kind.as_str()
            )),
            q(&group_for(
                sess.project.as_ref().map(|p| p.id.as_str()),
                &sess.id
            ))
        ));
    }
    out.push_str("],\"resumable\":[");
    let mut seen: std::collections::HashSet<&str> = s
        .sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    for (i, cp) in resumable
        .iter()
        .filter(|cp| seen.insert(cp.session_id.as_str()))
        .take(RESUMABLE_CAP)
        .enumerate()
    {
        if i > 0 {
            out.push(',');
        }
        let cp_name = project_basename(&cp.project_dir);
        out.push_str(&format!(
            "{{\"checkpoint_id\":{},\"session_id\":{},\"project_id\":{},\"project_dir\":{},\"project_name\":{},\"branch\":{},\"git_clean\":{},\"agent_kind\":{},\"agent_confidence\":{},\"state\":{},\"last_activity_epoch\":{},\"created_at\":{},\"note\":{},\"trigger\":{},\"tier\":{},\"group\":{}}}",
            cp.id,
            q(&cp.session_id),
            q(&cp.project_id),
            q(&cp.project_dir),
            q(cp_name),
            opt_q(cp.branch.as_deref()),
            opt_bool(cp.git_clean),
            q(&cp.agent_kind),
            q(&cp.agent_confidence),
            q(normalize_state(&cp.state)),
            cp.last_activity_epoch,
            cp.created_at,
            opt_q(cp.note.as_deref()),
            q(&cp.trigger),
            q(tier_for(Some(cp_name), Some(cp.project_dir.as_str()), cp.agent_kind.as_str())),
            q(&group_for(Some(cp.project_id.as_str()), &cp.session_id))
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
    out.push_str(",\"notifications\":[");
    for (i, n) in notifications.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"id\":{},\"kind\":{},\"session_id\":{},\"project\":{},\"agent\":{},\"state\":{},\"severity\":{},\"detail\":{},\"created_at\":{}}}",
            n.id,
            q(&n.kind),
            q(&n.session_id),
            q(&n.project_name),
            q(&n.agent_kind),
            q(&n.state),
            q(&n.severity),
            q(&n.detail),
            n.created_at
        ));
    }
    out.push_str(&format!("],\"unread_count\":{}", unread_count));
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
        let state = snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
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
        let state = snapshot_to_state_json(
            &snap,
            &[],
            Some(&sum),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
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
        let state = snapshot_to_state_json(
            &snap,
            &[],
            Some(&sum),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        assert!(state.contains("\"status\":\"error\""), "{state}");
        assert!(state.contains("\"message\":\"timeout\""), "{state}");
        assert!(state.contains("\"text\":\"\""), "{state}");
    }

    /// String-aware JSON well-formedness: balanced {}[] outside strings,
    /// valid escapes, no trailing garbage. Catches brace bugs that
    /// substring assertions miss (e.g. a doubled closing brace).
    /// NOTE: bracket balance alone does NOT catch missing commas
    /// (`}{` balances!). Use [`assert_valid_json`] for real validation.
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

    /// Strict JSON validator (recursive descent): objects/arrays with
    /// comma discipline, no trailing commas, complete consumption. The
    /// bracket-balance checker above is blind to missing commas, which
    /// broke a live state.json once — this one is not.
    struct JsonParser<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl<'a> JsonParser<'a> {
        fn new(s: &'a str) -> JsonParser<'a> {
            JsonParser {
                bytes: s.as_bytes(),
                pos: 0,
            }
        }
        fn ws(&mut self) {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
        }
        fn lit(&mut self, s: &str) -> bool {
            if self.bytes[self.pos..].starts_with(s.as_bytes()) {
                self.pos += s.len();
                true
            } else {
                false
            }
        }
        fn string(&mut self) -> bool {
            if self.bytes.get(self.pos) != Some(&b'"') {
                return false;
            }
            self.pos += 1;
            while self.pos < self.bytes.len() {
                match self.bytes[self.pos] {
                    b'"' => {
                        self.pos += 1;
                        return true;
                    }
                    b'\\' => self.pos += 2,
                    _ => self.pos += 1,
                }
            }
            false
        }
        fn number(&mut self) -> bool {
            let start = self.pos;
            if self.bytes.get(self.pos) == Some(&b'-') {
                self.pos += 1;
            }
            let digits = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == digits {
                return false;
            }
            if self.bytes.get(self.pos) == Some(&b'.') {
                self.pos += 1;
                let f = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                if self.pos == f {
                    return false;
                }
            }
            if matches!(self.bytes.get(self.pos), Some(b'e') | Some(b'E')) {
                self.pos += 1;
                if matches!(self.bytes.get(self.pos), Some(b'+') | Some(b'-')) {
                    self.pos += 1;
                }
                let f = self.pos;
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                if self.pos == f {
                    return false;
                }
            }
            self.pos > start
        }
        fn value(&mut self) -> bool {
            self.ws();
            if self.string() || self.number() {
                return true;
            }
            if self.lit("true") || self.lit("false") || self.lit("null") {
                return true;
            }
            if self.bytes.get(self.pos) == Some(&b'{') {
                self.pos += 1;
                self.ws();
                if self.bytes.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return true;
                }
                loop {
                    self.ws();
                    if !self.string() {
                        return false;
                    }
                    self.ws();
                    if self.bytes.get(self.pos) != Some(&b':') {
                        return false;
                    }
                    self.pos += 1;
                    if !self.value() {
                        return false;
                    }
                    self.ws();
                    match self.bytes.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return true;
                        }
                        _ => return false,
                    }
                }
            }
            if self.bytes.get(self.pos) == Some(&b'[') {
                self.pos += 1;
                self.ws();
                if self.bytes.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return true;
                }
                loop {
                    if !self.value() {
                        return false;
                    }
                    self.ws();
                    match self.bytes.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return true;
                        }
                        _ => return false,
                    }
                }
            }
            false
        }
        fn document(&mut self) -> bool {
            let ok = self.value();
            self.ws();
            ok && self.pos == self.bytes.len()
        }
    }

    fn assert_valid_json(json: &str) {
        assert!(JsonParser::new(json).document(), "invalid JSON: {json}");
    }

    #[test]
    fn strict_validator_rejects_missing_commas() {
        assert_valid_json("{\"a\":1,\"b\":[1,2],\"c\":null}");
        assert!(!JsonParser::new("{\"a\":1\"b\":2}").document());
        assert!(!JsonParser::new("[1,2,]").document());
        assert!(!JsonParser::new("{\"a\":}").document());
        assert!(!JsonParser::new("").document());
    }

    #[test]
    fn all_artifacts_are_well_formed_json() {
        let snap = sample_snapshot();
        let cps = vec![sample_checkpoint(1, "sess_x")];
        let sum = StateSummary::ready("Hi.".to_string(), None, 1, "fnv:1".to_string());
        assert_well_formed(&snapshot_to_json(&snap));
        assert_well_formed(&snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        ));
        assert_well_formed(&snapshot_to_state_json(
            &snap,
            &cps,
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        ));
        assert_well_formed(&snapshot_to_state_json(
            &snap,
            &cps,
            Some(&sum),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        ));
        assert_well_formed(&snapshot_to_state_json(
            &snap,
            &cps,
            Some(&StateSummary::error("timeout")),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        ));
        let full = snapshot_to_state_json(
            &snap,
            &cps,
            Some(&sum),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[sample_notification(9, "stopped", "attention")],
            1,
        );
        assert_valid_json(&full);
    }

    #[test]
    fn timeline_fields_come_from_meta_or_unknown() {
        let snap = sample_snapshot();
        // No meta: unknown age, empty history, real root pid.
        let bare = snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        assert!(bare.contains("\"age_secs\":null"), "{bare}");
        assert!(bare.contains("\"history\":\"\""), "{bare}");
        assert!(bare.contains("\"root_pid\":10"), "{bare}");
        // With meta: rendered verbatim.
        let mut meta = HashMap::new();
        meta.insert(
            "sess_abc".to_string(),
            SessionMeta {
                age_secs: Some(3600),
                history: "RRS".to_string(),
            },
        );
        let enriched =
            snapshot_to_state_json(&snap, &[], None, &meta, &ConfigEcho::default(), &[], 0);
        assert!(enriched.contains("\"age_secs\":3600"), "{enriched}");
        assert!(enriched.contains("\"history\":\"RRS\""), "{enriched}");
        assert_well_formed(&enriched);
    }

    #[test]
    fn tier_and_group_follow_documented_rules() {
        // pitwall-native: product home project surfaces first.
        assert_eq!(
            tier_for(
                Some("pitwall"),
                Some("/home/u/Projects/pitwall"),
                "opencode"
            ),
            "pitwall-native"
        );
        assert_eq!(tier_for(Some("PITWALL"), None, "unknown"), "pitwall-native");
        // Known agents tier above plain workspace.
        assert_eq!(
            tier_for(Some("Work"), Some("/home/u/Work"), "opencode"),
            "agents"
        );
        assert_eq!(
            tier_for(Some("Work"), Some("/home/u/Work"), "claude"),
            "agents"
        );
        // Unknown agents never promote, even with a project.
        assert_eq!(
            tier_for(Some("Work"), Some("/home/u/Work"), "unknown"),
            "workspace"
        );
        assert_eq!(tier_for(None, None, ""), "workspace");
        // Group: shared project id, else own session id (no merging).
        assert_eq!(group_for(Some("proj_x"), "sess_a"), "proj_x");
        assert_eq!(group_for(None, "sess_a"), "sess_a");
    }

    #[test]
    fn writer_carries_tier_and_group() {
        let snap = sample_snapshot();
        let state = snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        // sample project is /home/u/Work, unknown agent -> workspace tier.
        assert!(state.contains("\"tier\":\"workspace\""), "{state}");
        assert!(state.contains("\"group\":\"proj_def\""), "{state}");
        assert_well_formed(&state);
    }

    fn sample_notification(id: i64, kind: &str, severity: &str) -> crate::store::Notification {
        crate::store::Notification {
            id,
            kind: kind.to_string(),
            session_id: "sess_abc".to_string(),
            project_id: "proj_def".to_string(),
            project_name: "Work".to_string(),
            branch: None,
            agent_kind: "opencode".to_string(),
            state: "running".to_string(),
            checkpoint_id: None,
            created_at: 1_700_000_400,
            read_at: None,
            severity: severity.to_string(),
            detail: "opencode on Work".to_string(),
        }
    }

    #[test]
    fn notifications_render_with_badge_count() {
        let snap = sample_snapshot();
        let notifs = vec![
            sample_notification(1, "stopped", "attention"),
            sample_notification(2, "appeared", "informational"),
        ];
        let state = snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &notifs,
            1,
        );
        assert!(state.contains("\"notifications\":[{"), "{state}");
        assert!(state.contains("\"kind\":\"stopped\""), "{state}");
        assert!(state.contains("\"severity\":\"attention\""), "{state}");
        assert!(state.contains("\"unread_count\":1"), "{state}");
        assert_well_formed(&state);
    }

    #[test]
    fn config_echo_renders_effective_choices() {
        let snap = sample_snapshot();
        let cfg = ConfigEcho {
            agent: "codex".to_string(),
            model: "prov/m".to_string(),
            summary_enabled: false,
        };
        let state = snapshot_to_state_json(&snap, &[], None, &HashMap::new(), &cfg, &[], 0);
        assert!(state.contains("\"config\":{\"agent\":\"codex\""), "{state}");
        assert!(state.contains("\"summary_enabled\":false"), "{state}");
        assert_well_formed(&state);
    }

    #[test]
    fn state_v3_absent_summary_is_null() {
        let snap = sample_snapshot();
        let state = snapshot_to_state_json(
            &snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
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
        let state = snapshot_to_state_json(
            &snap,
            &cps,
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        // v1 shape intact (newest-first resumable is purely additive).
        assert!(state.contains("\"hostname\":\"test\\\"box\""), "{state}");
        assert!(state.contains("\"sessions\":[{"), "{state}");
        let rpos = state.find("\"resumable\":[").expect("resumable key");
        let body = &state[rpos..];
        assert!(
            body.find("\"checkpoint_id\":2").unwrap() < body.find("\"checkpoint_id\":1").unwrap()
        );
        assert!(body.contains("\"project_name\":\"Work\""), "{body}");
        assert!(body.contains("\"agent_confidence\":\"high\""), "{body}");
        assert!(body.contains("\"session_id\":\"sess_old\""), "{body}");
        assert!(body.contains("\"note\":\"halfway through auth\""), "{body}");
        assert!(!body.contains("\"pid\""), "{body}");
        assert!(!body.contains("evidence"), "{body}");
    }

    #[test]
    fn resumable_excludes_live_and_duplicate_sessions_before_cap() {
        let snap = sample_snapshot();
        let mut cps = vec![sample_checkpoint(100, "sess_abc")];
        cps.extend((0..20).map(|i| sample_checkpoint(i, "sess_old")));
        cps.push(sample_checkpoint(99, "sess_other"));
        let state = snapshot_to_state_json(
            &snap,
            &cps,
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        assert_eq!(state.matches("\"checkpoint_id\"").count(), 2);
        assert!(state.contains("\"checkpoint_id\":99"));
        assert!(!state.contains("\"checkpoint_id\":100"));
        assert_valid_json(&state);
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
        let state = snapshot_to_state_json(
            &snap,
            &cps,
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        let rpos = state.find("\"resumable\":[").unwrap();
        let body = &state[rpos..];
        assert_eq!(body.matches("\"checkpoint_id\"").count(), RESUMABLE_CAP);
        assert!(!body.contains("\"flying\""), "foreign states normalized");
        assert!(body.contains("\"state\":\"unknown\""), "{body}");
    }
}
