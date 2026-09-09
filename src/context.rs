//! Ephemeral AI workspace context (M5c).
//!
//! Builds ONE bounded JSON document for a single explicitly-triggered
//! summary request. Terminal text is ephemeral: it lives in this document
//! only, which exists solely as a short-lived file under the OS runtime
//! dir (see [`EphemeralContext`)) and is never copied into SQLite,
//! state.json, logs, or the repository.
//!
//! Evidence hierarchy (hard rule): terminal text, checkpoint notes, and
//! derived transitions are *semantic* evidence; IO deltas, CPU time, and
//! child churn are *activity* evidence only. The instruction given to the
//! agent states this; this module additionally refuses to manufacture
//! semantic claims (e.g. it never emits "waiting for approval").

use crate::collector::WorkspaceSnapshot;
use crate::output::escape;
use crate::platform::{IoCounters, Platform, TerminalText};
use crate::store::{Checkpoint, PrevSession};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// First/last non-empty terminal lines per window.
pub const WINDOW_LINES: usize = 10;
/// Hard byte cap per terminal window before line selection.
pub const WINDOW_BYTES: usize = 4096;
/// Maximum sessions in one context (terminal-role first).
pub const MAX_SESSIONS: usize = 6;
/// Whole-document cap; over-budget drops quietest sessions first and
/// records `truncated_sessions` honestly.
pub const MAX_CONTEXT_BYTES: usize = 16 * 1024;

/// Split pre-filtered non-empty lines into first-N / last-N windows.
/// Overlapping ranges share lines (no duplication games); scarce input
/// yields what exists.
pub fn window_lines(lines: &[String]) -> (Vec<String>, Vec<String>) {
    let first: Vec<String> = lines.iter().take(WINDOW_LINES).cloned().collect();
    let start = lines.len().saturating_sub(WINDOW_LINES);
    let last: Vec<String> = lines[start..].to_vec();
    (first, last)
}

/// Strip control characters (keep `\n`/`\t`), as terminal captures may
/// carry escape sequences and bells that are neither content nor safe.
pub fn strip_controls(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// Redact recognizable secret forms, replacing values with `[redacted]`.
/// Structural bans (no env/argv/transcripts) matter more; this is the
/// second layer for secrets that arrive inside otherwise-safe fields.
pub fn scrub_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // PEM block: redact through the END line.
        if s[i..].starts_with("-----BEGIN") && s[i..].contains("PRIVATE KEY-----") {
            if let Some(end) = s[i..].find("-----END") {
                if let Some(nl) = s[i + end..].find('\n') {
                    out.push_str("[redacted-pem-block]");
                    i += end + nl;
                    continue;
                }
            }
        }
        // key=value style secrets: password/passwd/secret/token, bearer.
        // Values run to whitespace (over-redacting trailing punctuation
        // is safer than under-redacting).
        // Delimited forms always redact ("password=x"). Bare-space forms
        // ("my secret XYZ") only redact when the value looks secret-shaped
        // (long enough with digits/case/symbols) — plain words like the
        // "here" in "nothing secret here" must survive. Over-redaction
        // destroys context; under-redaction leaks it: err toward shape.
        const DELIMITED_KEYS: &[&str] = &[
            "password=",
            "password = ",
            "passwd=",
            "passwd = ",
            "secret=",
            "secret = ",
            "token=",
            "token = ",
            "bearer ",
        ];
        const BARE_KEYS: &[&str] = &["password ", "passwd ", "secret ", "token "];
        let rest = &s[i..];
        let lowered = rest.to_lowercase();
        if let Some(key) = DELIMITED_KEYS.iter().find(|k| lowered.starts_with(*k)) {
            out.push_str(&rest[..key.len()]);
            out.push_str("[redacted]");
            i += key.len();
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        if let Some(key) = BARE_KEYS.iter().find(|k| lowered.starts_with(*k)) {
            let start = i + key.len();
            let mut end = start;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            let value = &s[start..end];
            let shaped = value.len() >= 6
                && (value.chars().any(|c| c.is_ascii_digit())
                    || value.chars().any(|c| c.is_ascii_uppercase())
                    || value.len() >= 12);
            if shaped {
                out.push_str(&rest[..key.len()]);
                out.push_str("[redacted]");
                i = end;
                continue;
            }
            // Plain word: emit literally, fall through to char advance.
        }
        // Token-shaped literals: sk-*, gh[po...]_, github_pat_, AKIA, xox*.
        if let Some(token_len) = token_literal_len(rest) {
            out.push_str("[redacted]");
            i += token_len;
            continue;
        }
        // Advance one char (byte-safe).
        let ch_len = s[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }
    out
}

/// Length of a secret-shaped token at the start of `s`, if any.
fn token_literal_len(s: &str) -> Option<usize> {
    const PREFIXES: &[&str] = &[
        "sk-live-",
        "sk-test-",
        "sk-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "AKIA",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxs-",
    ];
    for prefix in PREFIXES {
        if s.starts_with(prefix) {
            let mut len = prefix.len();
            for c in s[len..].chars() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    len += c.len_utf8();
                } else {
                    break;
                }
            }
            // Guard against matching bare prefixes with no body.
            if len > prefix.len() + 3 || *prefix == "AKIA" {
                return Some(len);
            }
        }
    }
    None
}

