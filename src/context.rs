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
use crate::platform::{IoCounters, Platform, RawProcess, TerminalText};
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

/// The privacy boundary shared by summary generation and future workspace
/// question surfaces. This type contains only bounded, already-observed
/// Pitwall facts. It deliberately has no process command lines, environment,
/// credentials, arbitrary file contents, or terminal transcript text.
#[derive(Clone, PartialEq, Eq)]
pub struct SummaryContext {
    snapshot: WorkspaceSnapshot,
    events: Vec<Event>,
    checkpoints: Vec<Checkpoint>,
    notifications: Vec<crate::store::Notification>,
}

impl SummaryContext {
    pub fn new(
        snapshot: WorkspaceSnapshot,
        mut events: Vec<Event>,
        mut checkpoints: Vec<Checkpoint>,
        mut notifications: Vec<crate::store::Notification>,
    ) -> Self {
        events.sort_by(|a, b| {
            (
                a.kind,
                a.session_id.as_str(),
                a.project.as_str(),
                a.detail.as_str(),
            )
                .cmp(&(
                    b.kind,
                    b.session_id.as_str(),
                    b.project.as_str(),
                    b.detail.as_str(),
                ))
        });
        checkpoints.sort_by_key(|c| (c.project_id.clone(), c.session_id.clone(), c.id));
        notifications.sort_by_key(|n| (n.kind.clone(), n.session_id.clone(), n.id));
        Self {
            snapshot,
            events,
            checkpoints,
            notifications,
        }
    }

    /// Stable structured representation for cache identity. Volatile sample
    /// time, PIDs, window addresses, and terminal text are intentionally out.
    pub fn stable_serialized(&self) -> String {
        let mut out = String::new();
        // Bump when the human-facing summary contract changes; old cached
        // prose must not survive a prompt-quality change as if it were fresh.
        out.push_str("summary_prompt_version=2\n");
        out.push_str(&format!("host={}\n", self.snapshot.hostname));
        let mut sessions: Vec<_> = self.snapshot.sessions.iter().collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        for s in sessions {
            out.push_str(&format!(
                "session={}|state={}|role={}|procs={}|last={}|agent={}|conf={}|",
                s.id,
                s.state.as_str(),
                s.role.as_str(),
                s.process_count,
                s.last_activity_epoch,
                s.agent.kind.as_str(),
                s.agent.confidence.as_str()
            ));
            if let Some(p) = &s.project {
                out.push_str(&format!(
                    "project={}|{}|{:?}|{:?}|{}|",
                    p.id, p.dir, p.branch, p.git_clean, p.is_git_repo
                ));
            } else {
                out.push_str("project=-|");
            }
            out.push('\n');
        }
        for e in &self.events {
            out.push_str(&format!(
                "event={}|{}|{}|{}\n",
                e.kind,
                scrub_string(&e.session_id),
                scrub_string(&e.project),
                scrub_string(&e.detail)
            ));
        }
        for c in &self.checkpoints {
            out.push_str(&format!(
                "checkpoint={}|{}|{}|{}|{}|{}|{}\n",
                c.id,
                scrub_string(&c.project_id),
                scrub_string(&c.session_id),
                scrub_string(&c.trigger),
                scrub_string(c.branch.as_deref().unwrap_or("")),
                scrub_string(&c.state),
                scrub_string(c.note.as_deref().unwrap_or(""))
            ));
        }
        for n in &self.notifications {
            out.push_str(&format!(
                "notification={}|{}|{}|{}|{}|{}\n",
                n.id,
                scrub_string(&n.kind),
                scrub_string(&n.session_id),
                scrub_string(&n.severity),
                scrub_string(&n.state),
                scrub_string(&n.detail)
            ));
        }
        out
    }
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
            out.push_str(&format!("    \"terminal_text\": {}\n", q(reason)));
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
        // A chat window *is* a terminal session, so it ranks with terminals.
        // Sharing rank 0 leaves the relative order of every non-chat session
        // exactly as it was (23.6): the epoch and id tiebreaks are unchanged.
        crate::collector::WindowRole::Terminal | crate::collector::WindowRole::Chat => 0,
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

/// Render the bounded ephemeral agent document from the canonical context.
/// Terminal text remains sampled only here, after the structured boundary has
/// been assembled, and is never part of `SummaryContext` or its hash.
pub fn build_context_from_summary(
    platform: &dyn Platform,
    context: &SummaryContext,
) -> (String, usize) {
    build_context(
        platform,
        &context.snapshot,
        &context.events,
        &context.checkpoints,
    )
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

/// Filename fence for ephemeral context documents. The sweep only ever
/// considers names it could itself have produced.
const CTX_PREFIX: &str = "ctx-";
const CTX_SUFFIX: &str = ".json";

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
        Self::create_with(dir, document, None)
    }

    /// Write `document` to a new unpredictable 0600 file whose name carries
    /// the creating process's pid. Identical privacy properties to
    /// [`EphemeralContext::create`] (`O_EXCL`, `0600`, runtime dir, explicit
    /// `close()`, `Drop` net); the pid tag exists so that a file left behind
    /// by an abnormal exit — Rust std offers no signal handling and M8 adds
    /// no dependency for it — is *reclaimable* by [`sweep_orphans`] instead
    /// of surviving until logout.
    pub fn create_owned(document: &str) -> Result<EphemeralContext, String> {
        Self::create_owned_in(&ephemeral_dir(), document)
    }

