//! Workspace collector: pure observation → normalized snapshot.
//!
//! The collector joins three evidence sources (compositor windows, OS
//! processes, git context) into [`WorkspaceSnapshot`]. It knows nothing
//! about Linux, Hyprland, or `/proc` — it only speaks the [`Platform`]
//! trait, so the core stays portable.
//!
//! Honesty rules (non-negotiable):
//!
//! - every heuristic carries [`Confidence`]; nothing is stated as fact
//!   without evidence (see `evidence` fields);
//! - `last_activity` is the latest descendant *start time*, labeled
//!   [`LAST_ACTIVITY_KIND`]; it is NOT true user activity;
//! - terminal emulator roots report cwd `/` (verified live), so the project
//!   directory is the **mode cwd of descendant processes**, never the root.

use crate::ids;
use crate::platform::{ChatLease, Platform, RawProcess, WindowInfo};
use std::collections::HashMap;

/// Schema version of [`WorkspaceSnapshot`]. The future panel/runtime treats
/// this as an API boundary: bump on any breaking field change.
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

/// Heuristic label for `last_activity`. Latest child-process start time —
/// a proxy for "something happened here recently", never true idle state.
pub const LAST_ACTIVITY_KIND: &str = "latest_child_start";

/// Cap on stored processes per session (snapshot stays small).
pub const MAX_SESSION_PROCESSES: usize = 32;

/// How certain a derived fact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Unknown,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
            Confidence::Unknown => "unknown",
        }
    }
}

/// AI agent kinds the collector can recognize from process evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentKind {
    Opencode,
    ClaudeCode,
    Codex,
    Gemini,
    Hermes,
    Aider,
    Unknown,
}

impl AgentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Opencode => "opencode",
            AgentKind::ClaudeCode => "claude-code",
            AgentKind::Codex => "codex",
            AgentKind::Gemini => "gemini",
            AgentKind::Hermes => "hermes",
            AgentKind::Aider => "aider",
            AgentKind::Unknown => "unknown",
        }
    }
}

/// Agent attribution with its evidence trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    pub kind: AgentKind,
    pub confidence: Confidence,
    /// Human-readable evidence, e.g. `cmd:opencode(pid 328900)`.
    pub evidence: Vec<String>,
}

/// Normalized per-process state (from the platform's raw state code).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Sleeping,
    DiskSleep,
    Stopped,
    Zombie,
    Idle,
    Dead,
    Unknown,
}

impl ProcessState {
    pub fn from_code(code: char) -> Self {
        match code {
            'R' => ProcessState::Running,
            'S' => ProcessState::Sleeping,
            'D' => ProcessState::DiskSleep,
            'T' | 't' => ProcessState::Stopped,
            'Z' => ProcessState::Zombie,
            'I' => ProcessState::Idle,
            'X' | 'x' => ProcessState::Dead,
            _ => ProcessState::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ProcessState::Running => "running",
            ProcessState::Sleeping => "sleeping",
            ProcessState::DiskSleep => "disk-sleep",
            ProcessState::Stopped => "stopped",
            ProcessState::Zombie => "zombie",
            ProcessState::Idle => "idle",
            ProcessState::Dead => "dead",
            ProcessState::Unknown => "unknown",
        }
    }
}

/// One process in a session tree, normalized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command: String,
    /// Basename of the executable (never a full path). Independent signal
    /// from argv: shims and wrappers lie in argv[0] but not in exe.
    pub exe_name: String,
    pub cwd: String,
    pub state: ProcessState,
    /// Seconds since Unix epoch; `-1` when the platform could not tell.
    pub started_at_epoch: i64,
}

/// Project context for a session directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInfo {
    /// Stable local ID (`proj_<hex>` of the directory). Same dir ⇒ same ID.
    pub id: String,
    pub dir: String,
    /// Basename of the directory (human label, not identity).
    pub name: String,
    pub is_git_repo: bool,
    pub branch: Option<String>,
    pub git_clean: Option<bool>,
}

/// What kind of window hosts a session. Terminal emulators (verified
/// Omarchy classes) are detection contexts for agents; anything else with
/// a process tree (browsers, GUI apps) is an `App` — agent inference is
/// skipped there rather than reporting a misleading `Unknown`.
///
/// `Chat` is different in kind from the other three: it is **not** derived
/// from a window class. It is an *upgrade* applied after the process-tree
/// walk, only when both Chat_Role signals corroborate each other (see
/// [`chat_facts_for`]). [`role_for_class`] never returns it, so every
/// class-based classification result is exactly what it was before Chat
/// existed (Requirements 17.3, 17.4, 23.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    Terminal,
    App,
    Unknown,
    /// A Pitwall Chat window, proven by title grammar **and** by a leased
    /// `pitwall chat` process inside this window's tree.
    Chat,
}

impl WindowRole {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowRole::Terminal => "terminal",
            WindowRole::App => "app",
            WindowRole::Unknown => "unknown",
            WindowRole::Chat => "chat",
        }
    }
}

/// Classify a window from its app-id/class. Only verified terminal classes
/// count as terminals (Omarchy default-terminal set + agent convention);
/// empty classes are Unknown; everything else is an App window that merely
/// happens to own processes.
pub fn role_for_class(class: &str, initial_class: &str) -> WindowRole {
    const TERMINALS: &[&str] = &[
        "foot",
        "kitty",
        "alacritty",
        "ghostty",
        "wezterm",
        "xterm",
        "org.omarchy.agent",
    ];
    let mut saw_any = false;
    for candidate in [class, initial_class] {
        let c = candidate.trim().to_lowercase();
        if c.is_empty() {
            continue;
        }
        saw_any = true;
        if TERMINALS.contains(&c.as_str()) {
            return WindowRole::Terminal;
        }
    }
    if saw_any {
        WindowRole::App
    } else {
        WindowRole::Unknown
    }
}

// ---------------------------------------------------------------------------
// Chat_Role corroboration (design §4.8)
//
// Two independent signals, both required:
//
//   A. Chat_Title_Grammar — the observed window title parses to the strict
//      grammar in `crate::chat::parse_title`;
//   B. Chat_Process_Identity — the number *in that title* names a chat
//      lease whose owner pid is present in *this* window's process tree and
//      is a `pitwall chat` invocation.
//
// Neither signal reads a window class, an app-id, `$TERM`, or an emulator
// name, so the Chat_Role extends the existing identity model instead of
// depending on a particular terminal (Requirement 28.12).
//
// Signal A alone is forgeable: anyone can `echo` the exact title. Signal B
// cannot be produced by printing anything — it requires a lease file the
// running chat created with `O_EXCL` under the runtime directory *and* that
// same pid to be observed inside this window's tree. So a spoofed title
// yields no upgrade and the window keeps its existing role, agent kind,
// confidence and evidence (17.3).
// ---------------------------------------------------------------------------

/// Is this process a `pitwall chat` invocation?
///
/// The rule: argv[0]'s basename **or** the observed executable basename is
/// `pitwall`, and argv[1] is exactly `chat`. Reading exe as an independent
/// signal is the same allowance [`classify_agent`] makes — shims lie in
/// argv[0], not in exe.
///
/// This is the single definition of that rule in the tree.
/// [`crate::context::is_live_pitwall_chat`] asks the same question about a
/// lease owner pid and calls straight into this function, so the collector's
/// corroboration and the lease validator provably cannot drift apart. The
/// signature takes the two observed strings rather than a struct because the
/// callers hold different shapes of the same observation
/// ([`ProcessInfo`] here, [`RawProcess`] there) whose `command` and
/// `exe_name` are field-for-field the same values.
pub fn is_pitwall_chat_argv(command: &str, exe_name: &str) -> bool {
    let mut argv = command.split_whitespace();
    let argv0 = argv.next().unwrap_or("");
    let argv1 = argv.next().unwrap_or("");
    let basename = argv0.rsplit('/').next().unwrap_or("");
    (basename == "pitwall" || exe_name == "pitwall") && argv1 == "chat"
}