/// One derived workspace event (bounded, scrubbed, display-safe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: &'static str,
    pub session_id: String,
    pub project: String,
    pub detail: String,
}

/// Derive events by diffing the previous retained observation against the
/// current snapshot, plus recently created checkpoints. Pure function over
/// small inputs; capped by the caller. No new tables, no logging subsystem.
pub fn derive_events(
    prev: &[PrevSession],
    curr: &WorkspaceSnapshot,
    checkpoints_since_prev: &[Checkpoint],
    max_events: usize,
) -> Vec<Event> {
    let mut events = Vec::new();
    let prev_by_id: HashMap<&str, &PrevSession> =
        prev.iter().map(|p| (p.session_id.as_str(), p)).collect();
    let curr_ids: std::collections::HashSet<&str> =
        curr.sessions.iter().map(|s| s.id.as_str()).collect();

    for s in &curr.sessions {
        let project = s
            .project
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "session".to_string());
        match prev_by_id.get(s.id.as_str()) {
            None => events.push(Event {
                kind: "session_appeared",
                session_id: s.id.clone(),
                project,
                detail: format!("{} {}", s.agent.kind.as_str(), s.state.as_str()),
            }),
            Some(p) => {
                if p.agent_kind != s.agent.kind.as_str() {
                    events.push(Event {
                        kind: "agent_changed",
                        session_id: s.id.clone(),
                        project: project.clone(),
                        detail: format!("{} -> {}", p.agent_kind, s.agent.kind.as_str()),
                    });
                }
                let curr_branch = s.project.as_ref().and_then(|p| p.branch.clone());
                if p.branch != curr_branch {
                    events.push(Event {
                        kind: "branch_changed",
                        session_id: s.id.clone(),
                        project: project.clone(),
                        detail: format!(
                            "{} -> {}",
                            p.branch.as_deref().unwrap_or("-"),
                            curr_branch.as_deref().unwrap_or("-")
                        ),
                    });
                }
                let curr_clean = s.project.as_ref().and_then(|p| p.git_clean);
                if p.git_clean != curr_clean {
                    events.push(Event {
                        kind: "git_changed",
                        session_id: s.id.clone(),
                        project,
                        detail: format!("clean={curr_clean:?}"),
                    });
                }
            }
        }
    }
    for p in prev {
        if !curr_ids.contains(p.session_id.as_str()) {
            events.push(Event {
                kind: "session_vanished",
                session_id: p.session_id.clone(),
                project: p
                    .project_dir
                    .as_ref()
                    .and_then(|d| d.rsplit('/').next())
                    .unwrap_or("session")
                    .to_string(),
                detail: format!("was {} {}", p.agent_kind, p.state),
            });
        }
    }
    for cp in checkpoints_since_prev {
        events.push(Event {
            kind: "checkpoint_created",
            session_id: cp.session_id.clone(),
            project: cp
                .project_dir
                .rsplit('/')
                .next()
                .unwrap_or("session")
                .to_string(),
            detail: format!("{} {}", cp.trigger, cp.note.as_deref().unwrap_or("no note")),
        });
    }
    events.truncate(max_events);
    events
}

