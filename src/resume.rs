//! Narrow, deterministic Resume (M4).
//!
//! Safety levels: Level 1 (focus a live session) and Level 2 (open one
//! terminal at a validated project directory). There is deliberately no
//! Level 3 (agent start) or Level 4 (arbitrary execution) here — this
//! module cannot construct commands, only select fixed-form operations.
//!
//! Every failure refuses with a reason. Resume never falls back to another
//! directory, never guesses among sessions, and never starts an agent.

use crate::collector;
use crate::platform::{Platform, TerminalSpec};
use crate::store;
use std::path::Path;

/// Session-id shape: `sess_` + 16 lowercase hex (the FNV-1a id family).
/// Anything else is refused before any lookup or action.
pub fn is_session_id(value: &str) -> bool {
    match value.strip_prefix("sess_") {
        Some(rest) => {
            rest.len() == 16
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        }
        None => false,
    }
}

/// What Resume did.
#[derive(Debug, PartialEq, Eq)]
pub enum ResumeOutcome {
    FocusedLive { session_id: String },
    OpenedTerminal { directory: String },
}

/// Resolve and perform Resume for `session_id`.
///
/// - live session found → focus it (Level 1), return `FocusedLive`;
/// - else checkpoint found + directory validates → open terminal (Level 2);
/// - else `Err(reason)`: unknown session, bad directory, launch failure.
///   Focus failure also errors WITHOUT falling back to Level 2 — silently
///   changing levels would change the meaning of the user's click.
pub fn resume(
    platform: &dyn Platform,
    db_path: &Path,
    session_id: &str,
) -> Result<ResumeOutcome, String> {
    if !is_session_id(session_id) {
        return Err("malformed session ID (refusing)".to_string());
    }

    // Level 1: the session is alive — focus, don't duplicate.
    let snapshot = collector::collect(platform);
    if let Some(live) = snapshot.sessions.iter().find(|s| s.id == session_id) {
        let addr = live
            .window
            .as_ref()
            .map(|w| w.address.as_str())
            .unwrap_or("");
        return platform
            .focus_window_address(addr)
            .map(|()| ResumeOutcome::FocusedLive {
                session_id: session_id.to_string(),
            })
            .map_err(|e| format!("focus failed ({e}); not falling back"));
    }

    // Level 2: checkpointed session — validate the directory, then launch.
    let db = store::Store::open(db_path).map_err(|e| format!("store unavailable ({e})"))?;
    let cp = db
        .latest_checkpoint_for_session(session_id)
        .map_err(|e| format!("checkpoint lookup failed ({e})"))?
        .ok_or_else(|| format!("unknown session {session_id} (no checkpoint)"))?;
    if !cp.project_dir.starts_with('/') {
        return Err("refusing non-absolute project dir".to_string());
    }
    match std::fs::metadata(&cp.project_dir) {
        Ok(md) if md.is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "project path is not a directory: {}",
                cp.project_dir
            ))
        }
        Err(_) => return Err(format!("project directory unavailable: {}", cp.project_dir)),
    }
    // Level 2 is an interactive terminal only: an empty command keeps the
    // launched argv exactly what it has always been (Requirement 23.1).
    let launched = platform.launch_terminal(&TerminalSpec {
        directory: &cp.project_dir,
        command: &[],
    });
    launched
        .map(|()| ResumeOutcome::OpenedTerminal {
            directory: cp.project_dir,
        })
        .map_err(|e| format!("terminal launch failed ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{ChatLease, GitInfo, InlineImage, RawProcess, WindowInfo};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    /// The recording platform mock. Every action the trait exposes is
    /// counted or captured here, which is what lets "no action happened"
    /// be asserted mechanically rather than argued.
    ///
    /// Note on harness spawns: the harness is not a `Platform` capability
    /// (it goes through `summary::run_agent`), so it cannot be recorded
    /// here. Harness-spawn counting uses fake harness binaries in a temp
    /// dir on `PATH` (chat's tests).
    struct MockPlatform {
        sessions: Vec<(String, String)>, // (session_id, project_dir)
        launched: std::cell::RefCell<Vec<String>>,
        focused: std::cell::RefCell<Vec<String>>,
        /// Leases this mock reports; `chat_leases` never touches a disk.
        leases: Vec<ChatLease>,
        /// How many times `chat_leases` was called.
        lease_reads: std::cell::Cell<usize>,
        fail_launch: bool,
        fail_focus: bool,
    }

    impl MockPlatform {
        fn with(sessions: Vec<(String, String)>) -> Self {
            MockPlatform {
                sessions,
                launched: std::cell::RefCell::new(Vec::new()),
                focused: std::cell::RefCell::new(Vec::new()),
                leases: Vec::new(),
                lease_reads: std::cell::Cell::new(0),
                fail_launch: false,
                fail_focus: false,
            }
        }
    }

    impl Platform for MockPlatform {
        fn processes(&self) -> Vec<RawProcess> {
            Vec::new()
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.sessions
                .iter()
                .enumerate()
                .map(|(n, _)| WindowInfo {
                    address: format!("0x{}", n + 1),
                    class: "foot".to_string(),
                    initial_class: "foot".to_string(),
                    title: "t".to_string(),
                    workspace: "1".to_string(),
                    pid: 100 + n as u32,
                })
                .collect()
        }
        fn git_info(&self, _dir: &str) -> GitInfo {
            GitInfo::default()
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
        fn launch_terminal(&self, spec: &TerminalSpec<'_>) -> Result<(), String> {
            if self.fail_launch {
                return Err("boom".to_string());
            }
            // Record argv surface: resume must always ask for a bare
            // interactive terminal, never a command.
            assert!(
                spec.command.is_empty(),
                "resume must never pass a command to the terminal"
            );
            self.launched.borrow_mut().push(spec.directory.to_string());
            Ok(())
        }
        fn chat_leases(&self) -> Vec<ChatLease> {
            self.lease_reads.set(self.lease_reads.get() + 1);
            self.leases.clone()
        }
        fn inline_image_capability(&self) -> InlineImage {
            // Fixed: a test must never probe a real terminal.
            InlineImage::None
        }
        fn focus_window_address(&self, address: &str) -> Result<(), String> {
            if self.fail_focus {
                return Err("gone".to_string());
            }
            self.focused.borrow_mut().push(address.to_string());
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

    // NOTE: collect() derives session ids from project/window/pid, so live
    // sessions can't use canned `sess_*` ids. These tests therefore drive
    // Level 2+ via checkpoints, and Level 1 via a real collected id.
    fn temp_db() -> (std::path::PathBuf, std::path::PathBuf) {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m4-resume-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("pitwall.db");
        (dir, db)
    }

    fn checkpoint_row(
        store: &mut store::Store,
        session_id: &str,
        project_dir: &str,
        now: i64,
    ) -> i64 {
        store
            .insert_checkpoint(
                now,
                "proj_x",
                session_id,
                project_dir,
                Some("main"),
                Some(true),
                "opencode",
                "high",
                "sleeping",
                now - 60,
                Some("0x1"),
                Some("foot"),
                None,
                store::trigger::DISAPPEARANCE,
                None,
            )
            .unwrap()
    }

    #[test]
    fn session_id_shape_is_strict() {
        assert!(is_session_id("sess_0123456789abcdef"));
        assert!(!is_session_id(""));
        assert!(!is_session_id("sess_0123456789abcde")); // short
        assert!(!is_session_id("sess_0123456789abcdef0")); // long
        assert!(!is_session_id("sess_0123456789ABCDEF")); // upper
        assert!(!is_session_id("sess_zzzzzzzzzzzzzzzz"));
        assert!(!is_session_id("proj_0123456789abcdef"));
        assert!(!is_session_id("sess_0123456789abcdef;rm"));
    }

    #[test]
    fn malformed_id_is_refused_before_anything() {
        let plat = MockPlatform::with(vec![]);
        let (_dir, db) = temp_db();
        let err = resume(&plat, &db, "not-a-session").unwrap_err();
        assert!(err.contains("malformed"));
        assert!(plat.launched.borrow().is_empty());
        assert!(plat.focused.borrow().is_empty());
    }

    #[test]
    fn live_session_is_focused_not_duplicated() {
        // A live windowed session: collect() builds its id; resume it.
        let plat = MockPlatform::with(vec![("ignored".to_string(), "/tmp".to_string())]);
        let snap = collector::collect(&plat);
        assert_eq!(snap.sessions.len(), 1);
        // The collected id is a real sess_* id — resume must focus it.
        let sid = snap.sessions[0].id.clone();
        assert!(is_session_id(&sid));
        let (_dir, db) = temp_db();
        let out = resume(&plat, &db, &sid).unwrap();
        assert_eq!(
            out,
            ResumeOutcome::FocusedLive {
                session_id: sid.clone()
            }
        );
        assert_eq!(plat.focused.borrow().len(), 1);
        assert!(plat.launched.borrow().is_empty(), "never open a duplicate");
    }

    #[test]
    fn focus_failure_does_not_fall_back_to_launch() {
        let mut plat = MockPlatform::with(vec![("x".to_string(), "/tmp".to_string())]);
        plat.fail_focus = true;
        let snap = collector::collect(&plat);
        let sid = snap.sessions[0].id.clone();
        let (_dir, db) = temp_db();
        let err = resume(&plat, &db, &sid).unwrap_err();
        assert!(err.contains("not falling back"));
        assert!(plat.launched.borrow().is_empty());
    }

    #[test]
    fn vanished_session_opens_validated_terminal() {
        let (dir, db) = temp_db();
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut store = store::Store::open(&db).unwrap();
        checkpoint_row(
            &mut store,
            "sess_aaaaaaaaaaaaaaaa",
            &proj.to_string_lossy(),
            1_700_000_200,
        );
        drop(store);
        let plat = MockPlatform::with(vec![]);
        let out = resume(&plat, &db, "sess_aaaaaaaaaaaaaaaa").unwrap();
        assert_eq!(
            out,
            ResumeOutcome::OpenedTerminal {
                directory: proj.to_string_lossy().into_owned()
            }
        );
        assert_eq!(plat.launched.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_directory_refuses_without_fallback() {
        let (dir, db) = temp_db();
        let mut store = store::Store::open(&db).unwrap();
        checkpoint_row(
            &mut store,
            "sess_bbbbbbbbbbbbbbbb",
            "/no/such/project",
            1_700_000_200,
        );
        drop(store);
        let plat = MockPlatform::with(vec![]);
        let err = resume(&plat, &db, "sess_bbbbbbbbbbbbbbbb").unwrap_err();
        assert!(err.contains("unavailable"));
        assert!(plat.launched.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_directory_path_refuses() {
        let (dir, db) = temp_db();
        let file = dir.join("afile");
        std::fs::write(&file, "x").unwrap();
        let mut store = store::Store::open(&db).unwrap();
        checkpoint_row(
            &mut store,
            "sess_cccccccccccccccc",
            &file.to_string_lossy(),
            1_700_000_200,
        );
        drop(store);
        let plat = MockPlatform::with(vec![]);
        let err = resume(&plat, &db, "sess_cccccccccccccccc").unwrap_err();
        assert!(err.contains("not a directory"));
        assert!(plat.launched.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_session_refuses() {
        let (dir, db) = temp_db();
        // Fresh DB, valid shape, no checkpoint.
        let plat = MockPlatform::with(vec![]);
        let err = resume(&plat, &db, "sess_dddddddddddddddd").unwrap_err();
        assert!(err.contains("unknown session"));
        assert!(plat.launched.borrow().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launch_failure_surfaces_without_side_effects() {
        let (dir, db) = temp_db();
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut store = store::Store::open(&db).unwrap();
        checkpoint_row(
            &mut store,
            "sess_eeeeeeeeeeeeeeee",
            &proj.to_string_lossy(),
            1_700_000_200,
        );
        drop(store);
        let mut plat = MockPlatform::with(vec![]);
        plat.fail_launch = true;
        let err = resume(&plat, &db, "sess_eeeeeeeeeeeeeeee").unwrap_err();
        assert!(err.contains("launch failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multiple_live_sessions_never_guess() {
        // Two live sessions: resuming one focuses exactly that one.
        let plat = MockPlatform::with(vec![
            ("a".to_string(), "/tmp".to_string()),
            ("b".to_string(), "/tmp".to_string()),
        ]);
        let snap = collector::collect(&plat);
        assert_eq!(snap.sessions.len(), 2);
        let target = snap.sessions[1].id.clone();
        let (_dir, db) = temp_db();
        let out = resume(&plat, &db, &target).unwrap();
        assert_eq!(
            out,
            ResumeOutcome::FocusedLive {
                session_id: target.clone()
            }
        );
        assert_eq!(plat.focused.borrow().len(), 1);
        // The focused address belongs to the target window.
        let want = snap.sessions[1].window.as_ref().unwrap().address.clone();
        assert_eq!(plat.focused.borrow()[0], want);
    }

    #[test]
    fn resume_never_starts_an_agent() {
        // Static guarantee by construction: this module has no agent-launch
        // path. This test pins the public surface: only two outcomes exist.
        let (dir, db) = temp_db();
        let proj = dir.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let mut store = store::Store::open(&db).unwrap();
        checkpoint_row(
            &mut store,
            "sess_ffffffffffffffff",
            &proj.to_string_lossy(),
            1_700_000_200,
        );
        drop(store);
        let plat = MockPlatform::with(vec![]);
        match resume(&plat, &db, "sess_ffffffffffffffff").unwrap() {
            ResumeOutcome::OpenedTerminal { .. } => {}
            ResumeOutcome::FocusedLive { .. } => panic!("no live sessions exist"),
        }
        // The only argv surface the platform saw was the terminal launcher.
        assert_eq!(plat.launched.borrow().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