/// Observed facts about one discovered Pitwall Chat window.
///
/// Everything here is read out of the window title (which the chat itself
/// set, and which the grammar validated) plus the start time of the proven
/// chat process. Nothing is inferred and no chat that is not running can be
/// described (17.7).
///
/// What this type deliberately cannot carry (22.7): conversation text,
/// environment values, command lines, window addresses, pids. The state
/// writer has no parameter that could smuggle them in because they are not
/// fields here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatFacts {
    /// Chat number, `1..=999`.
    pub number: u16,
    /// Harness id, guaranteed to be a [`crate::agents::KNOWN`] id.
    pub harness: String,
    /// Model id; empty means the harness default (`agent default`).
    pub model: String,
    /// Project name, or `Workspace` for a whole-workspace chat.
    pub context_label: String,
    /// Pitwall-local context session id, when it can be observed.
    ///
    /// Always `None` from this function, and honestly so: the
    /// Chat_Title_Grammar carries the context *label*, not the session id
    /// (design §4.4/§5.1), and the collector has no other observation that
    /// would reveal it — the id lives in the running chat's descriptor,
    /// which is process-private. Inventing one would breach 17.7, and
    /// widening the window title to carry it is a grammar change, not a
    /// collector change. The field exists because the state artifact's shape
    /// (§5.3) includes it and a future observable source may fill it.
    pub context_session_id: Option<String>,
    /// Start epoch of the proven `pitwall chat` process; `-1` when the
    /// platform could not tell.
    pub started_at_epoch: i64,
}

impl ChatFacts {
    /// Three-digit form, identical to the digits in the window title and in
    /// the lease file name.
    pub fn number_text(&self) -> String {
        format!("{:03}", self.number)
    }
}

/// Corroborate the two Chat_Role signals for one window. Pure: no I/O, no
/// clock, no config, no platform — everything it decides on is in its
/// arguments, so it is fully testable off-target.
///
/// `tree` is the window's own process tree as already collected (root
/// included, the observer's own pid excluded); `leases` is what
/// [`crate::platform::Platform::chat_leases`] observed. Liveness needs no
/// extra check: a pid is only in `tree` because it was observed running in
/// this window, and [`is_pitwall_chat_argv`] then decides that what runs
/// there is a chat.
///
/// Returns `None` — meaning "not a chat, keep the existing classification" —
/// when the title does not parse, when no lease claims the title's number,
/// when the leaseholder pid is not in this window's tree, or when that pid
/// is not a `pitwall chat` process.
pub fn chat_facts_for(
    window_title: &str,
    tree: &[ProcessInfo],
    leases: &[ChatLease],
) -> Option<ChatFacts> {
    // Signal A: the reserved title grammar, strictly.
    let title = crate::chat::parse_title(window_title)?;

    // Signal B: the number in *that* title must name a lease whose owner is
    // running in *this* window's tree as `pitwall chat`. The number↔lease↔pid
    // binding is what keeps a server-mode terminal's sibling windows out of
    // it: only the window whose title claims the number can be upgraded.
    let lease = leases.iter().find(|l| l.number == title.number())?;
    let owner = tree.iter().find(|p| p.pid == lease.pid)?;
    if !is_pitwall_chat_argv(&owner.command, &owner.exe_name) {
        return None;
    }

    Some(ChatFacts {
        number: title.number(),
        harness: title.harness().to_string(),
        model: title.model().to_string(),
        context_label: title.context_label().to_string(),
        context_session_id: None,
        started_at_epoch: owner.started_at_epoch,
    })
}

/// Aggregate liveness of a session tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Sleeping,
    Stopped,
    Unknown,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Running => "running",
            SessionState::Sleeping => "sleeping",
            SessionState::Stopped => "stopped",
            SessionState::Unknown => "unknown",
        }
    }
}

/// One terminal session: a compositor window plus its process tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSession {
    /// Stable-while-alive ID (`sess_<hex>`). See [`crate::ids`].
    pub id: String,
    pub window: Option<WindowInfo>,
    pub root_pid: u32,
    /// Window role (terminal/app/unknown). Agent inference only runs for
    /// terminal (and unknown-role) windows; App windows skip it.
    pub role: WindowRole,
    pub project: Option<ProjectInfo>,
    pub agent: AgentIdentity,
    /// Observed chat facts when this window is a corroborated Pitwall Chat
    /// (`role == WindowRole::Chat`), `None` for every other window. Agent
    /// inference above is untouched either way: a chat window whose tree
    /// momentarily contains a harness process still reports that evidence
    /// honestly.
    pub chat: Option<ChatFacts>,
    pub state: SessionState,
    /// Total processes in the tree (may exceed `processes.len()` cap).
    pub process_count: usize,
    pub processes: Vec<ProcessInfo>,
    /// Latest descendant start epoch; `-1` when unknown.
    pub last_activity_epoch: i64,
    /// Always [`LAST_ACTIVITY_KIND`] in M1 — the heuristic label.
    pub last_activity_kind: &'static str,
    /// One-line deterministic summary (human aid, not identity).
    pub summary: String,
}

/// Whole-workspace observation at one instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub schema_version: u32,
    pub collected_at_epoch: i64,
    pub hostname: String,
    pub sessions: Vec<TerminalSession>,
}

/// Basename of argv[0]-ish command, lowercased, for agent matching.
fn command_basename(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    base.to_lowercase()
}

/// Classify the agent running in a session tree.
///
/// Evidence counted (each adds one evidence string):
/// - a descendant command basename containing a known agent token;
/// - a descendant **exe basename** containing a token (independent of
///   argv: shims/wrappers lie in argv[0], not in exe);
/// - window class `org.omarchy.agent` (Omarchy agent-terminal convention);
/// - window title prefix (`OC |`, `CC |`, …) matching the agent family.
///
/// `High` needs ≥2 corroborating signals; a lone title hint is `Low`;
/// a lone command/exe match is `Medium`; nothing at all is honest
/// Unknown — `Low` with reason `terminal context only` for terminal
/// windows (we looked, found nothing), plain `Unknown` for App windows
/// (agent inference does not apply there at all).
fn classify_agent(
    processes: &[ProcessInfo],
    window: Option<&WindowInfo>,
    role: WindowRole,
) -> AgentIdentity {
    if role == WindowRole::App {
        return AgentIdentity {
            kind: AgentKind::Unknown,
            confidence: Confidence::Unknown,
            evidence: Vec::new(),
        };
    }
    // (command token, kind, title prefix)
    const KNOWN: &[(&str, AgentKind, &str)] = &[
        ("opencode", AgentKind::Opencode, "OC |"),
        ("claude", AgentKind::ClaudeCode, "CC |"),
        ("codex", AgentKind::Codex, "CX |"),
        ("gemini", AgentKind::Gemini, "GM |"),
        ("hermes", AgentKind::Hermes, "HM |"),
        ("aider", AgentKind::Aider, "AD |"),
    ];

    let mut cmd_hits: HashMap<usize, Vec<String>> = HashMap::new();
    for p in processes {
        let base = command_basename(&p.command);
        let base_or_name = if base.is_empty() {
            p.name.to_lowercase()
        } else {
            base
        };
        let exe = p.exe_name.to_lowercase();
        for (idx, (token, _, _)) in KNOWN.iter().enumerate() {
            if base_or_name.contains(token) {
                cmd_hits
                    .entry(idx)
                    .or_default()
                    .push(format!("cmd:{} (pid {})", base_or_name, p.pid));
            }
            // Exe evidence is independent: a wrapper in argv does not
            // change what binary actually runs.
            if !exe.is_empty() && exe.contains(token) && !base_or_name.contains(token) {
                cmd_hits
                    .entry(idx)
                    .or_default()
                    .push(format!("exe:{} (pid {})", exe, p.pid));
            }
        }
    }

    let agent_window = window
        .map(|w| w.class == "org.omarchy.agent" || w.initial_class == "org.omarchy.agent")
        .unwrap_or(false);
    let title = window.map(|w| w.title.as_str()).unwrap_or("");

    // Prefer the family with command evidence; else title-only (Low); else Unknown.
    let mut best: Option<usize> = None;
    for (idx, (_, _, prefix)) in KNOWN.iter().enumerate() {
        if cmd_hits.contains_key(&idx) {
            best = Some(idx);
            break;
        }
        if best.is_none() && !prefix.is_empty() && title.starts_with(prefix) {
            best = Some(idx);
        }
    }

    let Some(idx) = best else {
        // Honest Unknown: App windows were never in the agent domain;
        // terminal windows were examined and yielded nothing.
        if role == WindowRole::App {
            return AgentIdentity {
                kind: AgentKind::Unknown,
                confidence: Confidence::Unknown,
                evidence: Vec::new(),
            };
        }
        return AgentIdentity {
            kind: AgentKind::Unknown,
            confidence: Confidence::Low,
            evidence: vec!["terminal context only".to_string()],
        };
    };

    let mut evidence = cmd_hits.remove(&idx).unwrap_or_default();
    let has_cmd = !evidence.is_empty();
    let title_hit = title.starts_with(KNOWN[idx].2);
    if agent_window {
        evidence.push("window:class org.omarchy.agent".to_string());
    }
    if title_hit {
        evidence.push(format!("window:title prefix {:?}", KNOWN[idx].2));
    }
    let corroboration = usize::from(has_cmd) + usize::from(agent_window) + usize::from(title_hit);
    let confidence = match corroboration {
        0 => Confidence::Unknown, // unreachable: best implies ≥1 signal
        1 if !has_cmd => Confidence::Low,
        1 => Confidence::Medium,
        _ => Confidence::High,
    };
    AgentIdentity {
        kind: KNOWN[idx].1.clone(),
        confidence,
        evidence,
    }
}