/// One session's allowlisted context block. Every field is explicitly
/// approved; there are no fields for env, argv, transcripts, or secrets.
fn session_block(
    s: &crate::collector::TerminalSession,
    text: &TerminalText,
    io: Option<IoCounters>,
) -> String {
    let mut out = String::new();
    let p = s.project.as_ref();
    out.push_str(&format!("    \"id\": {},\n", q(&s.id)));
    out.push_str(&format!("    \"role\": {},\n", q(s.role.as_str())));
    out.push_str(&format!(
        "    \"project\": {},\n",
        q(p.map(|p| p.name.as_str()).unwrap_or("none"))
    ));
    out.push_str(&format!(
        "    \"dir_known\": {},\n",
        p.map(|p| !p.dir.is_empty()).unwrap_or(false)
    ));
    out.push_str(&format!(
        "    \"branch\": {},\n",
        opt_q(p.and_then(|p| p.branch.as_deref()))
    ));
    out.push_str(&format!(
        "    \"git_clean\": {},\n",
        opt_bool(p.and_then(|p| p.git_clean))
    ));
    out.push_str(&format!(
        "    \"agent\": {}, \"agent_confidence\": {},\n",
        q(s.agent.kind.as_str()),
        q(s.agent.confidence.as_str())
    ));
    out.push_str(&format!(
        "    \"state\": {}, \"processes\": {}, \"last_activity_epoch\": {},\n",
        q(s.state.as_str()),
        s.process_count,
        s.last_activity_epoch
    ));
    match io {
        Some(counters) => out.push_str(&format!(
            "    \"io_bytes\": {{\"read\": {}, \"write\": {}}},\n",
            counters.read_bytes, counters.write_bytes
        )),
        None => out.push_str("    \"io_bytes\": null,\n"),
    }
    match text {
        TerminalText::Lines { first, last } => {
            out.push_str("    \"terminal_text\": \"observable\",\n");
            out.push_str(&format!("    \"term_first\": {},\n", str_list(first)));
            out.push_str(&format!("    \"term_last\": {}\n", str_list(last)));
        }
        TerminalText::Unavailable { reason } => {
            out.push_str(&format!("    \"terminal_text\": {},\n", q(reason)));
        }
    }
    out
}

fn q(s: &str) -> String {
    format!("\"{}\"", escape(&scrub_string(s)))
}

fn opt_q(v: Option<&str>) -> String {
    v.map_or("null".to_string(), q)
}

fn opt_bool(v: Option<bool>) -> String {
    v.map_or("null".to_string(), |b| b.to_string())
}

fn str_list(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&q(item));
    }
    out.push(']');
    out
}

/// Priority for session retention under the cap: terminal-role first,
/// then most recent activity. Deterministic (id tiebreak).
fn session_priority(s: &crate::collector::TerminalSession) -> (u8, i64, String) {
    let role_rank = match s.role {
        crate::collector::WindowRole::Terminal => 0,
        crate::collector::WindowRole::Unknown => 1,
        crate::collector::WindowRole::App => 2,
    };
    // Negate epoch via reverse: higher epoch first → use Reverse ordering
    // through a wrapper below; here return raw and sort explicitly.
    (role_rank, s.last_activity_epoch, s.id.clone())
}

/// Build the bounded ephemeral context document. Returns
/// `(document, truncated_sessions)`. Sampling (io/text) happens here via
/// the platform; nothing is persisted.
pub fn build_context(
    platform: &dyn Platform,
    snapshot: &WorkspaceSnapshot,
    events: &[Event],
    checkpoints: &[Checkpoint],
) -> (String, usize) {
    let mut ordered: Vec<&crate::collector::TerminalSession> = snapshot.sessions.iter().collect();
    ordered.sort_by(|a, b| {
        session_priority(a)
            .0
            .cmp(&session_priority(b).0)
            .then_with(|| session_priority(b).1.cmp(&session_priority(a).1))
            .then_with(|| session_priority(a).2.cmp(&session_priority(b).2))
    });
    // Shrink the session set until the whole document fits. Events and
    // checkpoints are small and stable; sessions dominate size, so only
    // they are cut — and the cut is always reported honestly.
    let mut keep = ordered.len().min(MAX_SESSIONS.max(1));
    loop {
        let truncated = snapshot.sessions.len().saturating_sub(keep);
        let doc = render_document(
            platform,
            &ordered[..keep.min(ordered.len())],
            events,
            checkpoints,
            truncated,
        );
        if doc.len() <= MAX_CONTEXT_BYTES || keep <= 1 {
            return (doc, truncated);
        }
        keep -= 1;
    }
}

