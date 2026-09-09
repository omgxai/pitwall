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
use crate::platform::{Platform, RawProcess, WindowInfo};
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
    pub project: Option<ProjectInfo>,
    pub agent: AgentIdentity,
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
/// - window class `org.omarchy.agent` (Omarchy agent-terminal convention);
/// - window title prefix (`OC |`, `CC |`, …) matching the agent family.
///
/// `High` needs ≥2 corroborating signals; a lone title hint is `Low`;
/// a lone command match is `Medium`; nothing at all is `Unknown`.
fn classify_agent(processes: &[ProcessInfo], window: Option<&WindowInfo>) -> AgentIdentity {
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
        for (idx, (token, _, _)) in KNOWN.iter().enumerate() {
            if base_or_name.contains(token) {
                cmd_hits
                    .entry(idx)
                    .or_default()
                    .push(format!("cmd:{} (pid {})", base_or_name, p.pid));
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
        return AgentIdentity {
            kind: AgentKind::Unknown,
            confidence: Confidence::Unknown,
            evidence: Vec::new(),
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

/// Derive the session state from its tree: stopped wins over running wins
/// over sleeping; empty/unknown-only trees are `Unknown`.
fn derive_session_state(processes: &[ProcessInfo]) -> SessionState {
    let mut saw_sleep = false;
    for p in processes {
        match p.state {
            ProcessState::Stopped => return SessionState::Stopped,
            ProcessState::Running | ProcessState::DiskSleep => return SessionState::Running,
            ProcessState::Sleeping | ProcessState::Idle => saw_sleep = true,
            ProcessState::Zombie | ProcessState::Dead | ProcessState::Unknown => {}
        }
    }
    if saw_sleep {
        SessionState::Sleeping
    } else {
        SessionState::Unknown
    }
}

/// Project directory = mode cwd across descendant processes (ties broken by
/// lexicographic order for determinism). Emulator roots (`/`, empty,
/// unreadable) are excluded — verified live that they carry no signal.
fn project_dir_for(processes: &[ProcessInfo]) -> Option<String> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for p in processes {
        let cwd = p.cwd.as_str();
        if cwd.is_empty() || cwd == "/" {
            continue;
        }
        *counts.entry(cwd).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(cwd, _)| cwd.to_string())
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

/// Collect one workspace snapshot. Single-shot and side-effect free
/// (observation only) — no loops, no writes, no network.
pub fn collect(platform: &dyn Platform) -> WorkspaceSnapshot {
    let raw = platform.processes();
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
            if !seen.insert(pid) {
                continue;
            }
            if pid != self_pid {
                tree.push(pid);
            }
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        let process_count = tree.len();
        let mut processes: Vec<ProcessInfo> = tree
            .iter()
            .filter_map(|pid| by_pid.get(pid))
            .map(|r| ProcessInfo {
                pid: r.pid,
                ppid: r.ppid,
                name: r.name.clone(),
                command: r.command.clone(),
                cwd: r.cwd.clone(),
                state: ProcessState::from_code(r.state_code),
                started_at_epoch: to_epoch(r.starttime_ticks),
            })
            .collect();
        processes.sort_by_key(|p| p.pid);
        processes.truncate(MAX_SESSION_PROCESSES);

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

        let agent = classify_agent(&processes, Some(&window));
        let state = derive_session_state(&processes);
        let last_activity_epoch = processes
            .iter()
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
        let summary = build_summary(&agent, project.as_ref(), state, process_count);
        sessions.push(TerminalSession {
            id,
            window: Some(window),
            root_pid: tree.first().copied().unwrap_or(0),
            project,
            agent,
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
    use crate::platform::{GitInfo, Platform, RawProcess, WindowInfo};

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
        assert_eq!(s.agent.kind, AgentKind::Unknown);
        assert_eq!(s.agent.confidence, Confidence::Unknown);
        assert!(s.project.is_none(), "root cwd must not become a project");
        assert_eq!(s.state, SessionState::Sleeping);
        assert_eq!(s.last_activity_epoch, 1_700_000_000); // boot + 10/100
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
            ],
            windows: vec![window("0x3", "foot", "t", 400)],
            repos: HashMap::new(),
        };
        let snap = collect(&plat);
        let s = &snap.sessions[0];
        assert!(s.processes.iter().all(|p| p.pid != self_pid));
        assert_eq!(s.process_count, 1);
        // last_activity must come from the shell, not the observer.
        assert_eq!(s.last_activity_epoch, 1_700_000_000);
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
}