/// Derive the session state from its tree, order-independently.
///
/// Severity precedence (highest wins regardless of pid order):
/// Stopped > Running/DiskSleep > Sleeping/Idle > Unknown.
/// Zombie/Dead/empty trees contribute nothing: a tree of only zombies is
/// `Unknown`, and a single Stopped process outranks any number of Running
/// ones (a waiting job must never be masked by an unrelated live child).
fn derive_session_state(processes: &[ProcessInfo]) -> SessionState {
    let mut severity = 0u8;
    for p in processes {
        let rank = match p.state {
            ProcessState::Stopped => 3,
            ProcessState::Running | ProcessState::DiskSleep => 2,
            ProcessState::Sleeping | ProcessState::Idle => 1,
            ProcessState::Zombie | ProcessState::Dead | ProcessState::Unknown => 0,
        };
        if rank > severity {
            severity = rank;
        }
    }
    match severity {
        3 => SessionState::Stopped,
        2 => SessionState::Running,
        1 => SessionState::Sleeping,
        _ => SessionState::Unknown,
    }
}

/// Project directory = mode cwd across descendant processes (ties broken by
/// lexicographic order for determinism). Emulator roots (`/`, empty,
/// unreadable) are excluded — verified live that they carry no signal.
/// Pseudo-filesystem cwds (`/proc`, `/sys`, `/dev` subtrees) are also
/// excluded: they are kernel artifacts (e.g. a browser helper reporting
/// `/proc/<pid>/fdinfo`), never projects. Without this, unrelated app
/// windows can share a spurious project identity.
fn project_dir_for(processes: &[ProcessInfo]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in processes {
        let cwd = p.cwd.as_str();
        if cwd.is_empty() || cwd == "/" || is_pseudo_fs_path(cwd) {
            continue;
        }
        *counts.entry(cwd).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(cwd, _)| cwd.to_string())
}

/// True for paths that can never be a project directory: kernel and
/// device pseudo-filesystems.
fn is_pseudo_fs_path(path: &str) -> bool {
    const ROOTS: &[&str] = &["/proc/", "/sys/", "/dev/"];
    ROOTS.iter().any(|r| path.starts_with(r))
}

fn project_name(dir: &str) -> String {
    dir.rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(dir)
        .to_string()
}

fn build_summary(
    agent: &AgentIdentity,
    project: Option<&ProjectInfo>,
    state: SessionState,
    process_count: usize,
) -> String {
    let where_part = match project {
        Some(p) => {
            let branch = p.branch.as_deref().unwrap_or("no branch");
            format!("{} ({})", p.name, branch)
        }
        None => "no project".to_string(),
    };
    format!(
        "{} [{}] on {} · {} · {} proc{}",
        agent.kind.as_str(),
        agent.confidence.as_str(),
        where_part,
        state.as_str(),
        process_count,
        if process_count == 1 { "" } else { "s" }
    )
}

/// One-line summary for a corroborated chat window (design §4.8):
/// `Pitwall Chat 001 · opencode · Work · running`.
///
/// A human aid, never identity — the full observed facts live in
/// [`TerminalSession::chat`], which is why the model is not repeated here.
fn build_chat_summary(chat: &ChatFacts, state: SessionState) -> String {
    format!(
        "{} {} · {} · {} · {}",
        crate::chat::CHAT_LABEL,
        chat.number_text(),
        chat.harness,
        chat.context_label,
        state.as_str()
    )
}