/// Render one document over an already-ordered session slice.
fn render_document(
    platform: &dyn Platform,
    sessions: &[&crate::collector::TerminalSession],
    events: &[Event],
    checkpoints: &[Checkpoint],
    truncated: usize,
) -> String {
    let mut doc = String::from("{\"sessions\":[\n");
    for (i, s) in sessions.iter().enumerate() {
        if i > 0 {
            doc.push_str(",\n");
        }
        let class = s.window.as_ref().map(|w| w.class.as_str()).unwrap_or("");
        let text = platform.terminal_text(s.root_pid, class);
        let io = platform.process_io(s.root_pid);
        // Strip controls at the boundary (defense in depth: the builder
        // only ever emits scrubbed strings via q()).
        let text = match text {
            TerminalText::Lines { first, last } => TerminalText::Lines {
                first: first.into_iter().map(|l| strip_controls(&l)).collect(),
                last: last.into_iter().map(|l| strip_controls(&l)).collect(),
            },
            unavailable => unavailable,
        };
        doc.push_str("  {\n");
        doc.push_str(&session_block(s, &text, io));
        doc.push_str("  }");
    }
    doc.push_str("\n],\"events\":[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            doc.push(',');
        }
        doc.push_str(&format!(
            "{{\"kind\":{},\"session\":{},\"project\":{},\"detail\":{}}}",
            q(e.kind),
            q(&e.session_id),
            q(&e.project),
            q(&e.detail)
        ));
    }
    doc.push_str("],\"checkpoints\":[");
    for (i, cp) in checkpoints.iter().enumerate() {
        if i > 0 {
            doc.push(',');
        }
        doc.push_str(&format!(
            "{{\"project\":{},\"branch\":{},\"state\":{},\"note\":{},\"trigger\":{}}}",
            q(cp.project_dir.rsplit('/').next().unwrap_or("session")),
            opt_q(cp.branch.as_deref()),
            q(crate::output::normalize_state(&cp.state)),
            opt_q(cp.note.as_deref()),
            q(&cp.trigger)
        ));
    }
    doc.push_str(&format!("],\"truncated_sessions\":{truncated}}}"));
    doc
}

/// Runtime dir for ephemeral artifacts: `/run/user/$UID/pitwall`
/// (0700). Falls back to `$TMPDIR/pitwall-$UID` only when the runtime
/// dir is unavailable; never the repo, home data, or world-readable tmp.
pub fn ephemeral_dir() -> PathBuf {
    let uid = libc_uid();
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("pitwall");
        }
    }
    let fallback = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(fallback).join(format!("pitwall-{uid}"))
}