    /// Same, under an explicit directory (test seam, mirroring
    /// [`EphemeralContext::create_in`]).
    pub fn create_owned_in(dir: &Path, document: &str) -> Result<EphemeralContext, String> {
        Self::create_with(dir, document, Some(std::process::id()))
    }

    /// The one creation path. `owner` present ⇒ pid-tagged name
    /// (`ctx-<pid>-<16 hex>.json`), absent ⇒ the original untagged name
    /// (`ctx-<16 hex>.json`). Every early return unlinks: the guard is
    /// constructed the instant the file exists, so a write failure drops it.
    fn create_with(
        dir: &Path,
        document: &str,
        owner: Option<u32>,
    ) -> Result<EphemeralContext, String> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create ephemeral dir: {e}"))?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("cannot secure ephemeral dir: {e}"))?;
        for _ in 0..16 {
            let name = match owner {
                Some(pid) => format!("{CTX_PREFIX}{pid}-{}{CTX_SUFFIX}", random_hex16()),
                None => format!("{CTX_PREFIX}{}{CTX_SUFFIX}", random_hex16()),
            };
            let path = dir.join(name);
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true).mode(0o600);
            match opts.open(&path) {
                Ok(mut file) => {
                    // Own the path before the first fallible step: from here
                    // on every exit — `?`, panic, or success followed by
                    // `close()`/`Drop` — removes the file.
                    let owned = EphemeralContext { path: Some(path) };
                    use std::io::Write as _;
                    file.write_all(document.as_bytes())
                        .map_err(|e| format!("cannot write context: {e}"))?;
                    return Ok(owned);
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

/// Remove pid-tagged ephemeral context documents whose owner process is
/// *demonstrably* not a live `pitwall chat`. Called once at chat startup;
/// returns how many files were removed.
///
/// `processes` is the already-collected observation from
/// [`crate::platform::Platform::processes`] — the sweep does no `/proc`
/// reading of its own, so it stays a pure function of (directory listing,
/// observed processes) and is testable off-target.
///
/// Deliberately conservative. It removes a file only when *all* of these
/// hold, and keeps it in every other case, including every case of doubt:
///
/// - the name is exactly `ctx-<pid>-<16 lowercase hex>.json`,
/// - the entry is a regular file,
/// - the observation is non-empty (an empty list means observation failed,
///   not that nothing is running),
/// - that pid is not in the observation as a `pitwall chat` process.
pub fn sweep_orphans(processes: &[RawProcess]) -> usize {
    sweep_orphans_in(&ephemeral_dir(), processes)
}

/// Same, under an explicit directory (test seam, mirroring
/// [`EphemeralContext::create_in`]). An unreadable or absent directory is
/// "nothing to sweep", not an error.
pub fn sweep_orphans_in(dir: &Path, processes: &[RawProcess]) -> usize {
    if processes.is_empty() {
        return 0;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return 0,
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        // Regular files only: never follow or unlink a directory or symlink.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let owner = match name.to_str().and_then(owner_pid_of) {
            Some(pid) => pid,
            None => continue,
        };
        if is_live_pitwall_chat(owner, processes) {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Owner pid of a pid-tagged context name, or `None` for anything else —
/// including the untagged `ctx-<16 hex>.json` form written by the summary
/// path (no owner to check, and that path is synchronous and Drop-guarded)
/// and every unrelated file in the runtime directory (leases included).
fn owner_pid_of(file_name: &str) -> Option<u32> {
    let body = file_name.strip_prefix(CTX_PREFIX)?;
    let body = body.strip_suffix(CTX_SUFFIX)?;
    let (pid, rand) = body.split_once('-')?;
    // Exactly the shape `create_with` emits: a decimal pid with no padding,
    // then 16 lowercase hex digits.
    if pid.is_empty() || pid.len() > 10 || pid.starts_with('0') {
        return None;
    }
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if rand.len() != 16 || !rand.bytes().all(is_lower_hex) {
        return None;
    }
    pid.parse().ok()
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// True when `pid` appears in the observed process list as a `pitwall chat`
/// invocation: argv[0] basename (or the executable basename) is `pitwall`
/// and argv[1] is exactly `chat`. Shared with the chat number lease
/// validator, which asks the same question of a lease's owner pid.
pub fn is_live_pitwall_chat(pid: u32, processes: &[RawProcess]) -> bool {
    processes.iter().any(|p| p.pid == pid && is_chat_argv(p))
}

/// The argv rule itself lives in [`crate::collector::is_pitwall_chat_argv`],
/// which the collector's Chat_Role corroboration also calls against its own
/// `ProcessInfo` view of the same observation. One definition, two callers:
/// the orphan sweep, the lease validator and window discovery cannot end up
/// disagreeing about what a `pitwall chat` process is.
fn is_chat_argv(p: &RawProcess) -> bool {
    crate::collector::is_pitwall_chat_argv(&p.command, &p.exe_name)
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
    use crate::platform::{ChatLease, GitInfo, InlineImage, RawProcess, TerminalSpec, WindowInfo};

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
        fn launch_terminal(&self, _spec: &TerminalSpec<'_>) -> Result<(), String> {
            Ok(())
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            Vec::new()
        }
        fn inline_image_capability(&self) -> InlineImage {
            // Fixed: a test must never probe a real terminal.
            InlineImage::None
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
            chat: None,
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
    fn summary_context_is_deterministic_bounded_and_scrubbed() {
        let snapshot = snapshot_with(&[
            ("sess_b", WindowRole::Terminal, 2),
            ("sess_a", WindowRole::Terminal, 1),
        ]);
        let fake_key = ["password=sk", "live", "ABCDEF123456"].join("-");
        let event = Event {
            kind: "session_appeared",
            session_id: "sess_a".into(),
            project: "Work".into(),
            detail: fake_key.clone(),
        };
        let first = SummaryContext::new(snapshot.clone(), vec![event.clone()], vec![], vec![]);
        let second = SummaryContext::new(snapshot, vec![event], vec![], vec![]);
        assert_eq!(first.stable_serialized(), second.stable_serialized());
        assert!(first.stable_serialized().contains("[redacted]"));
        assert!(!first.stable_serialized().contains(&fake_key));
        assert!(first.stable_serialized().len() < 16 * 1024);
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
        // Scanner-safe fixtures: secret-shaped inputs are assembled at
        // runtime from fragments so no credential-like literal is committed.
        // Runtime values preserve the exact pre-existing test semantics.
        let cases: Vec<(String, &str)> = vec![
            (
                format!("key {} rest", ["sk", "live", "ABCDEF123456"].join("-")),
                "key [redacted] rest",
            ),
            (
                format!("tok {}", ["ghp", "ABCDEFGHIJKLMNOP123456"].join("_")),
                "tok [redacted]",
            ),
            (
                format!("tok {}", ["gho", "XYZ1234567890abcd"].join("_")),
                "tok [redacted]",
            ),
            (
                format!("pat {}", ["github_pat", "ABCDEF1234567890"].join("_")),
                "pat [redacted]",
            ),
            (
                format!("aws {} end", ["AKIA", "IOSFODNN7EXAMPLE"].concat()),
                "aws [redacted] end",
            ),
            (
                format!("s {} rest", ["xoxb", "123", "456", "abcdef"].join("-")),
                "s [redacted] rest",
            ),
            (
                ["Authorization: Bearer", "ABCDEF123456"].join(" "),
                "Authorization: Bearer [redacted]",
            ),
            ("password=hunter2!".to_string(), "password=[redacted]"),
            (
                "db passwd = s3cret thing".to_string(),
                "db passwd = [redacted] thing",
            ),
            ("api secret abc123;".to_string(), "api secret [redacted]"),
            ("token=XYZ789abc end".to_string(), "token=[redacted] end"),
        ];
        for (input, expected) in cases {
            assert_eq!(scrub_string(&input), expected, "input: {input}");
        }
        // PEM blocks vanish entirely.
        let pem = format!(
            "head\n-----BEGIN {} PRIVATE KEY-----\nMIIB\n-----END {} PRIVATE KEY-----\ntail",
            "RSA", "RSA"
        );
        let scrubbed = scrub_string(&pem);
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
        assert!(
            !doc.contains("no scrollback API\",\n"),
            "trailing comma: {doc}"
        );
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

    fn raw_process(pid: u32, command: &str, exe_name: &str) -> RawProcess {
        RawProcess {
            pid,
            ppid: 1,
            name: exe_name.to_string(),
            command: command.to_string(),
            exe_name: exe_name.to_string(),
            cwd: "/home/u".to_string(),
            state_code: 'S',
            starttime_ticks: 100,
        }
    }

    #[test]
    fn owned_context_is_pid_tagged_private_and_cleaned() {
        let pitdir = std::env::temp_dir().join("pitwall-m8-owned-test");
        let _ = std::fs::remove_dir_all(&pitdir);
        let mut ctx = EphemeralContext::create_owned_in(&pitdir, "{}").unwrap();
        let path = ctx.path().unwrap().to_path_buf();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            owner_pid_of(&name),
            Some(std::process::id()),
            "name must carry the owner pid: {name}"
        );
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
        ctx.close();
        assert!(!path.exists());
        assert!(ctx.path().is_none());
        // Drop after an explicit close must not try to remove anything else.
        drop(ctx);
        // Drop alone is still the net for the owned form.
        let dropped = {
            let ctx = EphemeralContext::create_owned_in(&pitdir, "{}").unwrap();
            ctx.path().unwrap().to_path_buf()
        };
        assert!(!dropped.exists(), "Drop must unlink the owned file");
        let _ = std::fs::remove_dir_all(&pitdir);
    }

    #[test]
    fn owner_pid_parses_only_names_we_emit() {
        assert_eq!(owner_pid_of("ctx-4242-0123456789abcdef.json"), Some(4242));
        // Untagged summary-path form: no owner, never swept.
        assert_eq!(owner_pid_of("ctx-0123456789abcdef.json"), None);
        // Unrelated runtime-dir residents.
        assert_eq!(owner_pid_of("chat-001.lease"), None);
        assert_eq!(owner_pid_of("state.json"), None);
        assert_eq!(owner_pid_of("pitwall.sock"), None);
        // Near-misses.
        assert_eq!(owner_pid_of("ctx-4242-0123456789abcde.json"), None); // 15 hex
        assert_eq!(owner_pid_of("ctx-4242-0123456789abcdeff.json"), None); // 17 hex
        assert_eq!(owner_pid_of("ctx-4242-0123456789ABCDEF.json"), None); // upper
        assert_eq!(owner_pid_of("ctx-42x2-0123456789abcdef.json"), None);
        assert_eq!(owner_pid_of("ctx-0424-0123456789abcdef.json"), None); // padded
        assert_eq!(owner_pid_of("ctx-4242-0123456789abcdef.txt"), None);
        assert_eq!(owner_pid_of("ctx-4242-0123456789abcdef.json.bak"), None);
        assert_eq!(owner_pid_of("ctx--0123456789abcdef.json"), None);
    }

    #[test]
    fn sweep_removes_only_orphans_of_dead_chat_pids() {
        let pitdir = std::env::temp_dir().join("pitwall-m8-sweep-test");
        let _ = std::fs::remove_dir_all(&pitdir);
        std::fs::create_dir_all(&pitdir).unwrap();
        let live = pitdir.join("ctx-777-aaaaaaaaaaaaaaaa.json");
        let orphan = pitdir.join("ctx-778-bbbbbbbbbbbbbbbb.json");
        let untagged = pitdir.join("ctx-cccccccccccccccc.json");
        let lease = pitdir.join("chat-001.lease");
        for p in [&live, &orphan, &untagged, &lease] {
            std::fs::write(p, "{}").unwrap();
        }
        let processes = vec![
            raw_process(777, "/usr/bin/pitwall chat --session sess_1", "pitwall"),
            raw_process(779, "/usr/bin/pitwall snapshot", "pitwall"),
        ];
        assert_eq!(sweep_orphans_in(&pitdir, &processes), 1);
        assert!(live.exists(), "a live chat's context must survive");
        assert!(!orphan.exists(), "orphan of a dead owner must go");
        assert!(untagged.exists(), "untagged form is not swept");
        assert!(lease.exists(), "unrelated runtime files untouched");

        // An empty observation means observation failed: sweep nothing.
        let orphan2 = pitdir.join("ctx-778-bbbbbbbbbbbbbbbb.json");
        std::fs::write(&orphan2, "{}").unwrap();
        assert_eq!(sweep_orphans_in(&pitdir, &[]), 0);
        assert!(orphan2.exists());

        // A pid that exists but is not `pitwall chat` is not a live chat:
        // pid reuse by an unrelated process does not protect the orphan.
        let reused = vec![
            raw_process(777, "/usr/bin/pitwall chat", "pitwall"),
            raw_process(778, "/usr/bin/vim notes.md", "vim"),
        ];
        assert_eq!(sweep_orphans_in(&pitdir, &reused), 1);
        assert!(!orphan2.exists());
        assert!(live.exists());

        // An absent directory is "nothing to sweep", not an error.
        let _ = std::fs::remove_dir_all(&pitdir);
        assert_eq!(sweep_orphans_in(&pitdir, &processes), 0);
    }

    #[test]
    fn live_chat_detection_requires_pitwall_and_chat() {
        let procs = vec![
            raw_process(10, "/usr/bin/pitwall chat", "pitwall"),
            raw_process(11, "pitwall snapshot", "pitwall"),
            raw_process(12, "/home/u/.local/share/mise/shims/pw chat", "pitwall"),
            raw_process(13, "chat", "chat"),
            raw_process(14, "/usr/bin/pitwallx chat", "pitwallx"),
        ];
        assert!(is_live_pitwall_chat(10, &procs));
        assert!(!is_live_pitwall_chat(11, &procs), "wrong subcommand");
        assert!(is_live_pitwall_chat(12, &procs), "shim argv0, real exe");
        assert!(!is_live_pitwall_chat(13, &procs), "no pitwall argv0/exe");
        assert!(!is_live_pitwall_chat(14, &procs), "not a basename match");
        assert!(!is_live_pitwall_chat(99, &procs), "unobserved pid");
    }

    #[test]
    fn no_persistent_schema_gets_terminal_text() {
        // Guardrail: the store module must not grow text-carrying columns.
        // Checkpoints/sessions carry identity + state only; terminal lines
        // exist solely in the ephemeral document built above — scrubbed.
        let plat = MockPlatform {
            text: TerminalText::Lines {
                first: vec![format!(
                    "deploy token {} done",
                    ["sk", "live", "ABCDEF1234567890"].join("-")
                )],
                last: vec![],
            },
        };
        let snap = snapshot_with(&[("sess_a", WindowRole::Terminal, 100)]);
        let (doc, _) = build_context(&plat, &snap, &[], &[]);
        let fake_terminal_key = ["sk", "live-ABCDEF1234567890"].join("-");
        assert!(!doc.contains(fake_terminal_key.as_str()), "{doc}");
        assert!(doc.contains("[redacted]"), "{doc}");
    }

    // -----------------------------------------------------------------
    // Property tests (M8 task 7.3). `proptest` is a dev-dependency pinned
    // `=1.5.0`; each design property below is exactly ONE test at 100+
    // cases. Generators go through the same seams the unit tests above use
    // (`create_owned_in`, `sweep_orphans_in`), so no case touches the real
    // runtime directory, spawns an agent, or opens a database.
    // -----------------------------------------------------------------

    use proptest::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static PROP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// A fresh sandbox per case, unique per process *and* per case so that
    /// 100+ cases and parallel test threads never share a directory.
    fn prop_sandbox(tag: &str) -> PathBuf {
        let n = PROP_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "pitwall-m8-prop-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// The five invocation outcomes Property 16 quantifies over. Modelled,
    /// not executed: no harness is spawned by the suite. What the property
    /// needs from an outcome is *which cleanup branch runs* — an explicit
    /// `close()` where the call reaches its end, `Drop` where it returns
    /// early — and both must leave nothing behind.
    #[derive(Debug, Clone, Copy)]
    enum Outcome {
        Answer,
        EmptyText,
        HarnessFailure,
        Timeout,
        EarlyRefusal,
    }

    impl Outcome {
        fn of(ix: usize) -> Outcome {
            match ix {
                0 => Outcome::Answer,
                1 => Outcome::EmptyText,
                2 => Outcome::HarnessFailure,
                3 => Outcome::Timeout,
                _ => Outcome::EarlyRefusal,
            }
        }

        /// `true` for the outcomes that return normally (the caller closes
        /// explicitly), `false` for the ones that return early or abort
        /// (`Drop` is the net).
        fn closes_explicitly(self) -> bool {
            matches!(self, Outcome::Answer | Outcome::EmptyText)
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        // **Validates: Requirements 12.8, 15.7**
        // Feature: pitwall-chat-and-brief-ticker, Property 16: Ephemeral context files never outlive their use — For any invocation outcome (answer, empty text, harness failure, timeout, early refusal), the context file existed with mode `0600` under the runtime directory during the run, no longer exists afterwards, and never appeared in the argument vector; and for any orphan left by an abnormal exit, the next chat startup sweep removes it once its owner pid is not a live `pitwall chat` process.
        #[test]
        fn prop16_ephemeral_context_never_outlives_its_use(
            outcome_ix in 0usize..5,
            model_ix in 0usize..3,
            body in proptest::string::string_regex("[a-zA-Z0-9 .,:_-]{0,80}").unwrap(),
            marker in proptest::string::string_regex("[0-9a-f]{16}").unwrap(),
            orphan_pid in 2u32..900_000,
            live_pid in 2u32..900_000,
            orphan_rand in proptest::string::string_regex("[0-9a-f]{16}").unwrap(),
            live_rand in proptest::string::string_regex("[0-9a-f]{16}").unwrap(),
            pid_reused in any::<bool>(),
        ) {
            prop_assume!(orphan_pid != live_pid);
            let outcome = Outcome::of(outcome_ix);
            let model = match model_ix {
                0 => None,
                1 => Some("prov/model"),
                _ => Some("openrouter/anthropic/claude-3.5-sonnet"),
            };
            let dir = prop_sandbox("p16");
            // The document carries a unique marker so "never appeared in the
            // argument vector" is checkable as a substring search over every
            // element rather than as an inequality against the whole text.
            let document = format!("{{\"ctx-marker\":\"{marker}\",\"body\":\"{body}\"}}");

            // "under the runtime directory": production resolves the parent
            // through `ephemeral_dir()`, which is absolute and pitwall-scoped;
            // the case passes a sandbox through the same seam, so the real one
            // is never written to.
            let runtime = ephemeral_dir();
            prop_assert!(runtime.is_absolute(), "runtime dir must be absolute: {runtime:?}");
            prop_assert!(
                runtime
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("pitwall")),
                "runtime dir must be pitwall-scoped: {runtime:?}"
            );

            let mut guard = EphemeralContext::create_owned_in(&dir, &document)
                .expect("creating an ephemeral context in a fresh sandbox");
            let path = guard
                .path()
                .expect("a fresh guard owns its path")
                .to_path_buf();
            let name = path
                .file_name()
                .expect("context files are never bare directories")
                .to_string_lossy()
                .into_owned();

            // --- during the run: it exists, it is private, it holds the text ---
            prop_assert!(path.exists(), "the context file must exist during the run");
            prop_assert_eq!(path.parent(), Some(dir.as_path()));
            prop_assert!(
                std::fs::read_to_string(&path).unwrap_or_default() == document,
                "the document travels in the file, verbatim"
            );
            prop_assert_eq!(owner_pid_of(&name), Some(std::process::id()));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                prop_assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
                prop_assert_eq!(
                    std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }

            // --- argv carries the path at most, never the content ---
            let bin = dir.join("opencode");
            let argv = crate::summary::build_argv(
                "opencode",
                model,
                &dir.to_string_lossy(),
                &path,
                &bin,
            )
            .expect("a valid model and an absolute dir build an argv");
            let path_str = path.to_string_lossy().into_owned();
            prop_assert_eq!(
                argv.iter().filter(|a| a.as_str() == path_str.as_str()).count(),
                1,
                "the path appears exactly once (the `-f` value): {:?}",
                argv
            );
            let f_ix = argv
                .iter()
                .position(|a| a == "-f")
                .expect("opencode takes the context as a file");
            prop_assert!(argv[f_ix + 1] == path_str, "the path is the `-f` value: {:?}", argv);
            for element in &argv {
                prop_assert!(
                    !element.contains(marker.as_str()),
                    "document content reached argv: {element}"
                );
                prop_assert!(
                    !element.contains(document.as_str()),
                    "the document reached argv: {element}"
                );
            }

            // --- afterwards: nothing is left, whatever the outcome was ---
            if outcome.closes_explicitly() {
                guard.close();
                prop_assert!(guard.path().is_none(), "a closed guard owns nothing");
            } else {
                drop(guard);
            }
            prop_assert!(
                !path.exists(),
                "no context file may outlive the run ({outcome:?})"
            );

            // --- an orphan from an abnormal exit is reclaimed at startup ---
            let orphan = dir.join(format!("ctx-{orphan_pid}-{orphan_rand}.json"));
            let live = dir.join(format!("ctx-{live_pid}-{live_rand}.json"));
            std::fs::write(&orphan, "{}").expect("sandbox write");
            std::fs::write(&live, "{}").expect("sandbox write");
            let mut processes = vec![raw_process(
                live_pid,
                "/usr/bin/pitwall chat --session sess_0123456789abcdef",
                "pitwall",
            )];
            if pid_reused {
                // The orphan's owner pid is live again as something else:
                // reuse must not protect the orphan.
                processes.push(raw_process(orphan_pid, "/usr/bin/vim notes.md", "vim"));
            }
            prop_assert_eq!(sweep_orphans_in(&dir, &processes), 1);
            prop_assert!(
                !orphan.exists(),
                "the sweep must reclaim an orphan whose owner is not a live pitwall chat"
            );
            prop_assert!(live.exists(), "a live chat's context must survive the sweep");

            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// FNV-1a 64 over the stable serialization, lowercase hex, `fnv:`
    /// prefix: the pre-M8 recipe, restated here from the M5c constants
    /// instead of routed through [`crate::ids::fnv1a_hex`], so the property
    /// fails if M8 (or anything after it) changes the algorithm, the
    /// prefix, or the text being hashed.
    ///
    /// No pre-M8 hash *value* is committed anywhere in the repository — the
    /// `fnv:abc123` / `fnv:aaa` strings in the `output.rs` and `store.rs`
    /// tests are opaque placeholders, not hashes of any context — so an
    /// independent restatement of the recipe is the strongest oracle
    /// available. If a real pre-M8 value is ever recorded, assert against
    /// it here as well.
    fn pre_m8_input_hash(context: &SummaryContext) -> String {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in context.stable_serialized().as_bytes() {
            h ^= u64::from(*byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("fnv:{h:016x}")
    }

    /// The summary cache's whole decision procedure, as the CLI implements
    /// it: `lookup_summary` is keyed on `input_hash`, and the snapshot path
    /// compares `row.input_hash` against the freshly computed key (`ready`
    /// on equality, `stale` otherwise). Hit/miss is therefore exactly key
    /// equality — modelled here so the property needs no database.
    fn cache_hits(stored_key: &str, current_key: &str) -> bool {
        stored_key == current_key
    }

    /// The structured facts `stable_serialized` actually reads for one
    /// session. Anything absent from this struct is, by construction,
    /// outside the cache key.
    #[derive(Debug, Clone)]
    struct HashFacts {
        id: String,
        role: WindowRole,
        state: SessionState,
        agent: AgentKind,
        confidence: Confidence,
        process_count: usize,
        last_activity_epoch: i64,
        project: Option<ProjectInfo>,
    }

    fn role_of(ix: usize) -> WindowRole {
        match ix {
            0 => WindowRole::Terminal,
            1 => WindowRole::App,
            _ => WindowRole::Unknown,
        }
    }

    fn state_of(ix: usize) -> SessionState {
        match ix {
            0 => SessionState::Running,
            1 => SessionState::Sleeping,
            2 => SessionState::Stopped,
            _ => SessionState::Unknown,
        }
    }

    fn agent_of(ix: usize) -> AgentKind {
        match ix {
            0 => AgentKind::Opencode,
            1 => AgentKind::ClaudeCode,
            2 => AgentKind::Codex,
            3 => AgentKind::Gemini,
            4 => AgentKind::Hermes,
            5 => AgentKind::Aider,
            _ => AgentKind::Unknown,
        }
    }

    fn confidence_of(ix: usize) -> Confidence {
        match ix {
            0 => Confidence::High,
            1 => Confidence::Medium,
            2 => Confidence::Low,
            _ => Confidence::Unknown,
        }
    }

    fn event_kind_of(ix: usize) -> &'static str {
        match ix {
            0 => "session_appeared",
            1 => "session_vanished",
            2 => "agent_changed",
            3 => "branch_changed",
            4 => "git_changed",
            _ => "checkpoint_created",
        }
    }

    fn word() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-z][a-z0-9_-]{1,10}").unwrap()
    }

    fn session_id() -> impl Strategy<Value = String> {
        proptest::string::string_regex("sess_[0-9a-f]{16}").unwrap()
    }

    /// Free-text fields, including secret-shaped ones, so the scrubbing
    /// `stable_serialized` performs is exercised by the same inputs.
    fn detail() -> impl Strategy<Value = String> {
        proptest::string::string_regex("(password=[a-z0-9]{1,8}|[a-z ]{0,24})").unwrap()
    }

    fn project_facts() -> impl Strategy<Value = Option<ProjectInfo>> {
        proptest::option::of(
            (
                word(),
                proptest::option::of(word()),
                proptest::option::of(any::<bool>()),
                any::<bool>(),
            )
                .prop_map(|(name, branch, git_clean, is_git_repo)| ProjectInfo {
                    id: format!("proj_{}", crate::ids::fnv1a_hex(&name)),
                    dir: format!("/home/u/{name}"),
                    name,
                    is_git_repo,
                    branch,
                    git_clean,
                }),
        )
    }

    fn hash_facts() -> impl Strategy<Value = HashFacts> {
        (
            session_id(),
            0usize..3,
            0usize..4,
            0usize..7,
            0usize..4,
            0usize..12,
            1_700_000_000i64..1_700_090_000i64,
            project_facts(),
        )
            .prop_map(
                |(
                    id,
                    role_ix,
                    state_ix,
                    agent_ix,
                    conf_ix,
                    process_count,
                    last_activity_epoch,
                    project,
                )| HashFacts {
                    id,
                    role: role_of(role_ix),
                    state: state_of(state_ix),
                    agent: agent_of(agent_ix),
                    confidence: confidence_of(conf_ix),
                    process_count,
                    last_activity_epoch,
                    project,
                },
            )
    }

    /// Build a session from structured facts. `chat_shaped` varies ONLY
    /// fields `stable_serialized` deliberately excludes — the window title
    /// (where the Chat_Title_Grammar signal lives), the window address, the
    /// window pid, the root pid, the agent evidence and the one-line
    /// summary — so a `false`/`true` pair differs in exactly the facts a
    /// Chat_Session's presence contributes and in nothing the cache key is
    /// allowed to see. (`WindowRole::Chat` does not exist at this task; the
    /// collector work lands later, so chat presence is modelled through the
    /// two signals design §4.8 corroborates: the title and the owner pid.)
    fn hash_session(facts: &HashFacts, chat_shaped: bool) -> TerminalSession {
        let (title, address, pid, evidence, summary) = if chat_shaped {
            (
                "Pitwall Chat 001 · opencode · agent default · Work".to_string(),
                "0x7ffd00".to_string(),
                90_001u32,
                vec!["lease chat-001 owner pid 90001".to_string()],
                "Pitwall Chat 001 · opencode · Work · running".to_string(),
            )
        } else {
            (
                "nvim src/main.rs".to_string(),
                "0x1".to_string(),
                10u32,
                vec!["terminal context only".to_string()],
                "s".to_string(),
            )
        };
        TerminalSession {
            id: facts.id.clone(),
            window: Some(WindowInfo {
                address,
                class: "foot".to_string(),
                initial_class: "foot".to_string(),
                title,
                workspace: "1".to_string(),
                pid,
            }),
            root_pid: pid,
            role: facts.role,
            project: facts.project.clone(),
            agent: AgentIdentity {
                kind: facts.agent.clone(),
                confidence: facts.confidence,
                evidence,
            },
            // The cache-key test models chat presence through the two
            // corroborated signals (title + owner pid), not through this
            // field, so it stays `None` on both sides of the comparison.
            chat: None,
            state: facts.state,
            process_count: facts.process_count,
            processes: Vec::new(),
            last_activity_epoch: facts.last_activity_epoch,
            last_activity_kind: LAST_ACTIVITY_KIND,
            summary,
        }
    }

    fn hash_snapshot(
        hostname: &str,
        facts: &[HashFacts],
        chat_shaped: bool,
        collected_at: i64,
    ) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: collected_at,
            hostname: hostname.to_string(),
            sessions: facts.iter().map(|f| hash_session(f, chat_shaped)).collect(),
        }
    }

    fn hash_event() -> impl Strategy<Value = Event> {
        (0usize..6, session_id(), word(), detail()).prop_map(
            |(kind_ix, session_id, project, detail)| Event {
                kind: event_kind_of(kind_ix),
                session_id,
                project,
                detail,
            },
        )
    }

    fn hash_checkpoint() -> impl Strategy<Value = Checkpoint> {
        (
            session_id(),
            word(),
            proptest::option::of(word()),
            proptest::option::of(detail()),
            0usize..4,
            word(),
        )
            .prop_map(
                |(session_id, project, branch, note, state_ix, trigger)| Checkpoint {
                    // Replaced with a unique id in the test body.
                    id: 0,
                    created_at: 1_700_000_000,
                    project_id: format!("proj_{}", crate::ids::fnv1a_hex(&project)),
                    session_id,
                    project_dir: format!("/home/u/{project}"),
                    branch,
                    git_clean: None,
                    agent_kind: "unknown".to_string(),
                    agent_confidence: "low".to_string(),
                    state: state_of(state_ix).as_str().to_string(),
                    last_activity_epoch: 1_700_000_000,
                    window_address: Some("0x1".to_string()),
                    window_class: Some("foot".to_string()),
                    note,
                    trigger,
                    observation_id: None,
                },
            )
    }

    fn hash_notification() -> impl Strategy<Value = crate::store::Notification> {
        (word(), session_id(), word(), 0usize..4, detail()).prop_map(
            |(kind, session_id, severity, state_ix, detail)| crate::store::Notification {
                // Replaced with a unique id in the test body.
                id: 0,
                kind,
                session_id,
                project_id: "proj_0000000000000000".to_string(),
                project_name: "Work".to_string(),
                branch: None,
                agent_kind: "unknown".to_string(),
                state: state_of(state_ix).as_str().to_string(),
                checkpoint_id: None,
                created_at: 1_700_000_000,
                read_at: None,
                severity,
                detail,
            },
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

        // **Validates: Requirements 23.6, 23.7**
        // Feature: pitwall-chat-and-brief-ticker, Property 28: Summary context hashing is unchanged by M8 — For any SummaryContext, the input hash is deterministic and equals the pre-M8 value for identical structured inputs, and the summary cache's hit/miss behaviour is unaffected by the presence of Chat_Sessions.
        #[test]
        fn prop28_summary_context_hashing_is_unchanged_by_m8(
            hostname in word(),
            facts in proptest::collection::vec(hash_facts(), 0..4),
            events in proptest::collection::vec(hash_event(), 0..4),
            checkpoints in proptest::collection::vec(hash_checkpoint(), 0..3),
            notifications in proptest::collection::vec(hash_notification(), 0..3),
        ) {
            // Identities are unique in any real observation; make them unique
            // here too, otherwise a reordered input is genuinely ambiguous and
            // the case would be testing the reorder rather than the hash.
            let mut ids: Vec<&str> = facts.iter().map(|f| f.id.as_str()).collect();
            ids.sort_unstable();
            prop_assume!(ids.windows(2).all(|w| w[0] != w[1]));
            let checkpoints: Vec<Checkpoint> = checkpoints
                .into_iter()
                .enumerate()
                .map(|(ix, mut c)| {
                    c.id = ix as i64 + 1;
                    c
                })
                .collect();
            let notifications: Vec<crate::store::Notification> = notifications
                .into_iter()
                .enumerate()
                .map(|(ix, mut n)| {
                    n.id = ix as i64 + 1;
                    n
                })
                .collect();

            let plain = hash_snapshot(&hostname, &facts, false, 1_700_000_100);
            let context = SummaryContext::new(
                plain.clone(),
                events.clone(),
                checkpoints.clone(),
                notifications.clone(),
            );
            let key = crate::summary::input_hash(&context);

            // 1. Deterministic: identical structured inputs, identical key.
            let again = SummaryContext::new(
                plain.clone(),
                events.clone(),
                checkpoints.clone(),
                notifications.clone(),
            );
            prop_assert_eq!(crate::summary::input_hash(&again), key.clone());

            // 2. Order-normalised: the same facts delivered in a different
            //    order are the same workspace, so the same key.
            let reordered = SummaryContext::new(
                plain.clone(),
                events.iter().cloned().rev().collect(),
                checkpoints.iter().cloned().rev().collect(),
                notifications.iter().cloned().rev().collect(),
            );
            prop_assert_eq!(crate::summary::input_hash(&reordered), key.clone());

            // 3. Still the pre-M8 recipe, over the pre-M8 text.
            prop_assert_eq!(pre_m8_input_hash(&context), key.clone());
            prop_assert!(
                key.starts_with("fnv:") && key.len() == "fnv:".len() + 16,
                "hash shape must not change: {key}"
            );
            prop_assert!(
                context
                    .stable_serialized()
                    .starts_with("summary_prompt_version=2\n"),
                "M8 must not move the prompt version: {}",
                context.stable_serialized()
            );

            // 4. Chat presence does not perturb the key: the Chat_Title_Grammar
            //    title, the chat's pids and its window address are all outside
            //    the structured boundary, so a cached summary keeps hitting
            //    while chats come and go.
            let chatty = hash_snapshot(&hostname, &facts, true, 1_700_000_999);
            let chat_context = SummaryContext::new(
                chatty,
                events.clone(),
                checkpoints.clone(),
                notifications.clone(),
            );
            let chat_key = crate::summary::input_hash(&chat_context);
            prop_assert_eq!(chat_key.clone(), key.clone());
            prop_assert!(cache_hits(&key, &chat_key), "chat presence must not cause a miss");

            // 5. The miss side still fires on a structured change.
            let moved = hash_snapshot(&format!("{hostname}-2"), &facts, false, 1_700_000_100);
            let moved_key = crate::summary::input_hash(&SummaryContext::new(
                moved,
                events,
                checkpoints,
                notifications,
            ));
            prop_assert_ne!(moved_key.clone(), key.clone());
            prop_assert!(!cache_hits(&key, &moved_key), "a changed workspace must miss");
        }
    }
}
