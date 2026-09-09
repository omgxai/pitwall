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

/// Collect one observation, persist it (hash-gated), and refresh the
/// `state.json` artifact. Every failure degrades to a warning: persistence
/// must never make observation fail.
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

    // 1. Persist (degradable).
    match store::Store::open(&db) {
        Ok(mut s) => {
            if s.recovered_from_corrupt {
                eprintln!(
                    "pitwall snapshot: warning: corrupt database was quarantined and recreated"
                );
            }
            match s.persist(&snapshot) {
                Ok(store::PersistOutcome::Written { observation_id }) => {
                    println!("snapshot: wrote observation {observation_id}");
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
    match store::atomic_write(
        &state_file,
        output::snapshot_to_state_json(&snapshot).as_bytes(),
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
        Some(other) => {
            eprintln!("pitwall: unknown subcommand '{other}'. Run `pitwall --help`.");
            ExitCode::from(2)
        }
    }
}
