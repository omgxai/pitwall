//! Snapshot rendering: machine JSON (stable API boundary) + human text.
//!
//! `status --json` output is the contract the future panel/runtime will
//! consume: `schema_version` bump on breaking change, fixed key order,
//! explicit `null` (never omitted) for unknown values.

use crate::collector::{WindowRole, WorkspaceSnapshot};
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
//
// state.json contract (state_version 4, design §4.9/§5.3): everything v3
// had, unchanged in name, type and value, plus three additive keys that a
// v3 reader simply ignores (Requirement 22.1):
// - top-level `pitwall_version`: the product version for the panel footer
//   (Requirement 7.7), from [`crate::version`] so it cannot drift from
//   `Cargo.toml`;
// - per session `chat`: the observed [`crate::collector::ChatFacts`] as
//   `{ number, harness, model, context_label, context_session_id,
//   started_at }`, or `null` on every non-chat session;
// - `role` may now be `"chat"`, and such sessions carry
//   `tier: "pitwall-native"` + `group: "chat:sessions"` for the panel's
//   Chat Sessions region (Requirement 17.6).
// Chat fields carry no conversation text, environment values, command
// lines, window addresses or pids (Requirement 22.7) — `ChatFacts` has no
// such field, so the writer has no parameter that could smuggle one in.
// The `window.address` / `root_pid` keys on every session are pre-existing
// v1 fields, not chat fields, and stay exactly as they were.
pub const STATE_SCHEMA_VERSION: u32 = 4;

/// Presentation group key for chat sessions. The panel matches this string
/// exactly (`Widget.qml` `chatGroupKey`) to render one low-weight "Chat
/// Sessions" region; grouping stays presentation-only, `sess_*` / `proj_*`
/// identity is untouched.
pub const CHAT_GROUP_KEY: &str = "chat:sessions";

/// Display status of the `summary` object in state.json. The panel uses
/// this to distinguish no-summary-yet (`None` object), ready, failed, and
/// disabled states. Error messages are fixed short strings — never agent
/// internals, terminal content, or command lines.
pub mod summary_status {
    pub const READY: &str = "ready";
    pub const ERROR: &str = "error";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const STALE: &str = "stale";
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