/// Collect one workspace snapshot. Single-shot and side-effect free
/// (observation only) — no loops, no writes, no network.
pub fn collect(platform: &dyn Platform) -> WorkspaceSnapshot {
    let raw = platform.processes();
    // Observation only: reads the chat lease directory, validates nothing,
    // unlinks nothing (see `Platform::chat_leases`). Read once for the whole
    // snapshot so every window is corroborated against the same observation.
    let leases = platform.chat_leases();
    let by_pid: HashMap<u32, &RawProcess> = raw.iter().map(|p| (p.pid, p)).collect();
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for p in &raw {
        children.entry(p.ppid).or_default().push(p.pid);
    }

    let boot = platform.boot_epoch();
    let hz = platform.clock_ticks_per_sec().max(1);
    let to_epoch = |ticks: i64| {
        if ticks < 0 || boot < 0 {
            -1
        } else {
            boot + ticks / hz
        }
    };

    let mut sessions = Vec::new();
    // Observer hygiene: the collector's own process is never part of the
    // observation. Without this, every invocation would force
    // `last_activity` to "now" in the invoking session, i.e. the observer
    // would always measure itself.
    let self_pid = std::process::id();
    for window in platform.windows() {
        // Descendant BFS from the window's root PID (root included, self excluded).
        let mut tree: Vec<u32> = Vec::new();
        let mut stack = vec![window.pid];
        let mut seen = std::collections::HashSet::new();
        while let Some(pid) = stack.pop() {
            if pid == self_pid || !seen.insert(pid) {
                continue;
            }
            tree.push(pid);
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        let mut processes: Vec<ProcessInfo> = tree
            .iter()
            .filter_map(|pid| by_pid.get(pid))
            .map(|r| ProcessInfo {
                pid: r.pid,
                ppid: r.ppid,
                name: r.name.clone(),
                command: r.command.clone(),
                exe_name: r.exe_name.clone(),
                cwd: r.cwd.clone(),
                state: ProcessState::from_code(r.state_code),
                started_at_epoch: to_epoch(r.starttime_ticks),
            })
            .collect();
        processes.sort_by_key(|p| p.pid);
        let process_count = processes.len();

        let project = project_dir_for(&processes).map(|raw_dir| {
            // P1: normalize before hashing or displaying, so `/x/`,
            // symlinked, and real paths share one project identity.
            let dir = ids::normalize_project_dir(&raw_dir);
            let git = platform.git_info(&dir);
            ProjectInfo {
                id: ids::project_id(&dir),
                name: project_name(&dir),
                dir,
                is_git_repo: git.is_repo,
                branch: git.branch,
                git_clean: git.clean,
            }
        });

        let mut role = role_for_class(&window.class, &window.initial_class);
        let agent = classify_agent(&processes, Some(&window), role);
        let state = derive_session_state(&processes);
        let last_activity_epoch = processes
            .iter()
            .filter(|p| p.pid != window.pid)
            .map(|p| p.started_at_epoch)
            .filter(|e| *e > 0)
            .max()
            .unwrap_or(-1);
        let id = match &project {
            Some(p) => ids::session_id(&p.id, &window.address, window.pid),
            None => ids::session_id(
                &ids::unknown_project_id(&window.address, window.pid),
                &window.address,
                window.pid,
            ),
        };
        // Chat is a post-BFS *upgrade*, computed against the full observed
        // tree (before the display cap below, so a leaseholder beyond the cap
        // still corroborates) and applied only when both signals hold. Every
        // fact derived above — project, agent kind, confidence, evidence,
        // state, activity, session id — is left exactly as the existing rules
        // produced it (17.3, 17.4, 23.3); only `role` and the one-line
        // `summary` change, and only for a proven chat.
        let chat = chat_facts_for(&window.title, &processes, &leases);
        let summary = match &chat {
            Some(facts) => {
                role = WindowRole::Chat;
                build_chat_summary(facts, state)
            }
            None => build_summary(&agent, project.as_ref(), state, process_count),
        };
        processes.truncate(MAX_SESSION_PROCESSES);
        let root_pid = window.pid;
        sessions.push(TerminalSession {
            id,
            window: Some(window),
            root_pid,
            role,
            project,
            agent,
            chat,
            state,
            process_count,
            processes,
            last_activity_epoch,
            last_activity_kind: LAST_ACTIVITY_KIND,
            summary,
        });
    }
    sessions.sort_by(|a, b| a.id.cmp(&b.id));

    WorkspaceSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        collected_at_epoch: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(-1),
        hostname: platform.hostname(),
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{
        ChatLease, GitInfo, InlineImage, Platform, RawProcess, TerminalSpec, WindowInfo,
    };

    struct MockPlatform {
        processes: Vec<RawProcess>,
        windows: Vec<WindowInfo>,
        repos: HashMap<String, GitInfo>,
    }

    impl Platform for MockPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            self.processes.clone()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.windows.clone()
        }
        fn git_info(&self, dir: &str) -> GitInfo {
            self.repos.get(dir).cloned().unwrap_or_default()
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
        fn launch_terminal(&self, _spec: &TerminalSpec<'_>) -> Result<(), String> {
            Ok(())
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            // Collector tests observe processes and windows only; lease
            // corroboration is exercised where it lives.
            Vec::new()
        }
        fn inline_image_capability(&self) -> InlineImage {
            // Fixed: a test must never probe a real terminal.
            InlineImage::None
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

    fn raw(
        pid: u32,
        ppid: u32,
        name: &str,
        cmd: &str,
        cwd: &str,
        state: char,
        ticks: i64,
    ) -> RawProcess {
        RawProcess {
            pid,
            ppid,
            name: name.to_string(),
            command: cmd.to_string(),
            exe_name: String::new(),
            cwd: cwd.to_string(),
            state_code: state,
            starttime_ticks: ticks,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn raw_exe(
        pid: u32,
        ppid: u32,
        name: &str,
        cmd: &str,
        exe: &str,
        cwd: &str,
        state: char,
        ticks: i64,
    ) -> RawProcess {
        RawProcess {
            pid,
            ppid,
            name: name.to_string(),
            command: cmd.to_string(),
            exe_name: exe.to_string(),
            cwd: cwd.to_string(),
            state_code: state,
            starttime_ticks: ticks,
        }
    }

    fn window(address: &str, class: &str, title: &str, pid: u32) -> WindowInfo {
        WindowInfo {
            address: address.to_string(),
            class: class.to_string(),
            initial_class: class.to_string(),
            title: title.to_string(),
            workspace: "1".to_string(),
            pid,
        }
    }

    fn agent_session_platform() -> MockPlatform {
        MockPlatform {
            processes: vec![
                raw(
                    100,
                    1,
                    "foot",
                    "/usr/bin/foot --app-id org.omarchy.agent opencode --auto",
                    "/",
                    'S',
                    100,
                ),
                raw(
                    101,
                    100,
                    "opencode",
                    "/home/u/.local/bin/opencode --auto",
                    "/home/u/Work",
                    'R',
                    200,
                ),
                raw(
                    102,
                    101,
                    "bash",
                    "/bin/bash -c somecmd",
                    "/home/u/Work",
                    'S',
                    300,
                ),
            ],
            windows: vec![window("0xabc", "org.omarchy.agent", "OC | my project", 100)],
            repos: HashMap::from([(
                "/home/u/Work".to_string(),
                GitInfo {
                    is_repo: false,
                    branch: None,
                    clean: None,
                },
            )]),
        }
    }

    #[test]
    fn collect_maps_agent_terminal_end_to_end() {
        let snap = collect(&agent_session_platform());
        assert_eq!(snap.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snap.hostname, "testbox");
        assert_eq!(snap.sessions.len(), 1);
        let s = &snap.sessions[0];
        assert_eq!(s.agent.kind, AgentKind::Opencode);
        assert_eq!(s.agent.confidence, Confidence::High);
        assert!(s.agent.evidence.len() >= 2);
        let proj = s.project.as_ref().expect("project from mode cwd");
        assert_eq!(proj.dir, "/home/u/Work");
        assert_eq!(proj.name, "Work");
        assert!(!proj.is_git_repo);
        assert!(proj.id.starts_with("proj_"));
        assert!(s.id.starts_with("sess_"));
        assert_eq!(s.state, SessionState::Running);
        assert_eq!(s.process_count, 3);
        assert_eq!(s.last_activity_kind, LAST_ACTIVITY_KIND);
        // latest descendant start: boot + 300/100
        assert_eq!(s.last_activity_epoch, 1_700_000_003);
        assert!(s.summary.contains("opencode"));
    }

    #[test]
    fn session_ids_are_stable_for_same_inputs() {
        let a = collect(&agent_session_platform());
        let b = collect(&agent_session_platform());
        assert_eq!(a.sessions[0].id, b.sessions[0].id);
        assert_eq!(
            a.sessions[0].project.as_ref().unwrap().id,
            b.sessions[0].project.as_ref().unwrap().id
        );
    }

    #[test]
    fn unknown_agent_when_no_evidence() {
        let plat = MockPlatform {
            processes: vec![raw(200, 1, "foot", "/usr/bin/foot", "/", 'S', 10)],
            windows: vec![window("0x1", "foot", "user@host:~", 200)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        // Honest Unknown: terminal examined, nothing found.
        assert_eq!(s.role, WindowRole::Terminal);
        assert_eq!(s.agent.kind, AgentKind::Unknown);
        assert_eq!(s.agent.confidence, Confidence::Low);
        assert_eq!(s.agent.evidence, vec!["terminal context only".to_string()]);
        assert!(s.project.is_none(), "root cwd must not become a project");
        assert_eq!(s.state, SessionState::Sleeping);
        assert_eq!(s.last_activity_epoch, -1);
    }

    #[test]
    fn app_window_skips_agent_inference() {
        let plat = MockPlatform {
            processes: vec![raw_exe(
                600,
                1,
                "chromium",
                "/usr/lib/chromium/chromium",
                "chromium",
                "/",
                'S',
                10,
            )],
            windows: vec![window("0x9", "chromium", "Some Page", 600)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.role, WindowRole::App);
        assert_eq!(s.agent.kind, AgentKind::Unknown);
        assert_eq!(s.agent.confidence, Confidence::Unknown);
        assert!(s.agent.evidence.is_empty());
    }

    #[test]
    fn pseudo_fs_cwds_are_never_projects() {
        // A browser helper reporting cwd under /proc (observed live as
        // /proc/<pid>/fdinfo) must not mint a project identity.
        let plat = MockPlatform {
            processes: vec![
                raw(
                    900,
                    1,
                    "chromium",
                    "/usr/lib/chromium/chromium",
                    "/",
                    'S',
                    1,
                ),
                raw(
                    901,
                    900,
                    "helper",
                    "/usr/lib/chromium/helper",
                    "/proc/726845/fdinfo",
                    'S',
                    2,
                ),
                raw(
                    902,
                    900,
                    "gpu",
                    "/usr/lib/chromium/gpu",
                    "/sys/fs/cgroup",
                    'S',
                    3,
                ),
            ],
            windows: vec![window("0x9", "chromium", "Page", 900)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        assert!(
            snap.sessions[0].project.is_none(),
            "pseudo-fs cwd must not projectize"
        );
        assert!(is_pseudo_fs_path("/proc/1/fdinfo"));
        assert!(is_pseudo_fs_path("/sys/kernel"));
        assert!(is_pseudo_fs_path("/dev/pts/0"));
        assert!(!is_pseudo_fs_path("/home/u/Work"));
        assert!(!is_pseudo_fs_path("/procstuff/x"));
    }

    #[test]
    fn role_classification_covers_known_terminals() {
        assert_eq!(role_for_class("foot", "foot"), WindowRole::Terminal);
        assert_eq!(
            role_for_class("org.omarchy.agent", "foot"),
            WindowRole::Terminal
        );
        assert_eq!(role_for_class("kitty", ""), WindowRole::Terminal);
        assert_eq!(role_for_class("Alacritty", ""), WindowRole::Terminal);
        assert_eq!(role_for_class("chromium", ""), WindowRole::App);
        assert_eq!(role_for_class("Code", "code"), WindowRole::App);
        assert_eq!(role_for_class("", ""), WindowRole::Unknown);
    }

    #[test]
    fn exe_basename_corroborates_agent_identity() {
        // argv[0] is a wrapper/shim; exe tells the truth → Medium alone,
        // High with the agent window class.
        let plat = MockPlatform {
            processes: vec![
                raw(
                    700,
                    1,
                    "foot",
                    "/usr/bin/foot --app-id org.omarchy.agent",
                    "/",
                    'S',
                    5,
                ),
                raw_exe(
                    701,
                    700,
                    "node",
                    "/usr/bin/node /opt/agent-shim/run.js",
                    "opencode",
                    "/home/u/Work",
                    'R',
                    6,
                ),
            ],
            windows: vec![window("0x7", "org.omarchy.agent", "OC | shimmed", 700)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.agent.kind, AgentKind::Opencode);
        assert_eq!(s.agent.confidence, Confidence::High);
        assert!(s
            .agent
            .evidence
            .iter()
            .any(|e| e.starts_with("exe:opencode")));
    }

    #[test]
    fn exe_only_match_is_medium() {
        let plat = MockPlatform {
            processes: vec![
                raw(800, 1, "foot", "/usr/bin/foot", "/", 'S', 5),
                raw_exe(
                    801,
                    800,
                    "runner",
                    "/usr/bin/runner",
                    "claude",
                    "/home/u/Work",
                    'S',
                    6,
                ),
            ],
            windows: vec![window("0x8", "foot", "plain shell", 800)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.agent.kind, AgentKind::ClaudeCode);
        assert_eq!(s.agent.confidence, Confidence::Medium);
    }

    #[test]
    fn title_only_hint_is_low_confidence() {
        let plat = MockPlatform {
            processes: vec![raw(
                300,
                1,
                "foot",
                "/usr/bin/foot --app-id org.omarchy.agent",
                "/",
                'S',
                5,
            )],
            windows: vec![window("0x2", "foot", "OC | renamed binary?", 300)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.agent.kind, AgentKind::Opencode);
        assert_eq!(s.agent.confidence, Confidence::Low);
    }

    #[test]
    fn stopped_process_marks_session_stopped() {
        let plat = MockPlatform {
            processes: vec![
                raw(400, 1, "bash", "/bin/bash", "/home/u/Work", 'S', 1),
                raw(401, 400, "vim", "vim file", "/home/u/Work", 'T', 2),
            ],
            windows: vec![window("0x3", "foot", "t", 400)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        assert_eq!(snap.sessions[0].state, SessionState::Stopped);
    }

    fn proc_with_state(pid: u32, state: ProcessState) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid: 1,
            name: "x".to_string(),
            command: "x".to_string(),
            exe_name: String::new(),
            cwd: "/home/u/Work".to_string(),
            state,
            started_at_epoch: 1,
        }
    }

    #[test]
    fn state_severity_is_order_independent() {
        // Every permutation of a mixed tree must agree: Stopped wins.
        let mixed = [
            ProcessState::Running,
            ProcessState::Sleeping,
            ProcessState::Stopped,
            ProcessState::Zombie,
            ProcessState::Unknown,
        ];
        let mut orders = vec![
            vec![0, 1, 2, 3, 4],
            vec![4, 3, 2, 1, 0],
            vec![2, 0, 4, 1, 3],
            vec![1, 3, 0, 4, 2],
        ];
        // Rotate through all cyclic shifts too.
        for shift in 0..5 {
            orders.push((0..5).map(|i| (i + shift) % 5).collect());
        }
        for order in orders {
            let procs: Vec<ProcessInfo> = order
                .iter()
                .enumerate()
                .map(|(n, &m)| proc_with_state(n as u32, mixed[m]))
                .collect();
            assert_eq!(
                derive_session_state(&procs),
                SessionState::Stopped,
                "order {order:?}"
            );
        }
    }

    #[test]
    fn stopped_beats_running_regardless_of_pid_order() {
        // Real-world case: low-pid Running agent + high-pid STOPPED job.
        let procs = vec![
            proc_with_state(100, ProcessState::Running),
            proc_with_state(99999, ProcessState::Stopped),
        ];
        assert_eq!(derive_session_state(&procs), SessionState::Stopped);
        // And the reverse pid assignment.
        let procs = vec![
            proc_with_state(100, ProcessState::Stopped),
            proc_with_state(99999, ProcessState::Running),
        ];
        assert_eq!(derive_session_state(&procs), SessionState::Stopped);
    }

    #[test]
    fn state_precedence_chain() {
        use ProcessState as P;
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Running)]),
            SessionState::Running
        );
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::DiskSleep)]),
            SessionState::Running
        );
        assert_eq!(
            derive_session_state(&[
                proc_with_state(1, P::Running),
                proc_with_state(2, P::Sleeping),
            ]),
            SessionState::Running
        );
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Sleeping), proc_with_state(2, P::Idle),]),
            SessionState::Sleeping
        );
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Sleeping)]),
            SessionState::Sleeping
        );
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Unknown)]),
            SessionState::Unknown
        );
        // Zombies/dead alone are Unknown, and never outrank the living.
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Zombie), proc_with_state(2, P::Dead)]),
            SessionState::Unknown
        );
        assert_eq!(
            derive_session_state(&[proc_with_state(1, P::Zombie), proc_with_state(2, P::Idle)]),
            SessionState::Sleeping
        );
        assert_eq!(derive_session_state(&[]), SessionState::Unknown);
    }

    #[test]
    fn git_project_carries_branch_and_clean() {
        let plat = MockPlatform {
            processes: vec![raw(500, 1, "bash", "/bin/bash", "/home/u/pitwall", 'S', 1)],
            windows: vec![window("0x4", "foot", "t", 500)],
            repos: HashMap::from([(
                "/home/u/pitwall".to_string(),
                GitInfo {
                    is_repo: true,
                    branch: Some("main".into()),
                    clean: Some(true),
                },
            )]),
        };
        let snap = collect(&plat);
        let p = snap.sessions[0].project.as_ref().unwrap();
        assert!(p.is_git_repo);
        assert_eq!(p.branch.as_deref(), Some("main"));
        assert_eq!(p.git_clean, Some(true));
    }

    #[test]
    fn aggregate_facts_use_full_tree_before_display_cap() {
        let mut plat = agent_session_platform();
        plat.windows[0].class = "foot".into();
        plat.windows[0].initial_class = "foot".into();
        plat.windows[0].title.clear();
        plat.processes = (100..140)
            .map(|pid| {
                raw(
                    pid,
                    if pid == 100 { 1 } else { 100 },
                    "sh",
                    "sh",
                    "/",
                    'S',
                    100,
                )
            })
            .collect();
        plat.processes.push(raw(
            999,
            100,
            "opencode",
            "opencode",
            "/home/u/Work",
            'T',
            9000,
        ));
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.processes.len(), MAX_SESSION_PROCESSES);
        assert_eq!(s.process_count, 41);
        assert_eq!(s.state, SessionState::Stopped);
        assert_eq!(s.agent.kind, AgentKind::Opencode);
        assert_eq!(s.last_activity_epoch, 1_700_000_090);
        assert_eq!(s.project.as_ref().unwrap().dir, "/home/u/Work");
    }

    #[test]
    fn missing_window_root_is_not_counted_as_observed_process() {
        let mut plat = agent_session_platform();
        plat.processes.clear();
        let snap = collect(&plat);
        assert_eq!(snap.sessions[0].process_count, 0);
        assert_eq!(snap.sessions[0].root_pid, 100);
        assert_eq!(snap.sessions[0].last_activity_epoch, -1);
    }

    #[test]
    fn app_window_ignores_agent_named_descendants_and_titles() {
        let mut plat = agent_session_platform();
        plat.windows[0].class = "chromium".into();
        plat.windows[0].initial_class = "chromium".into();
        let snap = collect(&plat);
        assert_eq!(snap.sessions[0].agent.kind, AgentKind::Unknown);
        assert_eq!(snap.sessions[0].agent.confidence, Confidence::Unknown);
        assert!(snap.sessions[0].agent.evidence.is_empty());
    }

    #[test]
    fn collector_excludes_its_own_process() {
        let self_pid = std::process::id();
        let plat = MockPlatform {
            processes: vec![
                raw(400, 1, "bash", "/bin/bash", "/home/u/Work", 'S', 1),
                // The observer, mid-tree: must not appear in output.
                raw(
                    self_pid,
                    400,
                    "pitwall",
                    "pitwall status --json",
                    "/home/u/Work",
                    'R',
                    99999,
                ),
                raw(
                    self_pid + 1,
                    self_pid,
                    "git",
                    "git status",
                    "/home/u/Work",
                    'R',
                    100000,
                ),
            ],
            windows: vec![window("0x3", "foot", "t", 400)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert!(s.processes.iter().all(|p| p.pid != self_pid));
        assert_eq!(s.process_count, 1);
        // No observed descendant remains; the window root is not child activity.
        assert_eq!(s.last_activity_epoch, -1);
    }

    // --- Chat_Role corroboration (task 11.1, design §4.8) ----------------
    //
    // The tests above are the regression proof that Chat changed nothing:
    // they are untouched, and so is the `MockPlatform` they build. Chat
    // needs one extra observation (the lease list), so it is supplied by a
    // wrapper that delegates every other call, rather than by widening the
    // mock every existing test constructs.

    struct LeasedPlatform {
        inner: MockPlatform,
        leases: Vec<ChatLease>,
    }

    impl Platform for LeasedPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            self.inner.processes()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.inner.windows()
        }
        fn git_info(&self, dir: &str) -> GitInfo {
            self.inner.git_info(dir)
        }
        fn boot_epoch(&self) -> i64 {
            self.inner.boot_epoch()
        }
        fn clock_ticks_per_sec(&self) -> i64 {
            self.inner.clock_ticks_per_sec()
        }
        fn hostname(&self) -> String {
            self.inner.hostname()
        }
        fn launch_terminal(&self, spec: &TerminalSpec<'_>) -> Result<(), String> {
            self.inner.launch_terminal(spec)
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            self.leases.clone()
        }
        fn inline_image_capability(&self) -> InlineImage {
            self.inner.inline_image_capability()
        }
        fn focus_window_address(&self, address: &str) -> Result<(), String> {
            self.inner.focus_window_address(address)
        }
        fn process_io(&self, pid: u32) -> Option<crate::platform::IoCounters> {
            self.inner.process_io(pid)
        }
        fn terminal_text(&self, pid: u32, class: &str) -> crate::platform::TerminalText {
            self.inner.terminal_text(pid, class)
        }
    }

    /// A Chat_Title_Grammar title, built the way a chat builds it.
    fn chat_title(number: u16, harness: &str, model_label: &str, label: &str) -> String {
        format!("Pitwall Chat {number:03} · {harness} · {model_label} · {label}")
    }

    /// The tree of a chat window: terminal root plus the `pitwall chat`
    /// process it hosts.
    fn chat_tree(root: u32, chat_pid: u32) -> Vec<ProcessInfo> {
        vec![
            ProcessInfo {
                pid: root,
                ppid: 1,
                name: "foot".to_string(),
                command: "/usr/bin/foot".to_string(),
                exe_name: "foot".to_string(),
                cwd: "/".to_string(),
                state: ProcessState::Sleeping,
                started_at_epoch: 1_700_000_010,
            },
            ProcessInfo {
                pid: chat_pid,
                ppid: root,
                name: "pitwall".to_string(),
                command: "/usr/bin/pitwall chat".to_string(),
                exe_name: "pitwall".to_string(),
                cwd: "/home/u/Work".to_string(),
                state: ProcessState::Running,
                started_at_epoch: 1_700_000_042,
            },
        ]
    }

    #[test]
    fn chat_facts_need_both_signals() {
        let title = chat_title(1, "opencode", "agent default", "Work");
        let tree = chat_tree(100, 101);
        let leases = vec![ChatLease {
            number: 1,
            pid: 101,
        }];
        let facts = chat_facts_for(&title, &tree, &leases).expect("both signals present");
        assert_eq!(facts.number, 1);
        assert_eq!(facts.number_text(), "001");
        assert_eq!(facts.harness, "opencode");
        assert_eq!(facts.model, "", "agent default parses to an empty model");
        assert_eq!(facts.context_label, "Work");
        // Not observable from the title, so not invented (17.7).
        assert_eq!(facts.context_session_id, None);
        // Taken from the proven process, not from the window root.
        assert_eq!(facts.started_at_epoch, 1_700_000_042);
    }

    #[test]
    fn a_spoofed_title_alone_proves_nothing() {
        // Signal A only: the exact reserved title, echoed by a plain shell.
        let title = chat_title(1, "opencode", "agent default", "Work");
        let tree = vec![ProcessInfo {
            pid: 200,
            ppid: 1,
            name: "bash".to_string(),
            command: "/bin/bash".to_string(),
            exe_name: "bash".to_string(),
            cwd: "/home/u/Work".to_string(),
            state: ProcessState::Sleeping,
            started_at_epoch: 1_700_000_001,
        }];
        assert!(chat_facts_for(&title, &tree, &[]).is_none());
        // Even with a real chat leasing 001 elsewhere on the machine.
        let leases = vec![ChatLease {
            number: 1,
            pid: 999,
        }];
        assert!(chat_facts_for(&title, &tree, &leases).is_none());
    }

    #[test]
    fn lease_pid_outside_this_window_tree_is_not_this_window() {
        let title = chat_title(1, "opencode", "agent default", "Work");
        // A live chat exists (pid 101) but this window's tree does not hold it.
        let other_window_tree = chat_tree(300, 301);
        let leases = vec![ChatLease {
            number: 1,
            pid: 101,
        }];
        assert!(chat_facts_for(&title, &other_window_tree, &leases).is_none());
        // And a lease for a *different* number never corroborates this title.
        let tree = chat_tree(100, 101);
        let wrong_number = vec![ChatLease {
            number: 2,
            pid: 101,
        }];
        assert!(chat_facts_for(&title, &tree, &wrong_number).is_none());
    }

    #[test]
    fn leaseholder_in_tree_must_actually_be_pitwall_chat() {
        let title = chat_title(1, "opencode", "agent default", "Work");
        let mut tree = chat_tree(100, 101);
        // Same pid, different program: a stale lease pointing at a recycled
        // pid must not hand a window someone else's identity.
        tree[1].command = "/usr/bin/pitwall status --json".to_string();
        let leases = vec![ChatLease {
            number: 1,
            pid: 101,
        }];
        assert!(chat_facts_for(&title, &tree, &leases).is_none());
        tree[1].command = "/usr/bin/htop".to_string();
        tree[1].exe_name = "htop".to_string();
        assert!(chat_facts_for(&title, &tree, &leases).is_none());
    }

    #[test]
    fn a_chat_process_without_a_lease_is_not_upgraded() {
        // Signal B is lease-anchored on purpose: the lease is what binds a
        // *number* to a pid, and the number is what the title claims.
        let title = chat_title(1, "opencode", "agent default", "Work");
        let tree = chat_tree(100, 101);
        assert!(chat_facts_for(&title, &tree, &[]).is_none());
    }

    #[test]
    fn malformed_titles_never_reach_signal_b() {
        let tree = chat_tree(100, 101);
        let leases = vec![ChatLease {
            number: 1,
            pid: 101,
        }];
        for title in [
            "",
            "user@host:~",
            "OC | my project",
            "Pitwall Chat 1 · opencode · agent default · Work",
            "Pitwall Chat 001 - opencode - agent default - Work",
            "Pitwall Chat 000 · opencode · agent default · Work",
            "Pitwall Chat 001 · notaharness · agent default · Work",
            "Pitwall Chat 001 · opencode · agent default",
        ] {
            assert!(
                chat_facts_for(title, &tree, &leases).is_none(),
                "title {title:?} must not classify as chat"
            );
        }
    }

    #[test]
    fn collect_upgrades_a_corroborated_chat_window() {
        let chat_pid = 101;
        let plat = LeasedPlatform {
            inner: MockPlatform {
                processes: vec![
                    raw_exe(100, 1, "foot", "/usr/bin/foot", "foot", "/", 'S', 10),
                    raw_exe(
                        chat_pid,
                        100,
                        "pitwall",
                        "/usr/bin/pitwall chat",
                        "pitwall",
                        "/home/u/Work",
                        'R',
                        4200,
                    ),
                ],
                windows: vec![window(
                    "0xc1",
                    "foot",
                    &chat_title(1, "opencode", "agent default", "Work"),
                    100,
                )],
                repos: HashMap::new(),
            },
            leases: vec![ChatLease {
                number: 1,
                pid: chat_pid,
            }],
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert_eq!(s.role, WindowRole::Chat);
        assert_eq!(WindowRole::Chat.as_str(), "chat");
        let facts = s.chat.as_ref().expect("chat facts recorded");
        assert_eq!(facts.number_text(), "001");
        assert_eq!(facts.harness, "opencode");
        assert_eq!(facts.context_label, "Work");
        assert_eq!(facts.started_at_epoch, 1_700_000_042);
        assert_eq!(s.summary, "Pitwall Chat 001 · opencode · Work · running");
        // Agent inference is untouched: a chat tree carries no agent
        // evidence, so the honest terminal-context Unknown still stands.
        assert_eq!(s.agent.kind, AgentKind::Unknown);
        assert_eq!(s.agent.confidence, Confidence::Low);
        assert_eq!(s.agent.evidence, vec!["terminal context only".to_string()]);
        // And everything else is derived exactly as before.
        assert_eq!(s.project.as_ref().unwrap().dir, "/home/u/Work");
        assert_eq!(s.state, SessionState::Running);
        assert_eq!(s.process_count, 2);
    }

    #[test]
    fn collect_keeps_existing_roles_when_corroboration_fails() {
        // A plain terminal, an OpenCode agent session, and a tmux-style
        // title — with a chat lease present on the machine. None of them may
        // change role, agent kind, confidence, evidence or summary.
        let plat = MockPlatform {
            processes: vec![
                raw(200, 1, "bash", "/bin/bash", "/home/u/Work", 'S', 10),
                raw(100, 1, "foot", "/usr/bin/foot", "/", 'S', 10),
                raw(
                    101,
                    100,
                    "opencode",
                    "/home/u/.local/bin/opencode --auto",
                    "/home/u/Work",
                    'R',
                    20,
                ),
                raw(400, 1, "zsh", "/bin/zsh", "/home/u/Work", 'S', 10),
            ],
            windows: vec![
                window("0x1", "foot", "user@host:~", 200),
                window("0x2", "org.omarchy.agent", "OC | my project", 100),
                window("0x3", "foot", "[0] 0:zsh*  \"host\" 12:00", 400),
            ],
            repos: HashMap::new(),
        };
        let baseline = collect(&plat);
        let leased = LeasedPlatform {
            inner: plat,
            leases: vec![ChatLease {
                number: 1,
                pid: 101,
            }],
        };
        let with_leases = collect(&leased);
        assert_eq!(baseline.sessions.len(), 3);
        for (before, after) in baseline.sessions.iter().zip(with_leases.sessions.iter()) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.role, after.role);
            assert_ne!(after.role, WindowRole::Chat);
            assert!(after.chat.is_none());
            assert_eq!(before.agent, after.agent);
            assert_eq!(before.summary, after.summary);
        }
        let by_address = |addr: &str| {
            with_leases
                .sessions
                .iter()
                .find(|s| s.window.as_ref().unwrap().address == addr)
                .unwrap()
                .clone()
        };
        let shell = by_address("0x1");
        assert_eq!(shell.role, WindowRole::Terminal);
        assert_eq!(shell.agent.confidence, Confidence::Low);
        let agent = by_address("0x2");
        assert_eq!(agent.role, WindowRole::Terminal);
        assert_eq!(agent.agent.kind, AgentKind::Opencode);
        assert_eq!(agent.agent.confidence, Confidence::High);
        let tmux = by_address("0x3");
        assert_eq!(tmux.role, WindowRole::Terminal);
        assert_eq!(tmux.agent.kind, AgentKind::Unknown);
    }

    #[test]
    fn pitwall_chat_argv_rule_is_shared_with_the_lease_validator() {
        assert!(is_pitwall_chat_argv("/usr/bin/pitwall chat", ""));
        assert!(is_pitwall_chat_argv("pitwall chat --session sess_x", ""));
        // Shim in argv[0], truth in exe.
        assert!(is_pitwall_chat_argv("/opt/shim/run chat", "pitwall"));
        assert!(!is_pitwall_chat_argv("/usr/bin/pitwall status", ""));
        assert!(!is_pitwall_chat_argv("/usr/bin/pitwallx chat", ""));
        assert!(!is_pitwall_chat_argv("/usr/bin/htop", "htop"));
        assert!(!is_pitwall_chat_argv("", ""));
        // The one rule, asked through the other caller.
        let procs = vec![RawProcess {
            pid: 101,
            ppid: 100,
            name: "pitwall".to_string(),
            command: "/usr/bin/pitwall chat".to_string(),
            exe_name: "pitwall".to_string(),
            cwd: "/home/u/Work".to_string(),
            state_code: 'R',
            starttime_ticks: 4200,
        }];
        assert!(crate::context::is_live_pitwall_chat(101, &procs));
        assert!(!crate::context::is_live_pitwall_chat(999, &procs));
    }

    #[test]
    fn process_state_codes_map_correctly() {
        assert_eq!(ProcessState::from_code('R'), ProcessState::Running);
        assert_eq!(ProcessState::from_code('D'), ProcessState::DiskSleep);
        assert_eq!(ProcessState::from_code('t'), ProcessState::Stopped);
        assert_eq!(ProcessState::from_code('Z'), ProcessState::Zombie);
        assert_eq!(ProcessState::from_code('I'), ProcessState::Idle);
        assert_eq!(ProcessState::from_code('?'), ProcessState::Unknown);
    }

    // -----------------------------------------------------------------
    // Property test (M8 task 11.3). `proptest` is a dev-dependency pinned
    // `=1.5.0`; Property 12 is exactly ONE test at 100+ cases.
    //
    // The example tests above pin individual shapes; this generalises over
    // the whole generated cross-product of
    //
    //   (title parses / does not) × (a lease names this number / no lease /
    //   a lease names a different number / both) × (the leaseholder pid is
    //   inside this window's tree / is not) × (the leaseholder's argv is
    //   `pitwall chat` / is not)
    //
    // and asserts the Chat_Role appears in exactly the conjunction case.
    //
    // "Changes nothing else" is expressed the way
    // `collect_keeps_existing_roles_when_corroboration_fails` expresses it,
    // generalised: every case runs `collect` twice over identical
    // observations, once through the bare `MockPlatform` (whose
    // `chat_leases` observes nothing, so `chat_facts_for` provably cannot
    // fire for any window — that run *is* the pre-M8 computation) and once
    // through `LeasedPlatform` carrying the generated leases. Nothing is
    // re-implemented: the oracle is the pair of generation choices.
    // -----------------------------------------------------------------

    use proptest::prelude::*;

    /// Window classes to vary the observation across.
    ///
    /// The first seven are the classes [`role_for_class`] already
    /// recognises as terminals; `st` is an emulator it does not know (so the
    /// window classifies as `App`, where agent inference is skipped
    /// entirely) and `""` is the empty class (`Unknown`). Varying the chat
    /// decision across all nine is the checkable form of "the decision reads
    /// no terminal-emulator-specific window property" (Requirement 28.12):
    /// a corroborated chat must be recognised identically in every one of
    /// them, including the two Pitwall has never heard of.
    ///
    /// These names appear here for the same reason `FORBIDDEN_TOKENS` does
    /// in `src/platform/linux.rs`: to prove independence from them. They are
    /// `#[cfg(test)]` and Pitwall requires, resolves and branches on none of
    /// them.
    const WINDOW_CLASSES: &[&str] = &[
        "foot",
        "kitty",
        "alacritty",
        "ghostty",
        "wezterm",
        "xterm",
        "org.omarchy.agent",
        "st",
        "",
    ];

    /// Context labels a real chat could carry: non-empty, no control
    /// character, no title separator, within [`crate::chat::MAX_CONTEXT_LABEL_CHARS`].
    const CONTEXT_LABELS: &[&str] = &["Work", "Workspace", "pitwall", "my project", "a"];

    /// The model field as it appears *in the title*: the agent-default
    /// literal (which parses back to an empty model) or a
    /// [`crate::summary::valid_model`] id.
    const MODEL_FIELDS: &[&str] = &[
        crate::chat::AGENT_DEFAULT_LABEL,
        "prov/model",
        "openrouter/anth-3.5",
        "a/b",
    ];

    /// Address of the window whose title and corroboration are generated.
    const GENERATED_ADDRESS: &str = "0xg1";

    /// A title that is deliberately *not* a Chat_Title_Grammar title, one
    /// near miss per shape. Each is rejected by a different clause of
    /// [`crate::chat::parse_title`], and the test proves that claim rather
    /// than assuming it.
    fn malformed_title(
        shape: usize,
        number: u16,
        harness: &str,
        model_label: &str,
        label: &str,
    ) -> String {
        match shape {
            // Four digits instead of three (never three for 1..=999).
            1 => format!("Pitwall Chat {number:04} · {harness} · {model_label} · {label}"),
            // A hyphen separator: the title then has one field, not four.
            2 => format!("Pitwall Chat {number:03} - {harness} - {model_label} - {label}"),
            // A harness that is not a `crate::agents::KNOWN` id.
            3 => chat_title(number, "notaharness", model_label, label),
            // An empty context label.
            4 => chat_title(number, harness, model_label, ""),
            // The reserved literal, mis-cased.
            5 => format!("Pitwall chat {number:03} · {harness} · {model_label} · {label}"),
            // An ordinary shell title.
            _ => "user@host:~".to_string(),
        }
    }

    /// The observation every case is built from: one window whose title and
    /// corroboration vary, plus two control windows that must never change.
    ///
    /// Pid 101 is the generated window's candidate chat process and pid 401
    /// the control agent window's — the same argv choice drives both, so
    /// "the leaseholder is not in this window's tree" can be exercised with
    /// a lease naming a *genuine* `pitwall chat` process that simply lives
    /// somewhere else, which is the case a weaker mock would miss.
    fn prop12_platform(class: &str, title: &str, leaseholder_is_chat: bool) -> MockPlatform {
        let candidate = if leaseholder_is_chat {
            "/usr/bin/pitwall chat"
        } else {
            "/usr/bin/pitwall status --json"
        };
        MockPlatform {
            processes: vec![
                raw_exe(100, 1, "term", "/usr/bin/term", "term", "/", 'S', 1000),
                raw_exe(
                    101,
                    100,
                    "pitwall",
                    candidate,
                    "pitwall",
                    "/home/u/Work",
                    'R',
                    4200,
                ),
                raw_exe(200, 1, "bash", "/bin/bash", "bash", "/home/u/Shell", 'S', 1000),
                raw_exe(400, 1, "term", "/usr/bin/term", "term", "/", 'S', 1000),
                raw_exe(
                    401,
                    400,
                    "pitwall",
                    candidate,
                    "pitwall",
                    "/home/u/Other",
                    'R',
                    4200,
                ),
                raw_exe(
                    402,
                    400,
                    "opencode",
                    "/home/u/.local/bin/opencode --auto",
                    "opencode",
                    "/home/u/Other",
                    'R',
                    2000,
                ),
            ],
            windows: vec![
                window(GENERATED_ADDRESS, class, title, 100),
                window("0xc1", "foot", "user@host:~", 200),
                window("0xc2", "org.omarchy.agent", "OC | my project", 400),
            ],
            repos: HashMap::new(),
        }
    }

    fn is_generated(session: &TerminalSession) -> bool {
        session
            .window
            .as_ref()
            .is_some_and(|w| w.address == GENERATED_ADDRESS)
    }

    proptest! {
        // 256 cases, comfortably above the 100 floor: the decision-relevant
        // cross-product is 7 title shapes × 4 lease shapes × 2 tree
        // placements × 2 argv shapes = 112 combinations, and each case
        // additionally re-runs the whole observation once per window class.
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        // **Validates: Requirements 16.5, 16.9, 17.1, 17.2, 17.3, 17.4, 17.7, 19.7, 23.3, 28.12**
        // Feature: pitwall-chat-and-brief-ticker, Property 12: Chat identity requires both signals and changes nothing else — For any observed window, the Collector assigns the Chat_Role if and only if (a) the window title matches the Chat_Title_Grammar with number `N`, and (b) a live lease for `N` names a `pitwall chat` process — the Chat_Process_Identity — present in that window's process tree; the decision reads no terminal-emulator-specific window property; for every window where that conjunction fails, the role, agent kind, confidence and evidence equal the pre-M8 computation; and every chat entry the writer emits corresponds to such an observed window.
        #[test]
        fn prop12_chat_identity_requires_both_signals_and_changes_nothing_else(
            title_shape in 0usize..7,
            number in 1u16..=999u16,
            harness_ix in 0usize..crate::agents::KNOWN.len(),
            model_ix in 0usize..MODEL_FIELDS.len(),
            label_ix in 0usize..CONTEXT_LABELS.len(),
            lease_shape in 0usize..4,
            lease_in_tree in any::<bool>(),
            leaseholder_is_chat in any::<bool>(),
        ) {
            let harness = crate::agents::KNOWN[harness_ix].id;
            let model_field = MODEL_FIELDS[model_ix];
            let label = CONTEXT_LABELS[label_ix];

            // ---- signal A: the title, well-formed or a near miss ---------
            let title = if title_shape == 0 {
                chat_title(number, harness, model_field, label)
            } else {
                malformed_title(title_shape, number, harness, model_field, label)
            };
            let title_parses = title_shape == 0;
            // The generator states which shape it built; this line proves it,
            // so a later grammar change cannot silently turn a "malformed"
            // case into a well-formed one and weaken the property.
            prop_assert_eq!(
                crate::chat::parse_title(&title).is_some(),
                title_parses,
                "title {:?}",
                title
            );

            // ---- signal B: the lease, its number, its pid, its argv ------
            let other_number = if number == crate::chat::MAX_CHAT_NUMBER {
                crate::chat::MIN_CHAT_NUMBER
            } else {
                number + 1
            };
            // In tree: the generated window's own candidate. Out of tree: the
            // control agent window's candidate — a real process, wrong window.
            let holder_pid = if lease_in_tree { 101 } else { 401 };
            let leases: Vec<ChatLease> = match lease_shape {
                0 => Vec::new(),
                1 => vec![ChatLease { number, pid: holder_pid }],
                2 => vec![ChatLease { number: other_number, pid: holder_pid }],
                _ => vec![
                    ChatLease { number, pid: holder_pid },
                    ChatLease { number: other_number, pid: 401 },
                ],
            };
            let lease_names_this_number = matches!(lease_shape, 1 | 3);

            // The conjunction, stated once, from the generation choices only.
            let expect_chat =
                title_parses && lease_names_this_number && lease_in_tree && leaseholder_is_chat;

            let expected_model = if model_field == crate::chat::AGENT_DEFAULT_LABEL {
                ""
            } else {
                model_field
            };
            let expected_number_text = format!("{number:03}");
            let expected_summary = format!(
                "{} {number:03} · {harness} · {label} · running",
                crate::chat::CHAT_LABEL
            );

            let mut decisions: Vec<Option<ChatFacts>> = Vec::new();
            for class in WINDOW_CLASSES {
                // The pre-M8 computation: identical observation, no leases.
                let baseline = collect(&prop12_platform(class, &title, leaseholder_is_chat));
                let leased = collect(&LeasedPlatform {
                    inner: prop12_platform(class, &title, leaseholder_is_chat),
                    leases: leases.clone(),
                });
                prop_assert_eq!(baseline.sessions.len(), 3);
                prop_assert_eq!(leased.sessions.len(), 3);
                prop_assert!(
                    baseline
                        .sessions
                        .iter()
                        .all(|s| s.chat.is_none() && s.role != WindowRole::Chat),
                    "the no-lease run must be chat-free by construction"
                );

                // Sessions are sorted by id and no id depends on a lease, so
                // zipping pairs the same window with itself.
                for (before, after) in baseline.sessions.iter().zip(leased.sessions.iter()) {
                    prop_assert_eq!(&before.id, &after.id);
                    // Everything the pre-M8 rules derived is untouched for
                    // *every* window, chat or not: only `role` and the
                    // one-line `summary` may move, and only for a proven chat
                    // (Requirements 17.3, 17.4, 23.3).
                    prop_assert_eq!(&before.agent, &after.agent);
                    prop_assert_eq!(&before.project, &after.project);
                    prop_assert_eq!(before.state, after.state);
                    prop_assert_eq!(before.process_count, after.process_count);
                    prop_assert_eq!(before.last_activity_epoch, after.last_activity_epoch);
                    prop_assert_eq!(&before.processes, &after.processes);
                    prop_assert_eq!(before.root_pid, after.root_pid);

                    if is_generated(after) && expect_chat {
                        // The conjunction holds: exactly this window is
                        // upgraded, and every recorded fact comes from the
                        // title plus the proven process.
                        prop_assert_eq!(after.role, WindowRole::Chat);
                        let facts = after.chat.as_ref().expect("chat facts recorded");
                        prop_assert_eq!(facts.number, number);
                        let number_text = facts.number_text();
                        prop_assert_eq!(number_text.as_str(), expected_number_text.as_str());
                        prop_assert_eq!(facts.harness.as_str(), harness);
                        prop_assert_eq!(facts.model.as_str(), expected_model);
                        prop_assert_eq!(facts.context_label.as_str(), label);
                        // Not observable from a title, so never invented (17.7).
                        prop_assert!(facts.context_session_id.is_none());
                        // The *proven process*' start time, not the window root's.
                        prop_assert_eq!(facts.started_at_epoch, 1_700_000_042);
                        prop_assert_eq!(after.summary.as_str(), expected_summary.as_str());
                    } else {
                        // The conjunction fails: role, agent kind, confidence
                        // and evidence are exactly the pre-M8 values.
                        prop_assert_eq!(after.role, before.role);
                        prop_assert_ne!(after.role, WindowRole::Chat);
                        prop_assert!(after.chat.is_none());
                        prop_assert_eq!(&after.summary, &before.summary);
                    }
                }

                let generated = leased
                    .sessions
                    .iter()
                    .find(|s| is_generated(s))
                    .expect("the generated window is observed");
                decisions.push(generated.chat.clone());

                // Last clause: every chat entry the writer emits corresponds
                // to an observed window the conjunction held for — no more,
                // no fewer. `"chat":{` opens a chat object; a non-chat
                // session emits `"chat":null`.
                let state = crate::output::snapshot_to_state_json(
                    &leased,
                    &[],
                    None,
                    &HashMap::new(),
                    &crate::output::ConfigEcho::default(),
                    &[],
                    0,
                );
                let observed_chats = leased.sessions.iter().filter(|s| s.chat.is_some()).count();
                prop_assert_eq!(observed_chats, usize::from(expect_chat));
                prop_assert_eq!(state.matches("\"chat\":{").count(), observed_chats);
                prop_assert_eq!(
                    state.matches("\"role\":\"chat\"").count(),
                    usize::from(expect_chat)
                );
            }

            // "Reads no terminal-emulator-specific window property", in the
            // form a test can check: nine different window classes/app-ids
            // over one identical observation, one identical decision.
            for decision in &decisions {
                prop_assert_eq!(decision, &decisions[0]);
            }
            prop_assert_eq!(decisions[0].is_some(), expect_chat);
        }
    }
}
