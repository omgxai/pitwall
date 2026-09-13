//! `pitwall` CLI entry point.
//!
//! M2: single-shot workspace observation plus local persistence.
//! `status` prints a human summary, `status --json` prints the live
//! machine-readable snapshot (stable read API), and `snapshot` collects,
//! persists to the local SQLite continuity cache, and refreshes the
//! `state.json` artifact. No daemon, no network, observation only.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use pitwall_lib::chat;
use pitwall_lib::collector;
use pitwall_lib::output;
use pitwall_lib::platform::{Platform, TerminalSpec};
use pitwall_lib::store;
use pitwall_lib::summary as summary_mod;

fn print_help() {
    println!(
        "pitwall {} — workspace awareness for Omarchy",
        pitwall_lib::version()
    );
    println!();
    println!("USAGE:");
    println!("    pitwall [OPTIONS] <SUBCOMMAND>");
    println!();
    println!("OPTIONS:");
    println!("    -V, --version    Print version and exit");
    println!("    -h, --help       Print this help and exit");
    println!();
    println!("SUBCOMMANDS:");
    println!("    status [--json]  Show detected workspace sessions");
    println!("    snapshot         Persist one observation to the local cache");
    println!("                     [--db PATH] [--state PATH] [--data-dir DIR]");
    println!("    checkpoint       Record a workspace checkpoint now");
    println!("                     [--note TEXT] [--session-id ID]");
    println!("    resume           Focus a live session or open its project terminal");
    println!("                     --session-id ID [--data-dir DIR]");
    println!("    chat             Open a native Pitwall Chat terminal and talk to it");
    println!("                     [--session sess_ID]");
    println!("    agents           List supported AI agents and detection status");
    println!("    models           List models for an agent (--agent ID, default opencode)");
    println!("    summarize        Ask the configured agent for a workspace summary");
    println!("    config             Get/set user configuration (get [key] | set <key> <value>)");
    println!("    notifications      List inbox ([--unread]) or mark read (read <id>)");
    println!("    doctor             Check installed paths and optional Omarchy integration");
    println!("    assign           Assign a task to a live session (--session-id ID --role ROLE --prompt TEXT [--model P/M] [--timeout SECS])");
    println!("                     [--agent ID] [--model P/M] [--dir DIR]");
    println!("                     [--timeout SECS] [--dry-run] [--clear] [--data-dir DIR]");
}

#[cfg(target_os = "linux")]
fn platform() -> impl Platform {
    pitwall_lib::platform::linux::LinuxPlatform
}

#[cfg(not(target_os = "linux"))]
fn platform() -> impl Platform {
    compile_error!("pitwall M2 supports Linux only (see ADR-001/ADR-003)");
}

fn cmd_status(json: bool) -> ExitCode {
    let snapshot = collector::collect(&platform());
    if json {
        println!("{}", output::snapshot_to_json(&snapshot));
    } else {
        print!("{}", output::snapshot_to_text(&snapshot));
    }
    ExitCode::SUCCESS
}

/// Print a small, non-invasive installation report. Doctor never creates
/// state, starts the timer, enables the plugin, or reads process contents.
fn cmd_doctor(args: &[String]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("pitwall doctor: unknown option '{}'.", args[0]);
        return ExitCode::from(2);
    }

    let data_dir = store::default_data_dir();
    let state_path = data_dir.join(store::STATE_FILENAME);
    let db_path = data_dir.join(store::DB_FILENAME);
    let config_path = pitwall_lib::config::config_path();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let plugin_path = home
        .as_ref()
        .map(|h| h.join(".config/omarchy/plugins/dev.pitwall"));
    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("pitwall"));

    println!("Pitwall doctor");
    println!("  version: {}", pitwall_lib::version());
    println!("  binary: {}", binary.display());
    println!("  data: {}", data_dir.display());
    println!("  config: {}", config_path.display());

    let mut failed = false;
    if data_dir.is_dir() {
        println!("  data directory: OK");
    } else if data_dir.exists() {
        println!("  data directory: ERROR (not a directory)");
        failed = true;
    } else {
        println!("  data directory: not initialized (run `pitwall snapshot`)");
    }
    report_artifact("state.json", &state_path);
    report_artifact("SQLite database", &db_path);
    report_artifact("config", &config_path);

    match plugin_path {
        Some(path) if path.join("manifest.json").is_file() => {
            println!("  Omarchy plugin: OK ({})", path.display());
        }
        Some(path) => println!("  Omarchy plugin: not installed ({})", path.display()),
        None => println!("  Omarchy plugin: HOME is unavailable"),
    }

    match std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "pitwall-snapshot.timer"])
        .output()
    {
        Ok(output) => {
            let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if output.status.success() {
                println!(
                    "  snapshot timer: {}",
                    if state.is_empty() { "enabled" } else { &state }
                );
            } else if state.is_empty() {
                println!("  snapshot timer: unavailable or not installed");
            } else {
                println!("  snapshot timer: {state}");
            }
        }
        Err(_) => println!("  snapshot timer: systemctl unavailable"),
    }

    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn report_artifact(label: &str, path: &std::path::Path) {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => println!("  {label}: OK ({})", path.display()),
        Ok(_) => println!("  {label}: ERROR (not a regular file)"),
        Err(_) => println!("  {label}: not present ({})", path.display()),
    }
}

/// Collect one observation, persist it (hash-gated), and refresh the
/// `state.json` artifact. Every failure degrades to a warning: persistence
/// must never make observation fail.
/// Timeline enrichment for state.json: observed age + state history per
/// live session, read from retained observations. Degradable: failures
/// yield an empty map (fields render unknown, never invented).
fn session_meta_map(
    store: &store::Store,
    snapshot: &collector::WorkspaceSnapshot,
) -> std::collections::HashMap<String, output::SessionMeta> {
    let mut map = std::collections::HashMap::new();
    for s in &snapshot.sessions {
        let age_secs = store
            .session_first_seen(&s.id)
            .ok()
            .flatten()
            .map(|first| snapshot.collected_at_epoch.saturating_sub(first).max(0));
        let history = store.session_history(&s.id, 16).unwrap_or_default();
        map.insert(s.id.clone(), output::SessionMeta { age_secs, history });
    }
    map
}

/// Effective user config for state echo and CLI defaults. Reads the
/// config file; missing/unreadable files yield defaults (panel always
/// works, settings show defaults).
fn load_config_echo() -> output::ConfigEcho {
    let cfg = pitwall_lib::config::load_from(&pitwall_lib::config::config_path());
    output::ConfigEcho {
        agent: cfg.agent,
        model: cfg.model,
        summary_enabled: cfg.summary_enabled,
    }
}

