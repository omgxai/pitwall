//! `pitwall` CLI entry point.
//!
//! M2: single-shot workspace observation plus local persistence.
//! `status` prints a human summary, `status --json` prints the live
//! machine-readable snapshot (stable read API), and `snapshot` collects,
//! persists to the local SQLite continuity cache, and refreshes the
//! `state.json` artifact. No daemon, no network, observation only.

use std::path::PathBuf;
use std::process::ExitCode;

use pitwall_lib::collector;
use pitwall_lib::output;
use pitwall_lib::platform::Platform;
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
    match summary_mod::run_agent(&argv, std::time::Duration::from_secs(timeout_secs)) {
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