    pub fn stale(input_hash: String, model: Option<String>, created_at: i64) -> StateSummary {
        StateSummary {
            text: String::new(),
            model,
            created_at,
            input_hash,
            status: summary_status::STALE,
            message: Some("workspace changed; generate a fresh summary".to_string()),
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

/// Display tier for a *session*, which is [`tier_for`] plus one rule: a
/// corroborated Pitwall Chat window is this product's own surface and always
/// sorts with `pitwall-native`, whatever project it happens to sit in
/// (Requirement 17.6). Every other role delegates unchanged, so the existing
/// tier rules (and their tests) are untouched.
pub fn tier_for_session(
    role: WindowRole,
    project_name: Option<&str>,
    project_dir: Option<&str>,
    agent_kind: &str,
) -> &'static str {
    if role == WindowRole::Chat {
        return "pitwall-native";
    }
    tier_for(project_name, project_dir, agent_kind)
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
    // `pitwall_version` (7.7) comes from the crate's single version source, so
    // the artifact can never disagree with Cargo.toml or the running binary.
    out.push_str(&format!(
        "{{\"state_version\":{},\"pitwall_version\":{},\"collected_at\":{},\"hostname\":{},\"session_count\":{},\"config\":{{\"agent\":{},\"model\":{},\"summary_enabled\":{}}},\"sessions\":[",
        STATE_SCHEMA_VERSION,
        q(crate::version()),
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
        // Observed chat facts (additive, v4); `null` on every non-chat
        // session. `number` is the three-digit *string* the panel's `chatOf`
        // gate requires; `context_session_id` stays `null` when it was not
        // observable — never invented (17.7).
        let chat = sess.chat.as_ref().map_or("null".to_string(), |c| {
            format!(
                "{{\"number\":{},\"harness\":{},\"model\":{},\"context_label\":{},\"context_session_id\":{},\"started_at\":{}}}",
                q(&c.number_text()),
                q(&c.harness),
                q(&c.model),
                q(&c.context_label),
                opt_q(c.context_session_id.as_deref()),
                c.started_at_epoch
            )
        });
        // Chat sessions land in the panel's single Chat Sessions region.
        // Keyed on the chat object (not just the role) so the group and the
        // reader's `chatOf` gate can never disagree; the collector sets
        // `role == Chat` exactly when `chat` is `Some`.
        let group = if sess.chat.is_some() {
            CHAT_GROUP_KEY.to_string()
        } else {
            group_for(sess.project.as_ref().map(|p| p.id.as_str()), &sess.id)
        };
        // Timeline enrichment (additive; unknown when unobserved).
        let meta_entry = meta.get(&sess.id);
        out.push_str(&format!(
            "{{\"id\":{},\"state\":{},\"process_count\":{},\"project\":{},\"agent\":{{\"kind\":{},\"confidence\":{}}},\"window\":{},\"last_activity\":{{\"epoch\":{},\"kind\":{}}},\"summary\":{},\"role\":{},\"age_secs\":{},\"history\":{},\"root_pid\":{},\"tier\":{},\"group\":{},\"chat\":{}}}",
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
            q(tier_for_session(
                sess.role,
                sess.project.as_ref().map(|p| p.name.as_str()),
                sess.project.as_ref().map(|p| p.dir.as_str()),
                sess.agent.kind.as_str()
            )),
            q(&group),
            chat
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
        AgentIdentity, AgentKind, ChatFacts, Confidence, ProcessInfo, ProcessState, ProjectInfo,
        SessionState, TerminalSession, WindowRole, WorkspaceSnapshot, LAST_ACTIVITY_KIND,
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
                chat: None,
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
        assert!(state.starts_with("{\"state_version\":4"), "{state}");
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

    #[test]
    fn state_v3_marks_changed_workspace_summary_stale() {
        let snap = sample_snapshot();
        let sum = StateSummary::stale("fnv:old".to_string(), None, 42);
        let state = snapshot_to_state_json(
            &snap,
            &[],
            Some(&sum),
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        );
        assert!(state.contains("\"status\":\"stale\""), "{state}");
        assert!(state.contains("workspace changed"), "{state}");
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

    /// A corroborated chat window, shaped like design §5.3's example: the
    /// collector sets `role == Chat` exactly when it recorded `ChatFacts`.
    fn chat_snapshot(context_session_id: Option<&str>) -> WorkspaceSnapshot {
        let mut snap = sample_snapshot();
        let sess = &mut snap.sessions[0];
        sess.role = WindowRole::Chat;
        sess.window.as_mut().expect("window").title =
            "Pitwall Chat 001 · opencode · muse-spark · Work".to_string();
        sess.chat = Some(ChatFacts {
            number: 1,
            harness: "opencode".to_string(),
            model: "muse-spark".to_string(),
            context_label: "Work".to_string(),
            context_session_id: context_session_id.map(str::to_string),
            started_at_epoch: 1_700_000_050,
        });
        sess.summary = "Pitwall Chat 001 · opencode · Work · sleeping".to_string();
        snap
    }

    fn state_of(snap: &WorkspaceSnapshot) -> String {
        snapshot_to_state_json(
            snap,
            &[],
            None,
            &HashMap::new(),
            &ConfigEcho::default(),
            &[],
            0,
        )
    }

    /// Slice out the emitted `chat` object so privacy assertions cannot be
    /// satisfied (or defeated) by the pre-existing session-level fields.
    fn chat_object(state: &str) -> String {
        let start = state.find("\"chat\":").expect("chat key") + "\"chat\":".len();
        let end = state[start..].find('}').expect("chat object end") + 1;
        state[start..start + end].to_string()
    }

    #[test]
    fn state_v4_emits_documented_chat_shape() {
        let state = state_of(&chat_snapshot(None));
        // Byte-for-byte the object in design §5.3: three-digit STRING number,
        // `null` for the context session id that is not observable.
        assert!(
            state.contains(
                "\"chat\":{\"number\":\"001\",\"harness\":\"opencode\",\"model\":\"muse-spark\",\"context_label\":\"Work\",\"context_session_id\":null,\"started_at\":1700000050}"
            ),
            "{state}"
        );
        assert!(state.contains("\"role\":\"chat\""), "{state}");
        assert!(state.contains("\"tier\":\"pitwall-native\""), "{state}");
        assert!(state.contains("\"group\":\"chat:sessions\""), "{state}");
        assert_valid_json(&state);
    }

    #[test]
    fn state_v4_chat_number_is_a_three_digit_string() {
        let mut snap = chat_snapshot(None);
        snap.sessions[0].chat.as_mut().expect("chat").number = 42;
        let state = state_of(&snap);
        assert!(state.contains("\"number\":\"042\""), "{state}");
        assert!(!state.contains("\"number\":42"), "integer number: {state}");
        assert_valid_json(&state);
    }

    #[test]
    fn state_v4_renders_observed_context_session_id_verbatim() {
        let state = state_of(&chat_snapshot(Some("sess_fedcba9876543210")));
        assert!(
            state.contains("\"context_session_id\":\"sess_fedcba9876543210\""),
            "{state}"
        );
        assert_valid_json(&state);
    }

    #[test]
    fn state_v4_chat_object_excludes_process_and_window_detail() {
        let mut snap = chat_snapshot(None);
        snap.sessions[0].processes[0].command =
            "pitwall chat --session sess_abc --token hunter2".to_string();
        let state = state_of(&snap);
        let chat = chat_object(&state);
        for banned in ["pid", "address", "command", "hunter2", "0x1", "env"] {
            assert!(!chat.contains(banned), "{banned} leaked into chat: {chat}");
        }
        assert!(!state.contains("hunter2"), "cmdline leaked: {state}");
        assert_valid_json(&state);
    }

    #[test]
    fn state_v4_non_chat_session_emits_null_chat() {
        let snap = sample_snapshot();
        assert!(snap.sessions[0].chat.is_none());
        let state = state_of(&snap);
        assert!(state.contains("\"chat\":null"), "{state}");
        // Non-chat sessions keep their v3 tier and project grouping.
        assert!(state.contains("\"tier\":\"workspace\""), "{state}");
        assert!(state.contains("\"group\":\"proj_def\""), "{state}");
        assert!(!state.contains("chat:sessions"), "{state}");
        assert_valid_json(&state);
    }

    #[test]
    fn state_v4_is_well_formed_in_both_chat_branches() {
        let cps = vec![sample_checkpoint(1, "sess_x")];
        let sum = StateSummary::ready("Hi.".to_string(), None, 1, "fnv:1".to_string());
        for snap in [
            sample_snapshot(),
            chat_snapshot(None),
            chat_snapshot(Some("sess_ctx")),
        ] {
            assert_valid_json(&state_of(&snap));
            assert_valid_json(&snapshot_to_state_json(
                &snap,
                &cps,
                Some(&sum),
                &HashMap::new(),
                &ConfigEcho::default(),
                &[sample_notification(9, "stopped", "attention")],
                1,
            ));
        }
    }

    #[test]
    fn tier_for_session_short_circuits_chat_and_delegates_otherwise() {
        // Chat outranks whatever project it sits in.
        assert_eq!(
            tier_for_session(
                WindowRole::Chat,
                Some("Work"),
                Some("/home/u/Work"),
                "unknown"
            ),
            "pitwall-native"
        );
        assert_eq!(
            tier_for_session(WindowRole::Chat, None, None, ""),
            "pitwall-native"
        );
        // Every other role delegates to the untouched tier_for, identically.
        let cases = [
            (Some("Work"), Some("/home/u/Work"), "unknown"),
            (Some("Work"), Some("/home/u/Work"), "opencode"),
            (Some("pitwall"), Some("/home/u/Projects/pitwall"), "claude"),
            (None, None, ""),
        ];
        for role in [WindowRole::Terminal, WindowRole::App, WindowRole::Unknown] {
            for (name, dir, agent) in cases {
                assert_eq!(
                    tier_for_session(role, name, dir, agent),
                    tier_for(name, dir, agent),
                    "role {role:?} must delegate for {name:?}/{dir:?}/{agent}"
                );
            }
        }
    }

    #[test]
    fn state_v4_carries_pitwall_version_for_the_footer() {
        let state = state_of(&sample_snapshot());
        let key = "\"pitwall_version\":\"";
        let start = state.find(key).expect("pitwall_version key") + key.len();
        let end = start + state[start..].find('"').expect("closing quote");
        let version = &state[start..end];
        assert!(!version.is_empty(), "empty pitwall_version: {state}");
        // One version source (Cargo.toml), so the footer cannot drift.
        assert_eq!(version, crate::version());
        assert_valid_json(&state);
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

    // -----------------------------------------------------------------
    // Property test (M8 task 11.3). `proptest` is a dev-dependency pinned
    // `=1.5.0`; Property 23 is exactly ONE test at 100+ cases. The example
    // tests above pin the documented chat object byte-for-byte; this
    // generalises over generated workspaces.
    //
    // Two honest limits, stated rather than worked around:
    //
    // 1. **"non-chat keys are exactly the v3 keys"** cannot be checked by
    //    diffing against a real v3 writer, because there is no v3 writer any
    //    more — the version was bumped in place. So the v3 key sets are
    //    spelled out below as data. That is deliberate: adding a key to the
    //    artifact now requires editing this table, which is exactly the
    //    review moment Requirement 22.1 (additive change only) needs.
    //
    // 2. **"the reader accepts documents of version 1, 2, 3 and 4"** cannot
    //    be asserted from Rust at all. No Rust code reads `state.json`; the
    //    reader is `plugin/dev.pitwall/StateReader.qml`, whose version gate
    //    (`version === 1 || … || version === 4`) is covered by the QML test
    //    file and the mandatory live gate (design Testing Strategy). What
    //    this test can honestly assert is the writer half: the emitted
    //    `state_version` is 4 and the document is well-formed. It does not
    //    pretend to cover the reader.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    /// Top-level keys of the v3 artifact. v4 adds `pitwall_version`
    /// (Requirement 7.7) and nothing else at this level.
    const V3_TOP_LEVEL_KEYS: &[&str] = &[
        "state_version",
        "collected_at",
        "hostname",
        "session_count",
        "config",
        "sessions",
        "resumable",
        "summary",
        "notifications",
        "unread_count",
    ];

    /// Per-session keys of the v3 artifact. v4 adds `chat` and nothing else.
    const V3_SESSION_KEYS: &[&str] = &[
        "id",
        "state",
        "process_count",
        "project",
        "agent",
        "window",
        "last_activity",
        "summary",
        "role",
        "age_secs",
        "history",
        "root_pid",
        "tier",
        "group",
    ];

    const V3_PROJECT_KEYS: &[&str] = &[
        "id",
        "dir",
        "name",
        "is_git_repo",
        "branch",
        "git_clean",
    ];
    const V3_AGENT_KEYS: &[&str] = &["kind", "confidence"];
    const V3_WINDOW_KEYS: &[&str] = &["address", "class", "title", "workspace"];
    const V3_LAST_ACTIVITY_KEYS: &[&str] = &["epoch", "kind"];
    const V3_CONFIG_KEYS: &[&str] = &["agent", "model", "summary_enabled"];
    const V3_RESUMABLE_KEYS: &[&str] = &[
        "checkpoint_id",
        "session_id",
        "project_id",
        "project_dir",
        "project_name",
        "branch",
        "git_clean",
        "agent_kind",
        "agent_confidence",
        "state",
        "last_activity_epoch",
        "created_at",
        "note",
        "trigger",
        "tier",
        "group",
    ];
    const V3_SUMMARY_KEYS: &[&str] =
        &["text", "model", "created_at", "input_hash", "status", "message"];
    const V3_NOTIFICATION_KEYS: &[&str] = &[
        "id",
        "kind",
        "session_id",
        "project",
        "agent",
        "state",
        "severity",
        "detail",
        "created_at",
    ];

    /// The chat object's keys (design §5.3).
    const V4_CHAT_KEYS: &[&str] = &[
        "number",
        "harness",
        "model",
        "context_label",
        "context_session_id",
        "started_at",
    ];

    // --- a minimal JSON extractor, for the "parsing that document" half ---
    //
    // `JsonParser` above answers "is this valid JSON?"; the round trip needs
    // the *values* back. This is the smallest thing that does that: an
    // ordered value tree with string unescaping, so a label carrying a quote
    // or a backslash has to survive the writer's escaping to compare equal.

    #[derive(Debug, Clone, PartialEq)]
    enum Json {
        Null,
        Bool(bool),
        /// Kept as text; compared through [`Json::as_i64`].
        Num(String),
        Str(String),
        Arr(Vec<Json>),
        /// Ordered entries, so duplicate keys would be visible rather than
        /// silently collapsed.
        Obj(Vec<(String, Json)>),
    }

    impl Json {
        fn parse(s: &str) -> Option<Json> {
            let mut r = JsonReader {
                b: s.as_bytes(),
                pos: 0,
            };
            let v = r.value()?;
            r.ws();
            if r.pos == r.b.len() {
                Some(v)
            } else {
                None
            }
        }
        fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        fn keys(&self) -> Vec<String> {
            match self {
                Json::Obj(entries) => entries.iter().map(|(k, _)| k.clone()).collect(),
                _ => Vec::new(),
            }
        }
        fn as_str(&self) -> Option<&str> {
            match self {
                Json::Str(s) => Some(s.as_str()),
                _ => None,
            }
        }
        fn as_i64(&self) -> Option<i64> {
            match self {
                Json::Num(n) => n.parse().ok(),
                _ => None,
            }
        }
        fn as_arr(&self) -> Option<&[Json]> {
            match self {
                Json::Arr(items) => Some(items.as_slice()),
                _ => None,
            }
        }
    }

    struct JsonReader<'a> {
        b: &'a [u8],
        pos: usize,
    }

    impl JsonReader<'_> {
        fn ws(&mut self) {
            while matches!(self.b.get(self.pos), Some(c) if c.is_ascii_whitespace()) {
                self.pos += 1;
            }
        }
        fn peek(&self) -> Option<u8> {
            self.b.get(self.pos).copied()
        }
        fn eat(&mut self, c: u8) -> bool {
            if self.peek() == Some(c) {
                self.pos += 1;
                true
            } else {
                false
            }
        }
        fn lit(&mut self, s: &str) -> bool {
            if self.b[self.pos..].starts_with(s.as_bytes()) {
                self.pos += s.len();
                true
            } else {
                false
            }
        }
        /// Unescape exactly the escapes [`escape`] can emit, plus the rest of
        /// the JSON set so a hand-written fixture would parse too.
        fn string(&mut self) -> Option<String> {
            if !self.eat(b'"') {
                return None;
            }
            let mut out: Vec<u8> = Vec::new();
            loop {
                match self.peek()? {
                    b'"' => {
                        self.pos += 1;
                        return String::from_utf8(out).ok();
                    }
                    b'\\' => {
                        self.pos += 1;
                        let esc = self.peek()?;
                        self.pos += 1;
                        match esc {
                            b'"' => out.push(b'"'),
                            b'\\' => out.push(b'\\'),
                            b'/' => out.push(b'/'),
                            b'n' => out.push(b'\n'),
                            b'r' => out.push(b'\r'),
                            b't' => out.push(b'\t'),
                            b'b' => out.push(0x08),
                            b'f' => out.push(0x0c),
                            b'u' => {
                                let hex =
                                    std::str::from_utf8(self.b.get(self.pos..self.pos + 4)?).ok()?;
                                self.pos += 4;
                                let cp = u32::from_str_radix(hex, 16).ok()?;
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(
                                    char::from_u32(cp)?.encode_utf8(&mut buf).as_bytes(),
                                );
                            }
                            _ => return None,
                        }
                    }
                    byte => {
                        out.push(byte);
                        self.pos += 1;
                    }
                }
            }
        }
        fn number(&mut self) -> Option<String> {
            let start = self.pos;
            self.eat(b'-');
            let digits = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
            if self.pos == digits {
                self.pos = start;
                return None;
            }
            if self.eat(b'.') {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.pos += 1;
                }
            }
            std::str::from_utf8(&self.b[start..self.pos])
                .ok()
                .map(str::to_string)
        }
        fn value(&mut self) -> Option<Json> {
            self.ws();
            match self.peek()? {
                b'"' => self.string().map(Json::Str),
                b'{' => {
                    self.pos += 1;
                    let mut entries: Vec<(String, Json)> = Vec::new();
                    self.ws();
                    if self.eat(b'}') {
                        return Some(Json::Obj(entries));
                    }
                    loop {
                        self.ws();
                        let key = self.string()?;
                        self.ws();
                        if !self.eat(b':') {
                            return None;
                        }
                        let value = self.value()?;
                        entries.push((key, value));
                        self.ws();
                        if self.eat(b',') {
                            continue;
                        }
                        if self.eat(b'}') {
                            return Some(Json::Obj(entries));
                        }
                        return None;
                    }
                }
                b'[' => {
                    self.pos += 1;
                    let mut items: Vec<Json> = Vec::new();
                    self.ws();
                    if self.eat(b']') {
                        return Some(Json::Arr(items));
                    }
                    loop {
                        items.push(self.value()?);
                        self.ws();
                        if self.eat(b',') {
                            continue;
                        }
                        if self.eat(b']') {
                            return Some(Json::Arr(items));
                        }
                        return None;
                    }
                }
                b't' => self.lit("true").then_some(Json::Bool(true)),
                b'f' => self.lit("false").then_some(Json::Bool(false)),
                b'n' => self.lit("null").then_some(Json::Null),
                _ => self.number().map(Json::Num),
            }
        }
    }

    #[test]
    fn json_extractor_round_trips_escapes_and_shapes() {
        // The extractor is test infrastructure, so it gets its own proof.
        let doc = Json::parse("{\"a\":[1,-2,null,true],\"b\":{\"c\":\"q\\\"\\\\\\u0007\"}}")
            .expect("parses");
        assert_eq!(doc.keys(), vec!["a".to_string(), "b".to_string()]);
        assert_eq!(doc.get("a").and_then(Json::as_arr).map(|a| a.len()), Some(4));
        let items = doc.get("a").and_then(Json::as_arr).expect("array");
        assert_eq!(items[1].as_i64(), Some(-2));
        assert!(matches!(items[2], Json::Null));
        assert!(matches!(items[3], Json::Bool(true)));
        assert_eq!(
            doc.get("b").and_then(|b| b.get("c")).and_then(Json::as_str),
            Some("q\"\\\u{7}")
        );
        // Whatever `escape` emits, the extractor reads back verbatim.
        for raw in ["plain", "q\"uote", "back\\slash", "tab\there", "nl\nhere", "café"] {
            let json = format!("{{\"v\":\"{}\"}}", escape(raw));
            assert_eq!(
                Json::parse(&json).and_then(|d| d.get("v").and_then(Json::as_str).map(str::to_string)),
                Some(raw.to_string()),
                "{json}"
            );
        }
        assert!(Json::parse("{\"a\":1\"b\":2}").is_none());
        assert!(Json::parse("").is_none());
    }

    // --- generators --------------------------------------------------------

    const CHAT_HARNESSES: &[&str] = &["opencode", "claude", "codex"];
    /// Empty means the harness default; the others are
    /// [`crate::summary::valid_model`] ids.
    const CHAT_MODELS: &[&str] = &["", "prov/model", "openrouter/anth-3.5"];
    /// Labels a real chat could carry — including two that force the writer's
    /// escaping to be correct on the way out and the extractor's unescaping on
    /// the way back in.
    const CHAT_LABELS: &[&str] = &[
        "Work",
        "Workspace",
        "my project",
        "He said \"hi\"",
        "back\\slash",
        "café",
    ];
    const HOSTNAMES: &[&str] = &["testbox", "test\"box", "box\\1"];

    fn chat_facts_strategy() -> impl Strategy<Value = ChatFacts> {
        (
            1u16..=999u16,
            0usize..CHAT_HARNESSES.len(),
            0usize..CHAT_MODELS.len(),
            0usize..CHAT_LABELS.len(),
            proptest::option::of(
                proptest::string::string_regex("sess_[0-9a-f]{16}").expect("static regex"),
            ),
            0i64..2_000_000_000i64,
        )
            .prop_map(
                |(number, harness_ix, model_ix, label_ix, context_session_id, started_at_epoch)| {
                    ChatFacts {
                        number,
                        harness: CHAT_HARNESSES[harness_ix].to_string(),
                        model: CHAT_MODELS[model_ix].to_string(),
                        context_label: CHAT_LABELS[label_ix].to_string(),
                        context_session_id,
                        started_at_epoch,
                    }
                },
            )
    }

    /// One observed session, built from [`sample_snapshot`] so every
    /// pre-existing field keeps a realistic value. `chat: Some(..)` mirrors
    /// what the collector produces: `role == Chat` exactly when facts exist.
    fn session_of(
        id: &str,
        chat: Option<ChatFacts>,
        with_window: bool,
        with_project: bool,
    ) -> TerminalSession {
        let mut snap = sample_snapshot();
        let mut sess = snap.sessions.remove(0);
        sess.id = id.to_string();
        if !with_window {
            sess.window = None;
        }
        if !with_project {
            sess.project = None;
        }
        match &chat {
            Some(facts) => {
                sess.role = WindowRole::Chat;
                let model_label = if facts.model.is_empty() {
                    "agent default"
                } else {
                    facts.model.as_str()
                };
                if let Some(w) = sess.window.as_mut() {
                    w.title = format!(
                        "Pitwall Chat {} · {} · {} · {}",
                        facts.number_text(),
                        facts.harness,
                        model_label,
                        facts.context_label
                    );
                }
                sess.summary = format!(
                    "Pitwall Chat {} · {} · {} · sleeping",
                    facts.number_text(),
                    facts.harness,
                    facts.context_label
                );
            }
            None => sess.role = WindowRole::Terminal,
        }
        sess.chat = chat;
        sess
    }

    /// A sorted key set, so assertions do not depend on emission order.
    fn sorted_set(keys: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = keys.iter().map(|k| (*k).to_string()).collect();
        out.sort();
        out
    }

    fn sorted_keys(value: &Json) -> Vec<String> {
        let mut out = value.keys();
        out.sort();
        out
    }

    proptest! {
        // 192 cases, well above the 100 floor: 1..3 chat sessions × 0..2
        // plain sessions × window present/absent × project present/absent ×
        // summary present/absent × 0..2 checkpoints × 0..2 notifications ×
        // meta present/absent, over generated chat facts.
        #![proptest_config(ProptestConfig { cases: 192, ..ProptestConfig::default() })]

        // **Validates: Requirements 17.6, 22.1, 22.2, 22.3, 22.4, 22.8**
        // Feature: pitwall-chat-and-brief-ticker, Property 23: state.json chat fields round-trip and stay additive — For any observed workspace containing Chat_Sessions, the writer emits a well-formed JSON document whose non-chat keys are exactly the v3 keys, and parsing that document yields Chat_Session field values equal to the observed ones (number, harness, model, context label, context session id, start epoch, tier, group); and the reader accepts documents of version 1, 2, 3 and 4.
        #[test]
        fn prop23_state_json_chat_fields_round_trip_and_stay_additive(
            chats in proptest::collection::vec(chat_facts_strategy(), 1..4),
            plain_count in 0usize..3,
            hostname_ix in 0usize..HOSTNAMES.len(),
            with_window in any::<bool>(),
            with_project in any::<bool>(),
            checkpoint_count in 0usize..3,
            with_summary in any::<bool>(),
            notification_count in 0usize..3,
            with_meta in any::<bool>(),
        ) {
            // ---- the observed workspace ---------------------------------
            let mut snap = sample_snapshot();
            snap.hostname = HOSTNAMES[hostname_ix].to_string();
            snap.sessions.clear();
            for (ix, facts) in chats.iter().enumerate() {
                snap.sessions.push(session_of(
                    &format!("sess_chat{ix}"),
                    Some(facts.clone()),
                    with_window,
                    with_project,
                ));
            }
            for ix in 0..plain_count {
                snap.sessions.push(session_of(
                    &format!("sess_plain{ix}"),
                    None,
                    with_window,
                    with_project,
                ));
            }
            let checkpoints: Vec<Checkpoint> = (0..checkpoint_count)
                .map(|i| sample_checkpoint(i as i64 + 1, &format!("sess_gone{i}")))
                .collect();
            let notifications: Vec<crate::store::Notification> = (0..notification_count)
                .map(|i| sample_notification(i as i64 + 1, "stopped", "attention"))
                .collect();
            let summary = StateSummary::ready(
                "All quiet.".to_string(),
                Some("prov/model".to_string()),
                1_700_000_300,
                "fnv:abc123".to_string(),
            );
            let mut meta: HashMap<String, SessionMeta> = HashMap::new();
            if with_meta {
                for sess in &snap.sessions {
                    meta.insert(
                        sess.id.clone(),
                        SessionMeta {
                            age_secs: Some(60),
                            history: "RRS".to_string(),
                        },
                    );
                }
            }

            let state = snapshot_to_state_json(
                &snap,
                &checkpoints,
                if with_summary { Some(&summary) } else { None },
                &meta,
                &ConfigEcho::default(),
                &notifications,
                notification_count as i64,
            );

            // ---- (1) a well-formed JSON document ------------------------
            // The strict validator the example tests use: comma discipline,
            // no trailing commas, complete consumption (22.2).
            assert_valid_json(&state);
            let doc = Json::parse(&state).expect("emitted state.json parses");

            // ---- (2) non-chat keys are exactly the v3 keys --------------
            // v4's only non-chat addition at the top level is
            // `pitwall_version`; naming it here is what makes any *further*
            // addition a deliberate edit rather than a silent one (22.1).
            let mut want_top: Vec<&str> = V3_TOP_LEVEL_KEYS.to_vec();
            want_top.push("pitwall_version");
            prop_assert_eq!(sorted_keys(&doc), sorted_set(&want_top));
            prop_assert_eq!(
                doc.get("config").map(sorted_keys),
                Some(sorted_set(V3_CONFIG_KEYS))
            );

            // ---- (3) the writer half of the version contract ------------
            // The reader half (accepting 1, 2, 3 and 4) lives in
            // StateReader.qml; see the module note above.
            prop_assert_eq!(
                doc.get("state_version").and_then(Json::as_i64),
                Some(i64::from(STATE_SCHEMA_VERSION))
            );
            prop_assert_eq!(doc.get("state_version").and_then(Json::as_i64), Some(4));
            prop_assert_eq!(
                doc.get("pitwall_version").and_then(Json::as_str),
                Some(crate::version())
            );
            prop_assert_eq!(
                doc.get("hostname").and_then(Json::as_str),
                Some(snap.hostname.as_str())
            );
            prop_assert_eq!(
                doc.get("session_count").and_then(Json::as_i64),
                Some(snap.sessions.len() as i64)
            );

            // ---- (4) every session: v3 keys plus `chat` -----------------
            let sessions = doc
                .get("sessions")
                .and_then(Json::as_arr)
                .expect("sessions array");
            prop_assert_eq!(sessions.len(), snap.sessions.len());
            let mut want_session: Vec<&str> = V3_SESSION_KEYS.to_vec();
            want_session.push("chat");
            let want_session = sorted_set(&want_session);

            let mut emitted_chat_objects = 0usize;
            for (observed, emitted) in snap.sessions.iter().zip(sessions.iter()) {
                prop_assert_eq!(sorted_keys(emitted), want_session.clone());
                prop_assert_eq!(
                    emitted.get("id").and_then(Json::as_str),
                    Some(observed.id.as_str())
                );
                // Pre-existing nested objects keep their v3 shape exactly;
                // `null` stays available for the unobserved cases.
                for (key, want) in [
                    ("project", V3_PROJECT_KEYS),
                    ("agent", V3_AGENT_KEYS),
                    ("window", V3_WINDOW_KEYS),
                    ("last_activity", V3_LAST_ACTIVITY_KEYS),
                ] {
                    match emitted.get(key) {
                        Some(Json::Null) => {}
                        Some(value) => prop_assert_eq!(sorted_keys(value), sorted_set(want)),
                        None => prop_assert!(false, "missing session key {}", key),
                    }
                }

                match &observed.chat {
                    Some(facts) => {
                        emitted_chat_objects += 1;
                        let chat = emitted.get("chat").expect("chat key");
                        prop_assert_eq!(sorted_keys(chat), sorted_set(V4_CHAT_KEYS));

                        // ---- (5) the round trip, field by field ---------
                        // `number` comes back as the three-digit STRING form,
                        // never an integer (the panel's `chatOf` gate matches
                        // `^[0-9]{3}$`).
                        let number_text = facts.number_text();
                        prop_assert_eq!(
                            chat.get("number").and_then(Json::as_str),
                            Some(number_text.as_str())
                        );
                        prop_assert!(chat.get("number").and_then(Json::as_i64).is_none());
                        prop_assert_eq!(
                            chat.get("harness").and_then(Json::as_str),
                            Some(facts.harness.as_str())
                        );
                        prop_assert_eq!(
                            chat.get("model").and_then(Json::as_str),
                            Some(facts.model.as_str())
                        );
                        prop_assert_eq!(
                            chat.get("context_label").and_then(Json::as_str),
                            Some(facts.context_label.as_str())
                        );
                        match facts.context_session_id.as_deref() {
                            Some(id) => prop_assert_eq!(
                                chat.get("context_session_id").and_then(Json::as_str),
                                Some(id)
                            ),
                            // `null`, never an empty string and never invented.
                            None => prop_assert!(matches!(
                                chat.get("context_session_id"),
                                Some(Json::Null)
                            )),
                        }
                        prop_assert_eq!(
                            chat.get("started_at").and_then(Json::as_i64),
                            Some(facts.started_at_epoch)
                        );

                        // ---- (6) tier and group for a chat session -------
                        prop_assert_eq!(emitted.get("role").and_then(Json::as_str), Some("chat"));
                        let want_tier = tier_for_session(
                            observed.role,
                            observed.project.as_ref().map(|p| p.name.as_str()),
                            observed.project.as_ref().map(|p| p.dir.as_str()),
                            observed.agent.kind.as_str(),
                        );
                        prop_assert_eq!(want_tier, "pitwall-native");
                        prop_assert_eq!(
                            emitted.get("tier").and_then(Json::as_str),
                            Some(want_tier)
                        );
                        prop_assert_eq!(
                            emitted.get("group").and_then(Json::as_str),
                            Some(CHAT_GROUP_KEY)
                        );
                    }
                    None => {
                        // Additive means invisible when absent: a non-chat
                        // session keeps its v3 tier and project grouping.
                        prop_assert!(matches!(emitted.get("chat"), Some(Json::Null)));
                        prop_assert_ne!(emitted.get("role").and_then(Json::as_str), Some("chat"));
                        let want_tier = tier_for(
                            observed.project.as_ref().map(|p| p.name.as_str()),
                            observed.project.as_ref().map(|p| p.dir.as_str()),
                            observed.agent.kind.as_str(),
                        );
                        prop_assert_eq!(
                            emitted.get("tier").and_then(Json::as_str),
                            Some(want_tier)
                        );
                        let want_group = group_for(
                            observed.project.as_ref().map(|p| p.id.as_str()),
                            &observed.id,
                        );
                        prop_assert_eq!(
                            emitted.get("group").and_then(Json::as_str),
                            Some(want_group.as_str())
                        );
                        prop_assert_ne!(want_group.as_str(), CHAT_GROUP_KEY);
                    }
                }
            }
            // Exactly as many chat objects as observed chat sessions.
            prop_assert_eq!(emitted_chat_objects, chats.len());

            // ---- (7) the rest of the artifact is untouched v3 -----------
            let resumable = doc
                .get("resumable")
                .and_then(Json::as_arr)
                .expect("resumable array");
            prop_assert_eq!(resumable.len(), checkpoint_count);
            for entry in resumable {
                prop_assert_eq!(sorted_keys(entry), sorted_set(V3_RESUMABLE_KEYS));
            }
            match doc.get("summary") {
                Some(Json::Null) => prop_assert!(!with_summary),
                Some(value) => {
                    prop_assert!(with_summary);
                    prop_assert_eq!(sorted_keys(value), sorted_set(V3_SUMMARY_KEYS));
                }
                None => prop_assert!(false, "missing summary key"),
            }
            let emitted_notifications = doc
                .get("notifications")
                .and_then(Json::as_arr)
                .expect("notifications array");
            prop_assert_eq!(emitted_notifications.len(), notification_count);
            for entry in emitted_notifications {
                prop_assert_eq!(sorted_keys(entry), sorted_set(V3_NOTIFICATION_KEYS));
            }
            prop_assert_eq!(
                doc.get("unread_count").and_then(Json::as_i64),
                Some(notification_count as i64)
            );
        }
    }
}