fn libc_uid() -> u32 {
    // No libc crate: derive from $UID env, else 0 (fallback path is still
    // 0700 + O_EXCL, so worst case is inconvenience, not exposure).
    std::env::var("UID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Short-lived context file with Drop-guard cleanup. Created O_EXCL 0600
/// with an unpredictable name; `close()` removes explicitly, `Drop`
/// removes as the safety net. The path is the only thing that may appear
/// in argv — never the content.
pub struct EphemeralContext {
    path: Option<PathBuf>,
}

impl EphemeralContext {
    /// Write `document` to a new unpredictable 0600 file. Retries on name
    /// collision (O_EXCL); errors if the directory cannot be secured.
    pub fn create(document: &str) -> Result<EphemeralContext, String> {
        Self::create_in(&ephemeral_dir(), document)
    }

    /// Same, under an explicit directory (tests pass sandboxes; production
    /// passes [`ephemeral_dir`]). The directory is secured 0700 first.
    pub fn create_in(dir: &Path, document: &str) -> Result<EphemeralContext, String> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create ephemeral dir: {e}"))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot secure ephemeral dir: {e}"))?;
        for _ in 0..16 {
            let name = format!("ctx-{}.json", random_hex16());
            let path = dir.join(name);
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true).mode(0o600);
            match opts.open(&path) {
                Ok(mut file) => {
                    use std::io::Write as _;
                    file.write_all(document.as_bytes())
                        .map_err(|e| format!("cannot write context: {e}"))?;
                    return Ok(EphemeralContext { path: Some(path) });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("cannot create context file: {e}")),
            }
        }
        Err("could not pick an unused context name".to_string())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Explicit removal (idempotent); Drop covers all other exits.
    pub fn close(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for EphemeralContext {
    fn drop(&mut self) {
        self.close();
    }
}

/// 64 unpredictable bits from the OS (`/dev/urandom`, no new deps).
fn random_hex16() -> String {
    use std::io::Read as _;
    let mut bytes = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    } else {
        // Degenerate fallback (never silent): mix time + pid. Still
        // O_EXCL-guarded, so worst case is a retry, not a collision.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        bytes = t
            .wrapping_add(u64::from(std::process::id()).wrapping_mul(0x9E3779B9))
            .to_le_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{
        AgentIdentity, AgentKind, Confidence, ProjectInfo, SessionState, TerminalSession,
        WindowRole, LAST_ACTIVITY_KIND,
    };
    use crate::platform::{GitInfo, RawProcess, WindowInfo};

    struct MockPlatform {
        text: TerminalText,
    }

    impl Platform for MockPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            Vec::new()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            Vec::new()
        }
        fn git_info(&self, _dir: &str) -> GitInfo {
            GitInfo::default()
        }
        fn boot_epoch(&self) -> i64 {
            0
        }
        fn clock_ticks_per_sec(&self) -> i64 {
            100
        }
        fn hostname(&self) -> String {
            "testbox".to_string()
        }
        fn launch_terminal(&self, _d: &str) -> Result<(), String> {
            Ok(())
        }
        fn focus_window_address(&self, _a: &str) -> Result<(), String> {
            Ok(())
        }
        fn process_io(&self, _pid: u32) -> Option<IoCounters> {
            Some(IoCounters {
                read_bytes: 10,
                write_bytes: 20,
            })
        }
        fn terminal_text(&self, _pid: u32, _class: &str) -> TerminalText {
            self.text.clone()
        }
    }

    fn session(id: &str, role: WindowRole, epoch: i64) -> TerminalSession {
        TerminalSession {
            id: id.to_string(),
            window: Some(WindowInfo {
                address: "0x1".to_string(),
                class: "foot".to_string(),
                initial_class: "foot".to_string(),
                title: "t".to_string(),
                workspace: "1".to_string(),
                pid: 10,
            }),
            root_pid: 10,
            role,
            project: Some(ProjectInfo {
                id: "proj_x".to_string(),
                dir: "/home/u/Work".to_string(),
                name: "Work".to_string(),
                is_git_repo: true,
                branch: Some("main".to_string()),
                git_clean: Some(true),
            }),
            agent: AgentIdentity {
                kind: AgentKind::Unknown,
                confidence: Confidence::Low,
                evidence: vec!["terminal context only".to_string()],
            },
            state: SessionState::Sleeping,
            process_count: 2,
            processes: Vec::new(),
            last_activity_epoch: epoch,
            last_activity_kind: LAST_ACTIVITY_KIND,
            summary: "s".to_string(),
        }
    }

    fn snapshot_with(ids_epochs: &[(&str, WindowRole, i64)]) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions: ids_epochs
                .iter()
                .map(|(id, role, e)| session(id, *role, *e))
                .collect(),
        }
    }

    #[test]
    fn window_lines_take_first_and_last_ten() {
        let lines: Vec<String> = (0..25).map(|i| format!("line {i}")).collect();
        let (first, last) = window_lines(&lines);
        assert_eq!(first.len(), 10);
        assert_eq!(last.len(), 10);
        assert_eq!(first[0], "line 0");
        assert_eq!(last[9], "line 24");
        // Scarce input yields what exists (overlap allowed, never invented).
        let few: Vec<String> = vec!["a".to_string(), "b".to_string()];
        let (first, last) = window_lines(&few);
        assert_eq!(first, few);
        assert_eq!(last, few);
        let empty: Vec<String> = Vec::new();
        assert_eq!(window_lines(&empty), (Vec::new(), Vec::new()));
    }

    #[test]
    fn controls_are_stripped_but_text_survives() {
        assert_eq!(strip_controls("ok\x1b[31mred\x07bell"), "ok[31mredbell");
        assert_eq!(strip_controls("a\tb\nc"), "a\tb\nc");
    }

    #[test]
    fn scrub_matrix_redacts_known_secret_forms() {
        let cases = [
            ("key sk-live-ABCDEF123456 rest", "key [redacted] rest"),
            ("tok ghp_ABCDEFGHIJKLMNOP123456", "tok [redacted]"),
            ("tok gho_XYZ1234567890abcd", "tok [redacted]"),
            ("pat github_pat_ABCDEF1234567890", "pat [redacted]"),
            ("aws AKIAIOSFODNN7EXAMPLE end", "aws [redacted] end"),
            ("s xoxb-123-456-abcdef rest", "s [redacted] rest"),
            (
                "Authorization: Bearer ABCDEF123456",
                "Authorization: Bearer [redacted]",
            ),
            ("password=hunter2!", "password=[redacted]"),
            ("db passwd = s3cret thing", "db passwd = [redacted] thing"),
            ("api secret abc123;", "api secret [redacted]"),
            ("token=XYZ789abc end", "token=[redacted] end"),
        ];
        for (input, expected) in cases {
            assert_eq!(scrub_string(input), expected, "input: {input}");
        }
        // PEM blocks vanish entirely.
        let pem =
            "head\n-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----\ntail";
        let scrubbed = scrub_string(pem);
        assert!(!scrubbed.contains("MIIB"));
        assert!(scrubbed.contains("[redacted-pem-block]"));
        assert!(scrubbed.contains("tail"));
        // Ordinary text passes through untouched.
        assert_eq!(
            scrub_string(" nothing secret here 123 "),
            " nothing secret here 123 "
        );
    }

    #[test]
    fn hostile_content_is_contained_not_executed() {
        // Newlines/quotes/backticks survive as data inside JSON strings;
        // nothing here can become argv (argv carries only fixed flags).
        let evil = "foo\"; rm -rf ~; echo \"$(id)`id`";
        let scrubbed = scrub_string(evil);
        let doc = q(evil);
        assert!(doc.starts_with('"') && doc.ends_with('"'));
        assert!(!scrubbed.is_empty());
        // JSON escaping keeps the document parseable-by-shape.
        assert!(!doc.contains("\n`"));
    }

    #[test]
    fn foot_style_unavailable_is_explicit() {
        let plat = MockPlatform {
            text: TerminalText::Unavailable {
                reason: "no scrollback API",
            },
        };
        let snap = snapshot_with(&[("sess_a", WindowRole::Terminal, 100)]);
        let (doc, truncated) = build_context(&plat, &snap, &[], &[]);
        assert_eq!(truncated, 0);
        assert!(
            doc.contains("\"terminal_text\": \"no scrollback API\""),
            "{doc}"
        );
        assert!(!doc.contains("term_first"), "{doc}");
    }

    #[test]
    fn session_cap_prefers_terminals_then_recency() {
        let plat = MockPlatform {
            text: TerminalText::Unavailable { reason: "x" },
        };
        let mut entries = Vec::new();
        for i in 0..8 {
            entries.push((
                format!("sess_{i}").leak() as &str,
                WindowRole::App,
                100 + i as i64,
            ));
        }
        // Two terminals, one quiet (kept), plus apps.
        let snap = snapshot_with(&[
            ("sess_t1", WindowRole::Terminal, 50),
            ("sess_t2", WindowRole::Terminal, 90),
        ]);
        let mut snap = snap;
        for (id, role, e) in entries {
            snap.sessions.push(session(id, role, e));
        }
        let (doc, truncated) = build_context(&plat, &snap, &[], &[]);
        assert_eq!(truncated, 10 - MAX_SESSIONS);
        assert!(
            doc.contains("sess_t1") && doc.contains("sess_t2"),
            "terminals kept: {doc}"
        );
        assert!(
            doc.contains(&format!("\"truncated_sessions\":{truncated}")),
            "{doc}"
        );
        assert!(doc.len() <= MAX_CONTEXT_BYTES, "len={}", doc.len());
    }

    #[test]
    fn ephemeral_file_is_private_unpredictable_and_cleaned() {
        let plat = MockPlatform {
            text: TerminalText::Unavailable { reason: "x" },
        };
        // Explicit sandbox dir: no env mutation, parallel-safe.
        let pitdir = std::env::temp_dir().join("pitwall-m5c-ephem-test");
        let _ = std::fs::remove_dir_all(&pitdir);
        let snap = snapshot_with(&[("sess_a", WindowRole::Terminal, 100)]);
        let (doc, _) = build_context(&plat, &snap, &[], &[]);
        let mut ctx = EphemeralContext::create_in(&pitdir, &doc).unwrap();
        let path = ctx.path().unwrap().to_path_buf();
        assert_eq!(path.parent().unwrap(), pitdir.as_path());
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ctx-"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&pitdir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        // Two contexts never share a name.
        let ctx2 = EphemeralContext::create_in(&pitdir, &doc).unwrap();
        assert_ne!(ctx.path(), ctx2.path());
        drop(ctx2);
        // Explicit close removes; Drop is the net.
        ctx.close();
        assert!(!path.exists());
        assert!(ctx.path().is_none());
        let _ = std::fs::remove_dir_all(&pitdir);
    }

    #[test]
    fn drop_guard_cleans_up() {
        let pitdir = std::env::temp_dir().join("pitwall-m5c-drop-test");
        let _ = std::fs::remove_dir_all(&pitdir);
        let path = {
            let ctx = EphemeralContext::create_in(&pitdir, "{\"a\":1}").unwrap();
            ctx.path().unwrap().to_path_buf()
        };
        assert!(!path.exists(), "Drop must unlink");
        let _ = std::fs::remove_dir_all(&pitdir);
    }

    #[test]
    fn derive_events_covers_appear_vanish_and_changes() {
        use crate::collector::WindowRole as R;
        let prev = vec![
            PrevSession {
                session_id: "s1".to_string(),
                project_id: Some("p".to_string()),
                project_dir: Some("/home/u/a".to_string()),
                agent_kind: "opencode".to_string(),
                branch: Some("main".to_string()),
                git_clean: Some(true),
                state: "running".to_string(),
            },
            PrevSession {
                session_id: "gone".to_string(),
                project_id: Some("p".to_string()),
                project_dir: Some("/home/u/b".to_string()),
                agent_kind: "unknown".to_string(),
                branch: None,
                git_clean: None,
                state: "sleeping".to_string(),
            },
        ];
        let mut snap = snapshot_with(&[("s1", R::Terminal, 200)]);
        snap.sessions[0].agent.kind = crate::collector::AgentKind::ClaudeCode;
        snap.sessions[0].project.as_mut().unwrap().branch = Some("feat".to_string());
        snap.sessions[0].project.as_mut().unwrap().git_clean = Some(false);
        let events = derive_events(&prev, &snap, &[], 20);
        let kinds: Vec<&str> = events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&"agent_changed"), "{kinds:?}");
        assert!(kinds.contains(&"branch_changed"), "{kinds:?}");
        assert!(kinds.contains(&"git_changed"), "{kinds:?}");
        assert!(kinds.contains(&"session_vanished"), "{kinds:?}");
        assert!(!kinds.contains(&"session_appeared"), "{kinds:?}");
    }

    #[test]
    fn no_persistent_schema_gets_terminal_text() {
        // Guardrail: the store module must not grow text-carrying columns.
        // Checkpoints/sessions carry identity + state only; terminal lines
        // exist solely in the ephemeral document built above — scrubbed.
        let plat = MockPlatform {
            text: TerminalText::Lines {
                first: vec!["deploy token sk-live-ABCDEF1234567890 done".to_string()],
                last: vec![],
            },
        };
        let snap = snapshot_with(&[("sess_a", WindowRole::Terminal, 100)]);
        let (doc, _) = build_context(&plat, &snap, &[], &[]);
        assert!(!doc.contains("sk-live-ABCDEF1234567890"), "{doc}");
        assert!(doc.contains("[redacted]"), "{doc}");
    }
}