/// Minimal config get/set (`pitwall config get [key]`, `pitwall config set
/// key value`). Only known keys; values validated. The settings popup and
/// power users share this path — QML never writes files directly.
fn cmd_config(args: &[String]) -> ExitCode {
    use pitwall_lib::config as config_mod;
    let path = config_mod::config_path();
    match args.first().map(String::as_str) {
        Some("get") => {
            let cfg = config_mod::load_from(&path);
            match args.get(1) {
                None => {
                    print!("{}", cfg.serialize());
                    ExitCode::SUCCESS
                }
                Some(key) => match key.as_str() {
                    "agent" => {
                        println!("{}", cfg.agent);
                        ExitCode::SUCCESS
                    }
                    "model" => {
                        println!("{}", cfg.model);
                        ExitCode::SUCCESS
                    }
                    "summary_enabled" => {
                        println!("{}", cfg.summary_enabled);
                        ExitCode::SUCCESS
                    }
                    _ => {
                        eprintln!("pitwall config: unknown key '{key}'");
                        ExitCode::from(2)
                    }
                },
            }
        }
        Some("set") => {
            let (Some(key), Some(value)) = (args.get(1), args.get(2)) else {
                eprintln!("pitwall config: usage: pitwall config set <key> <value>");
                return ExitCode::from(2);
            };
            let mut cfg = config_mod::load_from(&path);
            match cfg.set(key, value) {
                Ok(()) => match config_mod::save_to(&path, &cfg) {
                    Ok(()) => {
                        println!("config: {key} updated");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("pitwall config: {e}");
                        ExitCode::from(1)
                    }
                },
                Err(e) => {
                    eprintln!("pitwall config: {e}");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("pitwall config: usage: pitwall config (get [key] | set <key> <value>)");
            ExitCode::from(2)
        }
    }
}

/// Notification inbox view for state.json: unread rows (cap 20) plus the
/// badge count (attention + completion). Degradable: store failure yields
/// an empty inbox, never a failed snapshot.
fn notification_view(db: &std::path::Path) -> (Vec<store::Notification>, i64) {
    let Ok(store) = store::Store::open(db) else {
        return (Vec::new(), 0);
    };
    let list = store.unread_notifications(20).unwrap_or_default();
    let badge = store.unread_badge_count().unwrap_or(0);
    (list, badge)
}

fn cmd_snapshot(args: &[String]) -> ExitCode {
    let mut db_path: Option<PathBuf> = None;
    let mut state_path: Option<PathBuf> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--db" => {
                i += 1;
                db_path = args.get(i).map(PathBuf::from);
            }
            "--state" => {
                i += 1;
                state_path = args.get(i).map(PathBuf::from);
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("pitwall snapshot: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let dir = data_dir.unwrap_or_else(store::default_data_dir);
    let db = db_path.unwrap_or_else(|| dir.join(store::DB_FILENAME));
    let state_file = state_path.unwrap_or_else(|| dir.join(store::STATE_FILENAME));

    let snapshot = collector::collect(&platform());
    let mut resumable: Vec<store::Checkpoint> = Vec::new();
    let mut summary_events = Vec::new();
    let mut summary_checkpoints: Vec<store::Checkpoint> = Vec::new();
    let mut last_summary: Option<output::StateSummary> = None;
    let mut meta: std::collections::HashMap<String, output::SessionMeta> =
        std::collections::HashMap::new();

    // 1. Persist (degradable).
    match store::Store::open(&db) {
        Ok(mut s) => {
            // Previous observation for notification diffing (degradable:
            // absent on first run; staleness harmless).
            let previous = s.latest_observation().ok().flatten();
            let prev_sessions: Vec<store::PrevSession> = previous
                .map(|(id, _)| s.observation_sessions(id).ok().unwrap_or_default())
                .unwrap_or_default();
            if let Some((_, collected_at)) = previous {
                let recent = s.checkpoints_since(collected_at, 20).unwrap_or_default();
                summary_events =
                    pitwall_lib::context::derive_events(&prev_sessions, &snapshot, &recent, 20);
            }
            if s.recovered_from_corrupt {
                eprintln!(
                    "pitwall snapshot: warning: corrupt database was quarantined and recreated"
                );
            }
            match s.persist(&snapshot) {
                Ok(store::PersistOutcome::Written { observation_id }) => {
                    println!("snapshot: wrote observation {observation_id}");
                    // Disappearance checkpoints: sessions present last time
                    // but gone now (at most one per absence; re-fires only
                    // after reappearance). Degradable like persist.
                    let live_ids: Vec<String> =
                        snapshot.sessions.iter().map(|s| s.id.clone()).collect();
                    match s.sync_snapshot_notifications(&prev_sessions, &snapshot, now_epoch()) {
                        Ok(n) if n > 0 => {
                            println!("snapshot: {n} notification(s)");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            eprintln!("pitwall snapshot: warning: notification sync failed ({e})");
                        }
                    }
                    match s.checkpoint_disappearances(&live_ids, observation_id, now_epoch()) {
                        Ok(ids) => {
                            for id in ids {
                                println!("snapshot: disappearance checkpoint {id}");
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "pitwall snapshot: warning: disappearance check failed ({e})"
                            );
                        }
                    }
                }
                Ok(store::PersistOutcome::Unchanged) => {
                    println!("snapshot: unchanged, no write");
                }
                Err(e) => {
                    eprintln!(
                        "pitwall snapshot: warning: persist failed ({e}); continuing live-only"
                    );
                }
            }
            // Resumable list for state.json v2 (degradable: empty on failure).
            // Only vanished sessions: live ones already render as rows.
            match s.latest_checkpoints(10) {
                Ok(cps) => {
                    let live: std::collections::HashSet<&str> =
                        snapshot.sessions.iter().map(|s| s.id.as_str()).collect();
                    resumable = cps
                        .into_iter()
                        .filter(|c| !live.contains(c.session_id.as_str()))
                        .collect();
                }
                Err(e) => {
                    eprintln!("pitwall snapshot: warning: checkpoint read failed ({e})");
                }
            }
            if let Ok(all) = s.latest_checkpoints(50) {
                let mut seen = std::collections::HashSet::new();
                summary_checkpoints = all
                    .into_iter()
                    .filter(|cp| seen.insert(cp.project_id.clone()))
                    .take(10)
                    .collect();
            }
            // Timeline enrichment + last cached summary (degradable).
            meta = session_meta_map(&s, &snapshot);
            // Only expose a cached summary when its exact structured input is
            // still current. Otherwise mark it stale instead of presenting an
            // old sentence as though it described this workspace.
            let summary_context = pitwall_lib::context::SummaryContext::new(
                snapshot.clone(),
                summary_events.clone(),
                summary_checkpoints.clone(),
                notification_view(&db).0,
            );
            let current_summary_hash = summary_mod::input_hash(&summary_context);
            match s.latest_summary() {
                Ok(Some(row)) => {
                    last_summary = Some(if row.input_hash == current_summary_hash {
                        output::StateSummary::ready(
                            row.text,
                            row.model,
                            row.created_at,
                            row.input_hash,
                        )
                    } else {
                        output::StateSummary::stale(row.input_hash, row.model, row.created_at)
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    eprintln!("pitwall snapshot: warning: summary read failed ({e})");
                }
            }
        }
        Err(store::StoreError::NewerVersion(v)) => {
            eprintln!(
                "pitwall snapshot: warning: database schema v{v} newer than supported; continuing live-only"
            );
        }
        Err(e) => {
            eprintln!("pitwall snapshot: warning: store unavailable ({e}); continuing live-only");
        }
    }

    // 2. Refresh the state.json artifact (degradable, preserves last good).
    let (notif_list, notif_badge) = notification_view(&db);
    match store::atomic_write(
        &state_file,
        output::snapshot_to_state_json(
            &snapshot,
            &resumable,
            last_summary.as_ref(),
            &meta,
            &load_config_echo(),
            &notif_list,
            notif_badge,
        )
        .as_bytes(),
    ) {
        Ok(()) => println!("snapshot: state artifact {}", state_file.display()),
        Err(e) => {
            eprintln!(
                "pitwall snapshot: warning: state artifact not written ({e}); last good preserved"
            );
        }
    }
    ExitCode::SUCCESS
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(-1)
}

/// Explicit manual checkpoint (`trigger::MANUAL`). Records live sessions;
/// never synthesizes context.
fn cmd_checkpoint(args: &[String]) -> ExitCode {
    let mut note: Option<String> = None;
    let mut only_session: Option<String> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--note" => {
                i += 1;
                note = args.get(i).cloned();
            }
            "--session-id" => {
                i += 1;
                only_session = args.get(i).cloned();
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("pitwall checkpoint: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let dir = data_dir.unwrap_or_else(store::default_data_dir);
    let db = dir.join(store::DB_FILENAME);
    let snapshot = collector::collect(&platform());
    match store::Store::open(&db) {
        Ok(mut s) => match s.checkpoint_live(
            &snapshot,
            only_session.as_deref(),
            note.as_deref(),
            now_epoch(),
        ) {
            Ok(ids) if ids.is_empty() => {
                eprintln!("pitwall checkpoint: no matching live session with a project");
                ExitCode::from(1)
            }
            Ok(ids) => {
                for id in ids {
                    println!("checkpoint: wrote checkpoint {id}");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("pitwall checkpoint: store failed ({e})");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("pitwall checkpoint: store unavailable ({e})");
            ExitCode::from(1)
        }
    }
}

/// Thin CLI wrapper over [`pitwall_lib::resume::resume`] (Level 1–2 only).
fn cmd_resume(args: &[String]) -> ExitCode {
    let mut session_id: Option<String> = None;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session-id" => {
                i += 1;
                session_id = args.get(i).cloned();
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("pitwall resume: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let Some(sid) = session_id else {
        eprintln!("pitwall resume: --session-id is required");
        return ExitCode::from(2);
    };
    let dir = data_dir.unwrap_or_else(store::default_data_dir);
    let db = dir.join(store::DB_FILENAME);
    match pitwall_lib::resume::resume(&platform(), &db, &sid) {
        Ok(pitwall_lib::resume::ResumeOutcome::FocusedLive { session_id }) => {
            println!("resume: focused live session {session_id}");
            ExitCode::SUCCESS
        }
        Ok(pitwall_lib::resume::ResumeOutcome::OpenedTerminal { directory }) => {
            println!("resume: opened terminal at {directory}");
            ExitCode::SUCCESS
        }
        Err(e) if e.contains("malformed") => {
            eprintln!("pitwall resume: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("pitwall resume: {e}");
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------------------
// `pitwall chat` — tasks 12.1 and 12.2 (design §4.2, §4.3)
//
// One subcommand, two shapes, one code path. Everything the chat itself does
// lives in `pitwall_lib::chat`; this file is the startup order, the exit-code
// classification, and the input loop's dispatch table — nothing more.
//
// **The startup order is the safety contract** (design §4.3, Requirement
// 23.2). Usage-class refusals are decided before operational ones, so a
// malformed `--session` exits 2 even when the configured harness is also
// missing, and nothing environmental is touched until the usage group has
// passed. No terminal, no harness invocation, no context file and no chat
// number exists on any refusal path.
//
// **The config is read exactly once, here.** `chat.rs` deliberately contains
// no config read at all, so the harness and model travel from this function
// into `ChatDescriptor::capture` and can never be re-read for the lifetime of
// the process (Requirements 11.1, 11.3, 11.4).
// ---------------------------------------------------------------------------

/// Erase the display and home the cursor. The only raw control sequence in
/// this file: `/clear` clears a *terminal*, which is not chat presentation
/// and so has no place in `chat.rs`.
const CLEAR_SCREEN: &str = "\u{1b}[2J\u{1b}[H";

/// Why `pitwall chat` refused to start, and with which exit code.
///
/// One value per refusal, so the 0/1/2 contract of Requirement 23.2 is a
/// property of the *refusal* rather than of whichever early return happened
/// to produce it: [`ChatRefusal::code`] is the single place that says which
/// class a failure belongs to, and [`ChatRefusal::report`] is the single
/// place that prints one.
///
/// Refusals never echo a value that could carry a control character into the
/// terminal: a malformed session id and a malformed model id are named by
/// class, exactly as [`pitwall_lib::resume::resume`] and
/// [`pitwall_lib::config::Config::set`] already name them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatRefusal {
    /// Step 1: an option the Chat_Command does not accept (9.6).
    UnknownOption { entered: String },
    /// Step 1: `--session` with nothing after it. Refused rather than
    /// silently treated as a whole-workspace chat (9.2, 9.4).
    SessionValueMissing,
    /// Step 2: `--session` outside `sess_[0-9a-f]{16}` (9.4).
    MalformedSession,
    /// Step 3: the configured model fails `summary::valid_model` (11.7).
    MalformedModel,
    /// Step 4: the configured harness is not a known agent id (11.6).
    UnknownHarness { harness: String },
    /// Step 4: the harness is known but not installed here (11.6).
    HarnessNotInstalled { harness: String },
    /// Step 5: `--session` names a session that is not currently live (9.5).
    SessionNotLive { session_id: String },
    /// Step 6: every chat number `001..999` is in use (10.5).
    NoChatNumber { reason: String },
    /// No candidate directory exists at all (16.7).
    NoDirectory,
    /// The chosen directory is not absolute (16.7). Named, never swapped.
    DirectoryNotAbsolute { directory: String },
    /// The chosen directory does not exist as a directory (16.7).
    DirectoryUnavailable { directory: String },
    /// The running binary's absolute path could not be resolved, so the
    /// relaunch argv cannot be built honestly (16.6).
    BinaryUnresolved,
    /// The single Terminal_Launch_Path refused or failed (16.8).
    LaunchFailed { reason: String },
    /// A descriptor could not be captured from already-validated values.
    DescriptorRejected { reason: String },
}

impl ChatRefusal {
    /// The process exit code: 2 for a usage error, 1 for an operational
    /// failure (Requirement 23.2, design §4.3).
    fn code(&self) -> u8 {
        match self {
            ChatRefusal::UnknownOption { .. }
            | ChatRefusal::SessionValueMissing
            | ChatRefusal::MalformedSession
            | ChatRefusal::MalformedModel => 2,
            ChatRefusal::UnknownHarness { .. }
            | ChatRefusal::HarnessNotInstalled { .. }
            | ChatRefusal::SessionNotLive { .. }
            | ChatRefusal::NoChatNumber { .. }
            | ChatRefusal::NoDirectory
            | ChatRefusal::DirectoryNotAbsolute { .. }
            | ChatRefusal::DirectoryUnavailable { .. }
            | ChatRefusal::BinaryUnresolved
            | ChatRefusal::LaunchFailed { .. }
            | ChatRefusal::DescriptorRejected { .. } => 1,
        }
    }

    /// The one sentence printed after the `pitwall chat: ` prefix.
    fn message(&self) -> String {
        match self {
            ChatRefusal::UnknownOption { entered } => format!("unknown option '{entered}'."),
            ChatRefusal::SessionValueMissing => "--session needs a session ID".to_string(),
            ChatRefusal::MalformedSession => "malformed session ID (refusing)".to_string(),
            ChatRefusal::MalformedModel => "malformed model id (refusing)".to_string(),
            ChatRefusal::UnknownHarness { harness } => {
                format!("unknown agent '{harness}' in the configuration")
            }
            ChatRefusal::HarnessNotInstalled { harness } => {
                format!("agent '{harness}' is not installed here")
            }
            ChatRefusal::SessionNotLive { session_id } => {
                format!("session {session_id} is not live (refusing)")
            }
            ChatRefusal::NoChatNumber { reason } => reason.clone(),
            ChatRefusal::NoDirectory => {
                "no directory to open a chat in: HOME is unavailable and no observed \
                 session has a project"
                    .to_string()
            }
            ChatRefusal::DirectoryNotAbsolute { directory } => {
                format!("refusing non-absolute chat directory: {directory}")
            }
            ChatRefusal::DirectoryUnavailable { directory } => {
                format!("chat directory unavailable: {directory}")
            }
            ChatRefusal::BinaryUnresolved => {
                "cannot resolve the running pitwall binary; not guessing one".to_string()
            }
            ChatRefusal::LaunchFailed { reason } => reason.clone(),
            ChatRefusal::DescriptorRejected { reason } => reason.clone(),
        }
    }

    /// Print the refusal on standard error and yield its exit code.
    fn report(&self) -> ExitCode {
        eprintln!("pitwall chat: {}", self.message());
        ExitCode::from(self.code())
    }
}

/// Steps 1-3 of the startup order (design §4.3): the option parse, the
/// `--session` shape check, and the model check. Every refusal in this group
/// is a usage error and exits 2 (9.4, 9.6, 11.7).
///
/// Pure and environment-free on purpose. The group that decides exit code 2
/// must be answerable without a PATH scan, without an observation and without
/// a terminal — which is both why it can run first and why the ordering is
/// testable off-target. The model arrives as a parameter because this
/// function must not read the config either: `cmd_chat` reads it once (11.1).
///
/// `Ok(None)` means no `--session` was given, which is a whole-workspace
/// chat (9.3) rather than a missing argument.
fn chat_usage_check(args: &[String], model: &str) -> Result<Option<String>, ChatRefusal> {
    // Step 1: the established explicit-match parse. `chat` accepts exactly
    // one option, so anything else is an unknown option (9.6).
    let mut session: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session" => {
                i += 1;
                match args.get(i) {
                    Some(value) => session = Some(value.clone()),
                    None => return Err(ChatRefusal::SessionValueMissing),
                }
            }
            other => {
                return Err(ChatRefusal::UnknownOption {
                    entered: other.to_string(),
                })
            }
        }
        i += 1;
    }

    // Step 2: shape before anything looks the session up (9.4). Delegated to
    // the one shape check in the tree, never re-implemented here.
    if let Some(id) = session.as_deref() {
        if !pitwall_lib::resume::is_session_id(id) {
            return Err(ChatRefusal::MalformedSession);
        }
    }

    // Step 3: the configured model. Empty means "agent default" and is
    // legitimate; anything else must pass the existing validator (11.7).
    if !model.is_empty() && !summary_mod::valid_model(model) {
        return Err(ChatRefusal::MalformedModel);
    }

    Ok(session)
}

/// Step 4 of the startup order: the configured harness must be a known agent
/// id and must be installed here. Both are operational failures and exit 1
/// (11.6).
///
/// `installed` is a parameter rather than a PATH scan performed inside, so
/// this stays pure and so the scan happens only after step 3 has passed.
fn chat_harness_check(harness: &str, installed: bool) -> Result<(), ChatRefusal> {
    if !pitwall_lib::agents::KNOWN.iter().any(|a| a.id == harness) {
        return Err(ChatRefusal::UnknownHarness {
            harness: harness.to_string(),
        });
    }
    if !installed {
        return Err(ChatRefusal::HarnessNotInstalled {
            harness: harness.to_string(),
        });
    }
    Ok(())
}

/// Requirement 16.7's ordered chain, as a pure choice among candidates that
/// have already been gathered: the context session's project directory, then
/// the most recently active observed project directory, then `$HOME`.
///
/// Empty candidates are skipped — an observed project with no directory is
/// not a directory. The chosen value is validated by
/// [`chat_directory_check`] and is never quietly swapped for the next
/// candidate when that validation fails: substituting a different directory
/// is exactly what `resume` refuses to do (26.16), and a chat that silently
/// opened somewhere else would be lying about its own scope.
fn chat_directory_candidate(
    scoped_dir: Option<&str>,
    recent_dir: Option<&str>,
    home: Option<&str>,
) -> Option<String> {
    [scoped_dir, recent_dir, home]
        .into_iter()
        .flatten()
        .find(|dir| !dir.is_empty())
        .map(str::to_string)
}

/// Absolute plus `is_dir()`, checked before any launch and before the
/// descriptor is captured (16.7, 16.8).
fn chat_directory_check(directory: &str) -> Result<(), ChatRefusal> {
    if !directory.starts_with('/') {
        return Err(ChatRefusal::DirectoryNotAbsolute {
            directory: directory.to_string(),
        });
    }
    if !std::fs::metadata(directory)
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        return Err(ChatRefusal::DirectoryUnavailable {
            directory: directory.to_string(),
        });
    }
    Ok(())
}

/// The most recently active observed project directory, chosen exactly the
/// way `summarize` chooses its working directory.
fn recent_project_dir(snapshot: &collector::WorkspaceSnapshot) -> Option<String> {
    snapshot
        .sessions
        .iter()
        .filter(|s| s.project.is_some())
        .max_by_key(|s| s.last_activity_epoch)
        .and_then(|s| s.project.as_ref().map(|p| p.dir.clone()))
}

/// The relaunch argument vector: the absolute path of the running binary,
/// the `chat` subcommand, and the already-validated `--session` when there
/// is one.
///
/// Fixed-form by construction (16.6): two or four elements, none of them
/// composed from anything but a validated value, and no shell anywhere. No
/// `--title`, `--app-id` or window-class flag appears — the chat sets its own
/// window title once it is running (16.5, 28.3).
fn chat_relaunch_command(binary: &str, session: Option<&str>) -> Vec<String> {
    let mut command = vec![binary.to_string(), "chat".to_string()];
    if let Some(id) = session {
        command.push("--session".to_string());
        command.push(id.to_string());
    }
    command
}

/// Absolute path of the running `pitwall` binary, for the relaunch argv.
///
/// `terminal_argv` refuses a `command[0]` that is not absolute, so a bare
/// `pitwall` would be rejected at the exec boundary rather than resolved
/// through `PATH`. There is no honest fallback: if the running executable
/// cannot be resolved, the relaunch is refused and named as such.
fn current_binary_path() -> Result<String, ChatRefusal> {
    let exe = std::env::current_exe().map_err(|_| ChatRefusal::BinaryUnresolved)?;
    let text = exe.to_str().ok_or(ChatRefusal::BinaryUnresolved)?;
    if !text.starts_with('/') {
        return Err(ChatRefusal::BinaryUnresolved);
    }
    Ok(text.to_string())
}

/// One line per observed session in this chat's scope.
///
/// Deliberately **not** [`output::snapshot_to_text`]: that renders window
/// addresses and pids, which Requirement 15.2 excludes from everything a chat
/// presents. Agent evidence strings are excluded for the same reason (they
/// carry pids), so an agent is named with its kind and the confidence the
/// observation recorded, and nothing more (13.4). Every field here is one
/// `state.json` already publishes.
fn render_chat_sessions(snapshot: &collector::WorkspaceSnapshot) -> String {
    if snapshot.sessions.is_empty() {
        return "No sessions are observed in this chat's context.\n".to_string();
    }
    let mut out = format!(
        "Observed sessions in this chat's context: {}\n",
        snapshot.sessions.len()
    );
    for s in &snapshot.sessions {
        let project = s
            .project
            .as_ref()
            .map(|p| p.name.as_str())
            .unwrap_or("no project");
        let last = if s.last_activity_epoch < 0 {
            "unknown".to_string()
        } else {
            chat::format_clock_utc(s.last_activity_epoch)
        };
        out.push_str(&format!(
            "  {}  {}  {}  agent {} ({} confidence)  last activity {}\n",
            s.id,
            s.state.as_str(),
            project,
            s.agent.kind.as_str(),
            s.agent.confidence.as_str(),
            last
        ));
    }
    out
}

/// Record one Pitwall turn and print it.
///
/// The human's own line is already on screen — the terminal echoed it as they
/// typed — so only Pitwall's side is printed, while both sides are recorded in
/// the in-memory conversation.
fn say(conversation: &mut chat::Conversation, palette: chat::Palette, text: &str) {
    conversation.record(chat::Role::Pitwall, chat::now_epoch(), text);
    if let Some(turn) = conversation.turns().last() {
        let _ = chat::print_turn(&mut std::io::stdout(), turn, palette);
    }
}

/// The four informational entries of the closed vocabulary (design §5.4).
///
/// Each one prints what the chat already holds or can observe read-only.
/// None of them invokes a harness and none of them can act: observation is
/// side-effect free, and the acting bridge is not reachable from here
/// (14.7, 14.8, 27.10).
fn run_info_command(
    plat: &dyn Platform,
    d: &chat::ChatDescriptor,
    cmd: chat::InfoCommand,
    data_dir: &std::path::Path,
    palette: chat::Palette,
    conversation: &mut chat::Conversation,
) {
    match cmd {
        chat::InfoCommand::Help => say(conversation, palette, &chat::render_vocabulary()),
        chat::InfoCommand::Context => {
            // The bounded document *is* what this chat can see, so it is what
            // `/context` shows — the same artifact `summarize --dry-run`
            // prints, bounded by the same ≤6 sessions, ≤20 events, ≤10
            // checkpoints and ≤16 KB (12.2).
            let observation = chat::observe(plat, d, data_dir);
            let mut text = chat::context_block(d).join("\n");
            if observation.truncated_sessions() > 0 {
                text.push_str(&format!(
                    "\n{} session(s) omitted to stay inside the bounded context.",
                    observation.truncated_sessions()
                ));
            }
            text.push('\n');
            text.push_str(observation.document());
            say(conversation, palette, &text);
        }
        chat::InfoCommand::Sessions => {
            // Scoped by the same function the responder uses, so `/sessions`
            // can never report a session the answers could not see (12.3,
            // 12.4). The store rows are irrelevant to a session listing, so
            // they are not read for one.
            let scope = chat::scope_to_context(
                d.context_session_id(),
                collector::collect(plat),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            );
            say(
                conversation,
                palette,
                &render_chat_sessions(scope.snapshot()),
            );
        }
        chat::InfoCommand::Clear => {
            use std::io::Write as _;
            // Clear the rendered conversation area and the in-memory turns,
            // then put the identity back. Nothing was ever stored, so there
            // is nothing else to undo (19.4, 19.5).
            conversation.clear();
            let mut out = std::io::stdout();
            let _ = out.write_all(CLEAR_SCREEN.as_bytes());
            let _ = out.write_all(chat::render_header(d, palette).as_bytes());
            let _ = out.flush();
            chat::assert_window_title(d);
        }
    }
}

/// The foreground input loop (design §4.6, §5.4) — step 7 of the startup
/// order.
///
/// Returns on `/exit`, on EOF, and on a failed read (which is what a closed
/// terminal window looks like from in here). All three are a normal end and
/// the caller exits 0 (9.7). Nothing in this loop spawns a background process
/// or a daemon (9.9).
///
/// Conversation turns live in the [`chat::Conversation`] value below and
/// nowhere else: nothing here writes to the continuity database, to
/// `state.json`, to a log or to the repository (15.6, 19.4), and the value is
/// dropped when this function returns (19.5).
///
/// The dispatch table is exhaustive over [`chat::ChatInput`] by construction,
/// so a future input class cannot slip through unhandled — and the acting arm
/// is the only one that reaches [`chat::resume_action`] (14.10, 26.3, 27.1).
fn chat_loop(plat: &dyn Platform, d: &chat::ChatDescriptor) {
    let palette = chat::Palette::detect();
    let data_dir = store::default_data_dir();
    let db = data_dir.join(store::DB_FILENAME);
    let mut conversation = chat::Conversation::new();
    let stdin = std::io::stdin();
    let mut line = String::new();

    loop {
        let _ = chat::print_prompt(&mut std::io::stdout(), palette);
        line.clear();
        match stdin.read_line(&mut line) {
            // EOF: the window closed, or piped input ran out.
            Ok(0) => return,
            Ok(_) => {}
            // The terminal is gone; ending is the honest response and it is
            // the same normal end.
            Err(_) => return,
        }

        match chat::classify(&line) {
            // Nothing was asked. Re-prompt without a word about it.
            chat::ChatInput::Blank => {}
            chat::ChatInput::Info(cmd) => {
                run_info_command(plat, d, cmd, &data_dir, palette, &mut conversation);
            }
            // The only arm that may change workspace state, reached only
            // because the human typed `/resume` themselves (26.2, 26.3).
            chat::ChatInput::Act(chat::ActionCommand::Resume { target }) => {
                let report = chat::resume_action(plat, &db, d, target.as_deref());
                say(&mut conversation, palette, &chat::resume_line(&report));
            }
            chat::ChatInput::End => return,
            chat::ChatInput::Unknown { entered } => {
                say(
                    &mut conversation,
                    palette,
                    &chat::unknown_command_message(&entered),
                );
            }
            chat::ChatInput::Question(question) => {
                conversation.record(chat::Role::You, chat::now_epoch(), question.text());
                // Observe, then answer. `respond` receives no platform, no
                // database and no resume handle, so the question path cannot
                // reach an action whatever the answer says (27.1, 27.3).
                let observation = chat::observe(plat, d, &data_dir);
                let answer = match chat::respond(d, &observation, &question) {
                    Ok(answer) => answer,
                    Err(e) => e.message(),
                };
                say(&mut conversation, palette, &answer);
            }
            // Refused rather than truncated, and the chat stays ready.
            chat::ChatInput::QuestionTooLong { chars } => {
                say(
                    &mut conversation,
                    palette,
                    &format!(
                        "that question is {chars} characters; the limit is {}. \
                         Trim it and ask again — nothing was sent.",
                        chat::MAX_CHAT_QUESTION_CHARS
                    ),
                );
            }
        }
    }
}

/// `pitwall chat [--session <session-id>]` — one native Pitwall Chat in the
/// foreground of its own terminal (Requirement 9, design §4.2/§4.3).
///
/// Two shapes, one code path:
///
/// - **stdout is not a TTY** — this invocation has nowhere to talk (the panel
///   runs it from a `Process` with fixed argv), so it opens one native
///   Omarchy terminal running the same subcommand through the single
///   Terminal_Launch_Path and exits 0 (16.1, 16.6, 28.1, 28.2, 28.6);
/// - **stdout is a TTY** — it runs the chat loop right there in the
///   foreground, with no background process and no daemon (9.9).
///
/// Every documented refusal is decided *before* either shape begins, so a
/// refusal reaches the caller's standard error with the right exit code
/// instead of flashing past in a terminal window nobody was watching. On
/// every refusal path no terminal is opened, no harness is invoked, no
/// context file is written, and no chat number is claimed.
fn cmd_chat(args: &[String]) -> ExitCode {
    // The one config read (11.1, 11.4). `chat.rs` has none, so these two
    // values are this chat's harness and model for its whole life however
    // often `pitwall config set` runs afterwards.
    let cfg = pitwall_lib::config::load_from(&pitwall_lib::config::config_path());

    // Steps 1-3 — usage class, exit 2. Nothing environmental yet.
    let session = match chat_usage_check(args, &cfg.model) {
        Ok(session) => session,
        Err(refusal) => return refusal.report(),
    };

    // Step 4 — harness known and installed, exit 1. Read-only PATH scan.
    let installed = pitwall_lib::agents::discover_in(&pitwall_lib::agents::path_dirs())
        .into_iter()
        .any(|a| a.id == cfg.agent.as_str() && a.path.is_some());
    if let Err(refusal) = chat_harness_check(&cfg.agent, installed) {
        return refusal.report();
    }

    // Step 5 — session liveness, exit 1. One observation serves the liveness
    // check, the Context_Label, the working directory and the numbers the
    // allocator must avoid.
    let plat = platform();
    let snapshot = collector::collect(&plat);
    let scoped = match session.as_deref() {
        None => None,
        Some(id) => match snapshot.sessions.iter().find(|s| s.id == id) {
            Some(found) => Some(found),
            None => {
                return ChatRefusal::SessionNotLive {
                    session_id: id.to_string(),
                }
                .report()
            }
        },
    };

    // The working directory (16.7): the ordered chain, then one validation,
    // with no substitution when that validation fails.
    let project = scoped.and_then(|s| s.project.as_ref());
    let home = std::env::var("HOME").ok();
    let recent = recent_project_dir(&snapshot);
    let scoped_dir = project.map(|p| p.dir.as_str());
    let candidate = chat_directory_candidate(scoped_dir, recent.as_deref(), home.as_deref());
    let directory = match candidate {
        Some(directory) => directory,
        None => return ChatRefusal::NoDirectory.report(),
    };
    if let Err(refusal) = chat_directory_check(&directory) {
        return refusal.report();
    }

    // Task 12.2 — the Chat_Launcher. Not a TTY: open one native terminal
    // through the *single* launch path and end. Nothing has been created at
    // this point, so a launch failure leaves no chat entry behind (16.8), and
    // the relaunched process runs this very function again on the other side
    // of the fork, where stdout is a terminal.
    if !std::io::stdout().is_terminal() {
        let binary = match current_binary_path() {
            Ok(binary) => binary,
            Err(refusal) => return refusal.report(),
        };
        let command = chat_relaunch_command(&binary, session.as_deref());
        return match plat.launch_terminal(&TerminalSpec {
            directory: &directory,
            command: &command,
        }) {
            Ok(()) => {
                println!("chat: opened a Pitwall Chat terminal in {directory}");
                ExitCode::SUCCESS
            }
            Err(reason) => ChatRefusal::LaunchFailed { reason }.report(),
        };
    }

    // Step 6 — the chat number, derived from what is running and from nothing
    // stored (10.2, 10.4): the numbers of chat-classified windows in this
    // observation, plus the live leases. The guard releases the number at
    // `/exit` and by `Drop` on every other path out of this function.
    let observed_numbers: Vec<u16> = snapshot
        .sessions
        .iter()
        .filter_map(|s| s.chat.as_ref().map(|c| c.number))
        .collect();
    let leases = plat.chat_leases();
    let processes = plat.processes();
    let mut lease = match chat::allocate_chat_number(&observed_numbers, &leases, &processes) {
        Ok(lease) => lease,
        Err(reason) => return ChatRefusal::NoChatNumber { reason }.report(),
    };

    // Reclaim ephemeral context files left behind by chats that died
    // abnormally (12.8, 15.7). Conservative by construction: an empty
    // observation sweeps nothing, and a live chat's file is never touched.
    let swept = pitwall_lib::context::sweep_orphans(&processes);
    if swept > 0 {
        eprintln!("pitwall chat: reclaimed {swept} orphaned context file(s)");
    }

    // Capture the identity once. Every value below was validated above, so
    // this cannot fail for a reason the human has not already been told
    // about — and it is the last word on the harness, the model, the label,
    // the context session and the start time (11.2, 11.3).
    let context_label = chat::context_label_for(project.map(|p| p.name.as_str()));
    let descriptor = match chat::ChatDescriptor::capture(
        lease.number(),
        &cfg.agent,
        &cfg.model,
        &context_label,
        session.as_deref(),
        chat::now_epoch(),
        &directory,
    ) {
        Ok(descriptor) => descriptor,
        Err(reason) => return ChatRefusal::DescriptorRejected { reason }.report(),
    };

    // Step 7 — claim the window identity, then talk. The title goes out
    // before the header so the window switcher is correct from the first
    // frame (16.2, 16.5).
    chat::assert_window_title(&descriptor);
    chat::show_header(&plat, &descriptor);
    chat_loop(&plat, &descriptor);

    // Explicit release, then `Drop` finds nothing left to do (9.7).
    lease.release();
    ExitCode::SUCCESS
}

/// List supported agents with detection evidence (read-only PATH scan).
fn cmd_agents() -> ExitCode {
    use pitwall_lib::agents::{Availability, ModelDiscovery};
    let found = pitwall_lib::agents::discover_in(&pitwall_lib::agents::path_dirs());
    for a in found {
        let status = match &a.path {
            Some(p) => format!("found {}", p.display()),
            None => "absent".to_string(),
        };
        let level = match a.availability {
            Availability::High => "high",
            Availability::Medium => "medium",
        };
        let models = match a.models {
            ModelDiscovery::Command(sub) => format!("list: {sub}"),
            ModelDiscovery::AgentDefault => "list: agent default".to_string(),
        };
        println!(
            "{:8} {:6} {:7} {} ({})",
            a.id, level, status, a.non_interactive, models
        );
    }
    ExitCode::SUCCESS
}

/// List models for one agent (executes only the verified list subcommand).
fn cmd_models(args: &[String]) -> ExitCode {
    let mut agent = "opencode".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    agent = a.clone();
                }
            }
            other => {
                eprintln!("pitwall models: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    match pitwall_lib::agents::models_for(&agent, &pitwall_lib::agents::path_dirs()) {
        Ok(models) if models.is_empty() => {
            println!("{agent}: no models reported");
            ExitCode::SUCCESS
        }
        Ok(models) => {
            for m in models {
                println!("{m}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pitwall models: {e}");
            ExitCode::from(1)
        }
    }
}

/// Explicit user-triggered workspace summary (M5c). Builds the ephemeral
/// context, optionally prints it (--dry-run), otherwise invokes the
/// configured agent once and prints the extracted text. Cleanup on every
/// path; never persists terminal content.
fn cmd_summarize(args: &[String]) -> ExitCode {
    use pitwall_lib::summary as summary_mod;

    let mut agent = "opencode".to_string();
    let mut model: Option<String> = None;
    let mut dir_override: Option<String> = None;
    let mut timeout_secs = summary_mod::DEFAULT_TIMEOUT_SECS;
    let mut dry_run = false;
    let mut clear_only = false;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    agent = a.clone();
                }
            }
            "--model" => {
                i += 1;
                model = args.get(i).cloned();
            }
            "--dir" => {
                i += 1;
                dir_override = args.get(i).cloned();
            }
            "--timeout" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u64>().ok()) {
                    Some(t) if (5..=600).contains(&t) => timeout_secs = t,
                    _ => {
                        eprintln!("pitwall summarize: --timeout must be 5..600 seconds");
                        return ExitCode::from(2);
                    }
                }
            }
            "--dry-run" => dry_run = true,
            "--clear" => clear_only = true,
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("pitwall summarize: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    if clear_only {
        let dir = data_dir.clone().unwrap_or_else(store::default_data_dir);
        let db = dir.join(store::DB_FILENAME);
        let store = match store::Store::open(&db) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("pitwall summarize: store unavailable ({e})");
                return ExitCode::from(1);
            }
        };
        match store.clear_summaries() {
            Ok(n) => eprintln!(
                "summary cache cleared ({} {})",
                n,
                if n == 1 { "entry" } else { "entries" }
            ),
            Err(e) => {
                eprintln!("pitwall summarize: clear failed ({e})");
                return ExitCode::from(1);
            }
        }
        // Refresh the artifact so the panel drops the text (same contract
        // as `snapshot`; degradable throughout).
        let snapshot = collector::collect(&platform());
        let live: std::collections::HashSet<&str> =
            snapshot.sessions.iter().map(|s| s.id.as_str()).collect();
        let resumable: Vec<store::Checkpoint> = store
            .latest_checkpoints(10)
            .map(|cps| {
                cps.into_iter()
                    .filter(|c| !live.contains(c.session_id.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let meta = session_meta_map(&store, &snapshot);
        let state_path = dir.join(store::STATE_FILENAME);
        let (notif_list, notif_badge) = notification_view(&db);
        let payload = output::snapshot_to_state_json(
            &snapshot,
            &resumable,
            None,
            &meta,
            &load_config_echo(),
            &notif_list,
            notif_badge,
        );
        match store::atomic_write(&state_path, payload.as_bytes()) {
            Ok(()) => eprintln!("pitwall summarize: state artifact {}", state_path.display()),
            Err(e) => eprintln!("pitwall summarize: warning: state artifact not written ({e})"),
        }
        return ExitCode::SUCCESS;
    }

    let plat = platform();
    let snapshot = collector::collect(&plat);

    // Previous observation + recent checkpoints feed the derived events.
    // Degradable: an unavailable store yields an event-free context.
    let (prev_sessions, recent_checkpoints) = (|| {
        let dir = data_dir.clone().unwrap_or_else(store::default_data_dir);
        let db = dir.join(store::DB_FILENAME);
        let store = store::Store::open(&db).ok()?;
        let (obs_id, collected_at) = store.latest_observation().ok()??;
        let prev = store.observation_sessions(obs_id).ok()?;
        let cps = store.checkpoints_since(collected_at, 20).ok()?;
        Some((prev, cps))
    })()
    .unwrap_or((Vec::new(), Vec::new()));
    let events =
        pitwall_lib::context::derive_events(&prev_sessions, &snapshot, &recent_checkpoints, 20);

    // Latest checkpoint per project for context (bounded, newest 10).
    let mut seen_projects = std::collections::HashSet::new();
    let mut checkpoints: Vec<store::Checkpoint> = Vec::new();
    {
        let dir = data_dir.clone().unwrap_or_else(store::default_data_dir);
        let db = dir.join(store::DB_FILENAME);
        if let Ok(store) = store::Store::open(&db) {
            if let Ok(all) = store.latest_checkpoints(50) {
                for cp in all {
                    if seen_projects.insert(cp.project_id.clone()) {
                        checkpoints.push(cp);
                    }
                    if checkpoints.len() >= 10 {
                        break;
                    }
                }
            }
        }
    }

    let summary_context = pitwall_lib::context::SummaryContext::new(
        snapshot.clone(),
        events.clone(),
        checkpoints.clone(),
        notification_view(
            &data_dir
                .clone()
                .unwrap_or_else(store::default_data_dir)
                .join(store::DB_FILENAME),
        )
        .0,
    );
    let (document, truncated) =
        pitwall_lib::context::build_context_from_summary(&plat, &summary_context);
    if truncated > 0 {
        eprintln!("pitwall summarize: note: {truncated} session(s) omitted from context");
    }
    let input_hash = summary_mod::input_hash(&summary_context);
    let data_path = data_dir.clone().unwrap_or_else(store::default_data_dir);
    let state_path = data_path.join(store::STATE_FILENAME);

    // Resumable entries + summary state for the state.json this command
    // refreshes (same contract as `snapshot`; degradable throughout).
    let resumable: Vec<store::Checkpoint>;
    let state_summary: Option<output::StateSummary>;
    let write_state = |resumable: &[store::Checkpoint], summary: Option<&output::StateSummary>| {
        // Timeline enrichment for the artifact (degradable: unknown fields).
        let meta = (|| {
            let db = data_path.join(store::DB_FILENAME);
            let store = store::Store::open(&db).ok()?;
            Some(session_meta_map(&store, &snapshot))
        })()
        .unwrap_or_default();
        let (notif_list, notif_badge) = notification_view(&data_path.join(store::DB_FILENAME));
        let payload = output::snapshot_to_state_json(
            &snapshot,
            resumable,
            summary,
            &meta,
            &load_config_echo(),
            &notif_list,
            notif_badge,
        );
        match store::atomic_write(&state_path, payload.as_bytes()) {
            Ok(()) => eprintln!("pitwall summarize: state artifact {}", state_path.display()),
            Err(e) => eprintln!("pitwall summarize: warning: state artifact not written ({e})"),
        }
    };
    let load_resumable = || -> Vec<store::Checkpoint> {
        let db = data_path.join(store::DB_FILENAME);
        let Ok(store) = store::Store::open(&db) else {
            return Vec::new();
        };
        let Ok(cps) = store.latest_checkpoints(10) else {
            return Vec::new();
        };
        let live: std::collections::HashSet<&str> =
            snapshot.sessions.iter().map(|s| s.id.as_str()).collect();
        cps.into_iter()
            .filter(|c| !live.contains(c.session_id.as_str()))
            .collect()
    };

    if dry_run {
        println!("{document}");
        return ExitCode::SUCCESS;
    }

    // Cache lookup BEFORE staging anything or spawning: identical
    // structured context reuses the stored interpretation, no AI call.
    let cached = (|| {
        let db = data_path.join(store::DB_FILENAME);
        let store = store::Store::open(&db).ok()?;
        store.lookup_summary(&input_hash).ok()?
    })();
    if let Some(row) = cached {
        eprintln!("pitwall summarize: cache hit ({})", row.input_hash);
        resumable = load_resumable();
        state_summary = Some(output::StateSummary::ready(
            row.text.clone(),
            row.model.clone(),
            row.created_at,
            row.input_hash.clone(),
        ));
        write_state(&resumable, state_summary.as_ref());
        println!("{}", row.text);
        return ExitCode::SUCCESS;
    }

    let mut ctx = match pitwall_lib::context::EphemeralContext::create(&document) {
        Ok(ctx) => ctx,
        Err(e) => {
            eprintln!("pitwall summarize: cannot stage context ({e})");
            return ExitCode::from(1);
        }
    };

    // Working directory for the agent: explicit override, else the most
    // recent terminal project, else the current directory. Validated.
    let run_dir = match dir_override {
        Some(d) => d,
        None => snapshot
            .sessions
            .iter()
            .filter(|s| s.project.is_some())
            .max_by_key(|s| s.last_activity_epoch)
            .and_then(|s| s.project.as_ref().map(|p| p.dir.clone()))
            .unwrap_or_else(|| std::env::var("PWD").unwrap_or_else(|_| "/tmp".to_string())),
    };
    if !run_dir.starts_with('/') {
        eprintln!("pitwall summarize: refusing non-absolute directory");
        ctx.close();
        return ExitCode::from(2);
    }
    if !std::fs::metadata(&run_dir)
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        eprintln!("pitwall summarize: directory unavailable: {run_dir}");
        ctx.close();
        return ExitCode::from(1);
    }
    let opencode_bin = match pitwall_lib::agents::path_dirs()
        .iter()
        .map(|d| d.join("opencode"))
        .find(|p| p.is_file())
    {
        Some(p) => p,
        None => {
            eprintln!("pitwall summarize: opencode binary not found on PATH");
            ctx.close();
            return ExitCode::from(1);
        }
    };
    let ctx_path = match ctx.path() {
        Some(p) => p.to_path_buf(),
        None => {
            eprintln!("pitwall summarize: context has no path");
            return ExitCode::from(1);
        }
    };
    let argv =
        match summary_mod::build_argv(&agent, model.as_deref(), &run_dir, &ctx_path, &opencode_bin)
        {
            Ok(argv) => argv,
            Err(e) => {
                eprintln!("pitwall summarize: {e}");
                ctx.close();
                return ExitCode::from(1);
            }
        };
    match summary_mod::run_agent(&argv, None, std::time::Duration::from_secs(timeout_secs)) {
        Ok(raw) => {
            let text = summary_mod::extract_summary_text(&raw);
            ctx.close();
            if text.is_empty() {
                eprintln!("pitwall summarize: agent returned no usable text");
                resumable = load_resumable();
                state_summary = Some(output::StateSummary::error("no usable text"));
                write_state(&resumable, state_summary.as_ref());
                return ExitCode::from(1);
            }
            // Persist final text + minimal metadata (never the context).
            let now = now_epoch();
            let db = data_path.join(store::DB_FILENAME);
            match store::Store::open(&db) {
                Ok(store) => {
                    if let Err(e) = store.store_summary(&input_hash, &text, model.as_deref(), now) {
                        eprintln!("pitwall summarize: warning: cache store failed ({e})");
                    }
                }
                Err(e) => {
                    eprintln!("pitwall summarize: warning: store unavailable ({e})");
                }
            }
            eprintln!("pitwall summarize: generated ({input_hash})");
            resumable = load_resumable();
            state_summary = Some(output::StateSummary::ready(
                text.clone(),
                model.clone(),
                now,
                input_hash.clone(),
            ));
            write_state(&resumable, state_summary.as_ref());
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            // Fixed short failure classes only — never agent internals.
            let short = if e.contains("timed out") {
                "timeout"
            } else if e.contains("not found") || e.contains("spawn failed") {
                "agent unavailable"
            } else {
                "agent error"
            };
            eprintln!("pitwall summarize: {e}");
            ctx.close();
            resumable = load_resumable();
            state_summary = Some(output::StateSummary::error(short));
            write_state(&resumable, state_summary.as_ref());
            ExitCode::from(1)
        }
    }
}

/// Explicit workforce assignment (M5g): one foreground agent run for
/// one live session. Human click required upstream; this CLI validates,
/// spawns, waits, filters, and prints the result text. No persistence,
/// no background execution, no fallbacks.
fn cmd_assign(args: &[String]) -> ExitCode {
    let mut session_id: Option<String> = None;
    let mut role = "Worker".to_string();
    let mut prompt: Option<String> = None;
    let mut model: Option<String> = None;
    let mut timeout_secs = pitwall_lib::summary::DEFAULT_TIMEOUT_SECS;
    let mut data_dir: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--session-id" => {
                i += 1;
                session_id = args.get(i).cloned();
            }
            "--role" => {
                i += 1;
                if let Some(r) = args.get(i) {
                    role = r.clone();
                }
            }
            "--prompt" => {
                i += 1;
                prompt = args.get(i).cloned();
            }
            "--model" => {
                i += 1;
                model = args.get(i).cloned();
            }
            "--timeout" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u64>().ok()) {
                    Some(t) if (5..=600).contains(&t) => timeout_secs = t,
                    _ => {
                        eprintln!("pitwall assign: --timeout must be 5..600 seconds");
                        return ExitCode::from(2);
                    }
                }
            }
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            other => {
                eprintln!("pitwall assign: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let (Some(sid), Some(prompt)) = (session_id, prompt) else {
        eprintln!("pitwall assign: --session-id and --prompt are required");
        return ExitCode::from(2);
    };
    let dir = data_dir.unwrap_or_else(store::default_data_dir);
    let db = dir.join(store::DB_FILENAME);
    // Assignment result notifications: completion on success, attention
    // on failure. Best-effort (store failure never masks the result);
    // detail carries outcome facts only, never prompt/reply bodies.
    fn notify_assign_result(
        db: &std::path::Path,
        a: &pitwall_lib::assign::Assignment,
        ok: bool,
        detail: &str,
    ) {
        let Ok(mut store) = store::Store::open(db) else {
            return;
        };
        let (kind, severity) = if ok {
            (store::notif_kind::ASSIGN_DONE, store::severity::COMPLETION)
        } else {
            (store::notif_kind::ASSIGN_FAILED, store::severity::ATTENTION)
        };
        let _ = store.notify(
            kind,
            &a.session_id,
            &a.project_id,
            if a.project_name.is_empty() {
                "session"
            } else {
                &a.project_name
            },
            None,
            &a.agent_kind,
            "unknown",
            None,
            severity,
            detail,
            now_epoch(),
        );
    }
    match pitwall_lib::assign::prepare(&platform(), &db, &sid, &role, &prompt, model.as_deref()) {
        Ok(a) => {
            match pitwall_lib::assign::execute(&a, std::time::Duration::from_secs(timeout_secs)) {
                Ok(text) => {
                    let detail = format!("{} done ({} chars)", a.role_label, text.chars().count());
                    notify_assign_result(&db, &a, true, &detail);
                    println!("{text}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    let short = if e.contains("timed out") {
                        "timeout"
                    } else if e.contains("spawn failed") || e.contains("not installed") {
                        "agent unavailable"
                    } else {
                        "agent error"
                    };
                    notify_assign_result(
                        &db,
                        &a,
                        false,
                        &format!("{} failed: {short}", a.role_label),
                    );
                    eprintln!("pitwall assign: {e}");
                    ExitCode::from(1)
                }
            }
        }
        Err(e) => {
            let code = if e.contains("malformed") || e.contains("usage") {
                2
            } else {
                1
            };
            eprintln!("pitwall assign: {e}");
            ExitCode::from(code)
        }
    }
}

/// Notification inbox CLI: list (newest first, `--unread` filters) and
/// mark-read by id. Reading is explicit — listing never marks.
fn cmd_notifications(args: &[String]) -> ExitCode {
    let mut data_dir: Option<PathBuf> = None;
    let mut unread_only = false;
    let mut read_id: Option<i64> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--data-dir" => {
                i += 1;
                data_dir = args.get(i).map(PathBuf::from);
            }
            "--unread" => unread_only = true,
            "read" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<i64>().ok()) {
                    Some(id) if id > 0 => read_id = Some(id),
                    _ => {
                        eprintln!("pitwall notifications: read needs a positive id");
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("pitwall notifications: unknown option '{other}'.");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let dir = data_dir.unwrap_or_else(store::default_data_dir);
    let db = dir.join(store::DB_FILENAME);
    let store = match store::Store::open(&db) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pitwall notifications: store unavailable ({e})");
            return ExitCode::from(1);
        }
    };
    if let Some(id) = read_id {
        match store.mark_notification_read(id, now_epoch()) {
            Ok(true) => {
                println!("notification {id} marked read");
                return ExitCode::SUCCESS;
            }
            Ok(false) => {
                eprintln!("pitwall notifications: no unread notification {id}");
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("pitwall notifications: read failed ({e})");
                return ExitCode::from(1);
            }
        }
    }
    let list = if unread_only {
        store.unread_notifications(30)
    } else {
        store.recent_notifications(30)
    };
    match list {
        Ok(rows) => {
            for n in rows {
                let read_mark = if n.read_at.is_some() { " " } else { "•" };
                println!(
                    "{read_mark} {} [{}|{}] {} — {}",
                    n.id, n.kind, n.severity, n.project_name, n.detail
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("pitwall notifications: list failed ({e})");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            print_help();
            ExitCode::from(2)
        }
        Some("--version") | Some("-V") => {
            println!("pitwall {}", pitwall_lib::version());
            ExitCode::SUCCESS
        }
        Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("status") => {
            let json = args.iter().any(|a| a == "--json");
            let unknown: Vec<&String> = args[1..]
                .iter()
                .filter(|a| a.as_str() != "--json")
                .collect();
            if !unknown.is_empty() {
                eprintln!("pitwall status: unknown option '{}'.", unknown[0]);
                return ExitCode::from(2);
            }
            cmd_status(json)
        }
        Some("snapshot") => cmd_snapshot(&args[1..]),
        Some("checkpoint") => cmd_checkpoint(&args[1..]),
        Some("resume") => cmd_resume(&args[1..]),
        Some("chat") => cmd_chat(&args[1..]),
        Some("agents") => cmd_agents(),
        Some("models") => cmd_models(&args[1..]),
        Some("summarize") => cmd_summarize(&args[1..]),
        Some("config") => cmd_config(&args[1..]),
        Some("notifications") => cmd_notifications(&args[1..]),
        Some("assign") => cmd_assign(&args[1..]),
        Some("doctor") => cmd_doctor(&args[1..]),
        Some(other) => {
            eprintln!("pitwall: unknown subcommand '{other}'. Run `pitwall --help`.");
            ExitCode::from(2)
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A well-shaped Pitwall-local session id: `sess_` + 16 lowercase hex.
    const LIVE_ID: &str = "sess_0123456789abcdef";

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    /// A refusal is one printable line: never empty, never multi-line.
    fn assert_one_line(refusal: &ChatRefusal) {
        let message = refusal.message();
        assert!(!message.trim().is_empty(), "{refusal:?}");
        assert!(!message.contains('\n'), "{refusal:?}");
    }

    // --- step 1: the option parse (9.6) ------------------------------------

    #[test]
    fn unknown_option_is_a_usage_error() {
        let refusal = chat_usage_check(&argv(&["--nope"]), "").unwrap_err();
        assert_eq!(
            refusal,
            ChatRefusal::UnknownOption {
                entered: "--nope".to_string(),
            }
        );
        assert_eq!(refusal.code(), 2);
    }

    #[test]
    fn a_positional_argument_is_an_unknown_option() {
        let refusal = chat_usage_check(&argv(&[LIVE_ID]), "").unwrap_err();
        assert_eq!(refusal.code(), 2);
    }

    #[test]
    fn session_without_a_value_is_refused_not_widened_to_the_workspace() {
        let refusal = chat_usage_check(&argv(&["--session"]), "").unwrap_err();
        assert_eq!(refusal, ChatRefusal::SessionValueMissing);
        assert_eq!(refusal.code(), 2);
    }

    #[test]
    fn no_options_means_a_whole_workspace_chat() {
        assert_eq!(chat_usage_check(&argv(&[]), "").unwrap(), None);
    }

    // --- step 2: the `--session` shape (9.4) -------------------------------

    #[test]
    fn malformed_session_exits_two() {
        let bad = ["nope", "sess_", "sess_0123", "sess_0123456789ABCDEF", "0123"];
        for entered in bad {
            let args = argv(&["--session", entered]);
            let refusal = chat_usage_check(&args, "").unwrap_err();
            assert_eq!(refusal, ChatRefusal::MalformedSession, "for {entered:?}");
            assert_eq!(refusal.code(), 2, "for {entered:?}");
        }
    }

    #[test]
    fn well_shaped_session_is_carried_through() {
        let args = argv(&["--session", LIVE_ID]);
        assert_eq!(
            chat_usage_check(&args, "").unwrap(),
            Some(LIVE_ID.to_string())
        );
    }

    // --- step 3: the model (11.7) ------------------------------------------

    #[test]
    fn malformed_model_exits_two() {
        let refusal = chat_usage_check(&argv(&[]), "no slash here").unwrap_err();
        assert_eq!(refusal, ChatRefusal::MalformedModel);
        assert_eq!(refusal.code(), 2);
    }

    #[test]
    fn empty_model_is_the_agent_default_and_passes() {
        assert!(chat_usage_check(&argv(&[]), "").is_ok());
    }

    #[test]
    fn valid_model_passes() {
        assert!(chat_usage_check(&argv(&[]), "openrouter/gpt-4o").is_ok());
    }

    #[test]
    fn the_session_shape_is_checked_before_the_model() {
        // Both are usage errors, so the exit code cannot tell them apart; the
        // refusal identity can, and it must be the earlier stage's.
        let args = argv(&["--session", "nope"]);
        let refusal = chat_usage_check(&args, "no slash").unwrap_err();
        assert_eq!(refusal, ChatRefusal::MalformedSession);
    }

    #[test]
    fn an_unknown_option_is_refused_before_the_session_shape() {
        let args = argv(&["--nope", "--session", "bad"]);
        let refusal = chat_usage_check(&args, "").unwrap_err();
        assert!(matches!(refusal, ChatRefusal::UnknownOption { .. }));
    }

    // --- step 4: the harness (11.6) ----------------------------------------

    #[test]
    fn unknown_harness_is_operational() {
        let refusal = chat_harness_check("nosuchagent", true).unwrap_err();
        assert_eq!(
            refusal,
            ChatRefusal::UnknownHarness {
                harness: "nosuchagent".to_string(),
            }
        );
        assert_eq!(refusal.code(), 1);
    }

    #[test]
    fn absent_harness_is_operational() {
        let refusal = chat_harness_check("opencode", false).unwrap_err();
        assert_eq!(
            refusal,
            ChatRefusal::HarnessNotInstalled {
                harness: "opencode".to_string(),
            }
        );
        assert_eq!(refusal.code(), 1);
    }

    #[test]
    fn known_and_installed_harness_passes() {
        for known in pitwall_lib::agents::KNOWN {
            assert!(chat_harness_check(known.id, true).is_ok(), "{}", known.id);
        }
    }

    /// The load-bearing ordering claim of design §4.3: `cmd_chat` runs
    /// [`chat_usage_check`] before [`chat_harness_check`], so a malformed
    /// `--session` decides both the refusal and the exit code even when the
    /// configured harness is also missing. Composed here in that same order.
    #[test]
    fn usage_refusals_precede_operational_ones() {
        let args = argv(&["--session", "nope"]);
        let first = chat_usage_check(&args, "no slash").unwrap_err();
        assert_eq!(first, ChatRefusal::MalformedSession);
        assert_eq!(first.code(), 2);
        // Had the harness stage run first, the process would have exited 1.
        let harness = chat_harness_check("opencode", false).unwrap_err();
        assert_eq!(harness.code(), 1);
    }

    // --- the exit-code contract as a whole (23.2) --------------------------

    #[test]
    fn usage_class_refusals_all_exit_two() {
        let usage = [
            ChatRefusal::UnknownOption {
                entered: "--x".to_string(),
            },
            ChatRefusal::SessionValueMissing,
            ChatRefusal::MalformedSession,
            ChatRefusal::MalformedModel,
        ];
        for refusal in usage {
            assert_eq!(refusal.code(), 2, "{refusal:?}");
            assert_one_line(&refusal);
        }
    }

    #[test]
    fn operational_refusals_all_exit_one() {
        let operational = [
            ChatRefusal::UnknownHarness {
                harness: "nosuchagent".to_string(),
            },
            ChatRefusal::HarnessNotInstalled {
                harness: "opencode".to_string(),
            },
            ChatRefusal::SessionNotLive {
                session_id: LIVE_ID.to_string(),
            },
            ChatRefusal::NoChatNumber {
                reason: "all chat numbers 001..999 are in use (refusing)".to_string(),
            },
            ChatRefusal::NoDirectory,
            ChatRefusal::DirectoryNotAbsolute {
                directory: "relative".to_string(),
            },
            ChatRefusal::DirectoryUnavailable {
                directory: "/nope".to_string(),
            },
            ChatRefusal::BinaryUnresolved,
            ChatRefusal::LaunchFailed {
                reason: "terminal launch failed".to_string(),
            },
            ChatRefusal::DescriptorRejected {
                reason: "refusing empty context label".to_string(),
            },
        ];
        for refusal in operational {
            assert_eq!(refusal.code(), 1, "{refusal:?}");
            assert_one_line(&refusal);
        }
    }

    #[test]
    fn refusals_name_a_malformed_value_by_class_not_by_echo() {
        // A hostile `--session` or model value never reaches the terminal
        // through a refusal line, because neither line carries it.
        assert_eq!(
            ChatRefusal::MalformedSession.message(),
            "malformed session ID (refusing)"
        );
        assert_eq!(
            ChatRefusal::MalformedModel.message(),
            "malformed model id (refusing)"
        );
    }

    // --- the working-directory chain (16.7) -------------------------------

    #[test]
    fn directory_chain_prefers_the_scoped_project() {
        let chosen = chat_directory_candidate(Some("/scoped"), Some("/recent"), Some("/home/x"));
        assert_eq!(chosen, Some("/scoped".to_string()));
    }

    #[test]
    fn directory_chain_falls_back_to_the_most_recent_project() {
        let chosen = chat_directory_candidate(None, Some("/recent"), Some("/home/x"));
        assert_eq!(chosen, Some("/recent".to_string()));
    }

    #[test]
    fn directory_chain_falls_back_to_home() {
        let chosen = chat_directory_candidate(None, None, Some("/home/x"));
        assert_eq!(chosen, Some("/home/x".to_string()));
    }

    #[test]
    fn directory_chain_skips_empty_candidates() {
        let chosen = chat_directory_candidate(Some(""), Some(""), Some("/home/x"));
        assert_eq!(chosen, Some("/home/x".to_string()));
        assert_eq!(chat_directory_candidate(Some(""), None, Some("")), None);
    }

    #[test]
    fn no_candidate_at_all_is_refused() {
        assert_eq!(chat_directory_candidate(None, None, None), None);
        assert_eq!(ChatRefusal::NoDirectory.code(), 1);
    }

    #[test]
    fn directory_check_refuses_a_relative_path_and_names_it() {
        let refusal = chat_directory_check("relative/dir").unwrap_err();
        assert_eq!(
            refusal,
            ChatRefusal::DirectoryNotAbsolute {
                directory: "relative/dir".to_string(),
            }
        );
        // The offending path is named, never substituted (16.8, 26.16).
        assert!(refusal.message().contains("relative/dir"));
    }

    #[test]
    fn directory_check_refuses_a_missing_directory_and_names_it() {
        let missing = "/pitwall-does-not-exist-9f3a1c";
        let refusal = chat_directory_check(missing).unwrap_err();
        assert_eq!(
            refusal,
            ChatRefusal::DirectoryUnavailable {
                directory: missing.to_string(),
            }
        );
        assert!(refusal.message().contains(missing));
    }

    #[test]
    fn directory_check_accepts_an_existing_absolute_directory() {
        assert!(chat_directory_check("/").is_ok());
    }

    #[test]
    fn directory_check_refuses_a_path_that_is_not_a_directory() {
        // Present on every Unix, and a file rather than a directory.
        assert!(chat_directory_check("/etc/hosts").is_err());
    }

    // --- the relaunch argv (16.6, 28.3) -----------------------------------

    #[test]
    fn relaunch_command_is_the_binary_the_subcommand_and_the_session() {
        let command = chat_relaunch_command("/usr/local/bin/pitwall", Some(LIVE_ID));
        let expected = argv(&["/usr/local/bin/pitwall", "chat", "--session", LIVE_ID]);
        assert_eq!(command, expected);
    }

    #[test]
    fn relaunch_command_without_a_session_is_two_elements() {
        let command = chat_relaunch_command("/usr/local/bin/pitwall", None);
        assert_eq!(command, argv(&["/usr/local/bin/pitwall", "chat"]));
    }

    #[test]
    fn relaunch_command_names_no_emulator_and_passes_no_identity_flag() {
        let command = chat_relaunch_command("/usr/local/bin/pitwall", Some(LIVE_ID));
        let forbidden = ["--app-id", "--class", "--title", "--hold", "foot", "kitty"];
        for element in &command {
            for bad in forbidden {
                assert_ne!(element.as_str(), bad, "{element} must not be in the argv");
            }
        }
        // `command[0]` must be absolute: `terminal_argv` refuses anything else.
        assert!(command[0].starts_with('/'));
    }

    #[test]
    fn the_running_binary_resolves_to_an_absolute_path() {
        // The test binary is a real executable, so this exercises the same
        // resolution the relaunch performs.
        let path = current_binary_path().expect("the test binary has a path");
        assert!(path.starts_with('/'));
    }

    // --- `/sessions` presentation (13.4, 15.2) ----------------------------

    #[test]
    fn session_listing_is_honest_when_nothing_is_observed() {
        let snapshot = collector::WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 0,
            hostname: "testbox".to_string(),
            sessions: Vec::new(),
        };
        let text = render_chat_sessions(&snapshot);
        assert!(text.contains("No sessions are observed"));
        assert!(text.ends_with('\n'));
    }

    // --- `/clear` -----------------------------------------------------------

    #[test]
    fn the_clear_sequence_is_exactly_erase_and_home() {
        assert_eq!(CLEAR_SCREEN, "\u{1b}[2J\u{1b}[H");
    }
}
