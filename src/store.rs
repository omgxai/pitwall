//! Local continuity cache (M2).
//!
//! SQLite-backed bounded history of workspace observations plus the
//! `state.json` artifact writer. Local-only: one file under the data dir,
//! no network, no sync.
//!
//! Design (see pre-flight review):
//!
//! - three tables (`meta`, `observations`, `sessions`), `PRAGMA user_version`
//!   schema gating, no migration framework;
//! - hash-gated writes: a snapshot is persisted only when its *meaningful*
//!   content differs from the last persisted one (timestamps excluded), so
//!   idle refreshes cost zero disk writes;
//! - bounded cache: the newest [`MAX_OBSERVATIONS`] observations are kept,
//!   older rows pruned on every write (single deterministic policy);
//! - privacy: full argv/cmdline NEVER enters SQLite (see [`persist`]); only
//!   process names, states, counts, and activity metadata are stored;
//! - failure: corruption quarantines + recreates, newer versions refuse
//!   writes, all failures degrade to live-only observation.

use crate::collector::WorkspaceSnapshot;
use crate::ids;
use rusqlite::{Connection, Error as RusqliteError};
use std::path::{Path, PathBuf};

/// SQLite `user_version` this code understands. Bump only with a new
/// `create_schema` that old binaries must refuse (see [`StoreError::NewerVersion`]).
pub const STORE_SCHEMA_VERSION: i64 = 1;

/// Bounded-cache policy: keep the newest N observations (and their
/// sessions). One deterministic rule — no time-based second policy.
pub const MAX_OBSERVATIONS: i64 = 100;

pub const DB_FILENAME: &str = "pitwall.db";
pub const STATE_FILENAME: &str = "state.json";

const SCHEMA_SQL: &str = "
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE observations (
  id           INTEGER PRIMARY KEY,
  collected_at INTEGER NOT NULL,
  hostname     TEXT NOT NULL
);
CREATE TABLE sessions (
  observation_id      INTEGER NOT NULL,
  session_id          TEXT NOT NULL,
  project_id          TEXT,
  project_dir         TEXT,
  project_name        TEXT,
  is_git_repo         INTEGER,
  branch              TEXT,
  git_clean           INTEGER,
  agent_kind          TEXT NOT NULL,
  agent_confidence    TEXT NOT NULL,
  state               TEXT NOT NULL,
  process_count       INTEGER NOT NULL,
  last_activity_epoch INTEGER NOT NULL,
  root_pid            INTEGER NOT NULL,
  window_address      TEXT,
  window_class        TEXT,
  window_title        TEXT,
  workspace           TEXT
);
CREATE INDEX idx_sessions_observation ON sessions (observation_id);
CREATE INDEX idx_sessions_session ON sessions (session_id);
";

/// Failures opening or using the store. All are degradable: the caller
/// falls back to live-only observation and warns on stderr.
#[derive(Debug)]
pub enum StoreError {
    /// Underlying SQLite/IO failure (permissions, readonly FS, …).
    /// Never triggers quarantine — only corruption does.
    Backend(String),
    /// DB was written by a newer Pitwall (`user_version` ahead). Refuse
    /// writes; live observation continues.
    NewerVersion(i64),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Backend(msg) => write!(f, "store backend error: {msg}"),
            StoreError::NewerVersion(v) => write!(
                f,
                "database schema version {v} is newer than supported v{STORE_SCHEMA_VERSION}; running live-only"
            ),
        }
    }
}

impl From<RusqliteError> for StoreError {
    fn from(e: RusqliteError) -> Self {
        StoreError::Backend(e.to_string())
    }
}

/// Outcome of [`Store::persist`].
#[derive(Debug, PartialEq, Eq)]
pub enum PersistOutcome {
    /// New observation row written.
    Written { observation_id: i64 },
    /// Meaningful content identical to last persisted snapshot; no write.
    Unchanged,
}

/// Opened, version-checked store handle.
pub struct Store {
    conn: Connection,
    pub path: PathBuf,
    /// True when the database was quarantined + recreated during [`Store::open`].
    pub recovered_from_corrupt: bool,
}

/// Default data directory: `$XDG_DATA_HOME/pitwall`, else
/// `~/.local/share/pitwall`.
pub fn default_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("pitwall");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".local/share/pitwall")
}

/// True only for errors that prove the file is not a usable database
/// (corruption), as opposed to environmental failures (permissions, missing
/// dirs, readonly FS) which must NOT trigger quarantine.
fn is_corruption(e: &RusqliteError) -> bool {
    // SQLite reports corruption as "file is not a database" (code 26) or
    // "database disk image is malformed" (code 11). Matched on message text
    // deliberately: rusqlite's error-code types shift across versions, but
    // these messages are stable SQLite engine strings.
    match e {
        RusqliteError::SqliteFailure(_, msg) => {
            let m = msg.as_deref().unwrap_or("").to_lowercase();
            m.contains("not a database") || m.contains("malformed")
        }
        _ => false,
    }
}

fn read_user_version(conn: &Connection) -> Result<i64, RusqliteError> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
}

fn has_meta_table(conn: &Connection) -> Result<bool, RusqliteError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
        [],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn create_schema(conn: &Connection) -> Result<(), RusqliteError> {
    conn.execute_batch(&format!(
        "{SCHEMA_SQL}\nPRAGMA user_version = {STORE_SCHEMA_VERSION};"
    ))
}

fn quarantine(corrupt_path: &Path) -> Result<PathBuf, StoreError> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let aside = corrupt_path.with_extension(format!("db.corrupt-{ts}"));
    std::fs::rename(corrupt_path, &aside)
        .map_err(|e| StoreError::Backend(format!("cannot quarantine corrupt db: {e}")))?;
    Ok(aside)
}

impl Store {
    /// Open (or create) the store at `path`, enforcing the failure model:
    /// fresh-create on first run, quarantine + recreate on corruption,
    /// [`StoreError::NewerVersion`] when ahead, plain error otherwise.
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| StoreError::Backend(format!("cannot create data dir: {e}")))?;
            }
        }
        match Self::open_existing(path, false) {
            Ok(store) => Ok(store),
            Err(StoreError::Backend(msg)) => {
                // Only corruption justifies quarantine; re-probe to decide.
                let probe = Connection::open(path).and_then(|c| has_meta_table(&c));
                match probe {
                    Err(e) if is_corruption(&e) => {
                        let aside = quarantine(path)?;
                        eprintln!(
                            "pitwall: database corrupt; moved aside to {} and recreating fresh",
                            aside.display()
                        );
                        let mut store = Self::open_existing(path, true)?;
                        store.recovered_from_corrupt = true;
                        Ok(store)
                    }
                    _ => Err(StoreError::Backend(msg)),
                }
            }
            Err(e) => Err(e),
        }
    }

    fn open_existing(path: &Path, fresh: bool) -> Result<Store, StoreError> {
        let conn = Connection::open(path).map_err(|e| {
            if !fresh {
                StoreError::from(e)
            } else {
                StoreError::Backend(format!("cannot recreate database: {e}"))
            }
        })?;
        if !has_meta_table(&conn).map_err(StoreError::from)? {
            create_schema(&conn).map_err(StoreError::from)?;
            return Ok(Store {
                conn,
                path: path.to_path_buf(),
                recovered_from_corrupt: false,
            });
        }
        let version = read_user_version(&conn).map_err(StoreError::from)?;
        if version > STORE_SCHEMA_VERSION {
            return Err(StoreError::NewerVersion(version));
        }
        if version < STORE_SCHEMA_VERSION {
            // Meta exists but version predates us: schema unknown, recreate.
            // (Unreachable in practice; v1 is the first schema.)
            create_schema(&conn).map_err(StoreError::from)?;
        }
        Ok(Store {
            conn,
            path: path.to_path_buf(),
            recovered_from_corrupt: false,
        })
    }

    /// Number of stored observations (bounded by [`MAX_OBSERVATIONS`]).
    pub fn observation_count(&self) -> Result<i64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))
            .map_err(StoreError::from)?;
        Ok(n)
    }

    fn last_snapshot_hash(&self) -> Result<Option<String>, StoreError> {
        let hash: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='last_snapshot_hash'",
                [],
                |row| row.get(0),
            )
            .ok();
        Ok(hash)
    }

    /// Persist a snapshot unless its meaningful content is unchanged.
    ///
    /// PRIVACY: full argv/cmdline never reaches SQLite. Persisted per
    /// session: identity, project/git context, agent kind + confidence,
    /// state, counts, activity epoch, window facts. See `SECURITY.md`
    /// ("Persistence boundary").
    pub fn persist(&mut self, snapshot: &WorkspaceSnapshot) -> Result<PersistOutcome, StoreError> {
        let hash = meaningful_hash(snapshot);
        if self.last_snapshot_hash()? == Some(hash.clone()) {
            return Ok(PersistOutcome::Unchanged);
        }
        let tx = self.conn.transaction().map_err(StoreError::from)?;
        let observation_id: i64 = {
            tx.query_row(
                "INSERT INTO observations (collected_at, hostname) VALUES (?1, ?2) RETURNING id",
                rusqlite::params![snapshot.collected_at_epoch, snapshot.hostname],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?
        };
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO sessions (
                      observation_id, session_id, project_id, project_dir, project_name,
                      is_git_repo, branch, git_clean, agent_kind, agent_confidence, state,
                      process_count, last_activity_epoch, root_pid,
                      window_address, window_class, window_title, workspace
                    ) VALUES (
                      ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                )
                .map_err(StoreError::from)?;
            for s in &snapshot.sessions {
                let proj = s.project.as_ref();
                let win = s.window.as_ref();
                stmt.execute(rusqlite::params![
                    observation_id,
                    s.id,
                    proj.map(|p| p.id.as_str()),
                    proj.map(|p| p.dir.as_str()),
                    proj.map(|p| p.name.as_str()),
                    proj.map(|p| i64::from(p.is_git_repo)),
                    proj.and_then(|p| p.branch.as_deref()),
                    proj.and_then(|p| p.git_clean.map(i64::from)),
                    s.agent.kind.as_str(),
                    s.agent.confidence.as_str(),
                    s.state.as_str(),
                    s.process_count as i64,
                    s.last_activity_epoch,
                    i64::from(s.root_pid),
                    win.map(|w| w.address.as_str()),
                    win.map(|w| w.class.as_str()),
                    win.map(|w| w.title.as_str()),
                    win.map(|w| w.workspace.as_str()),
                ])
                .map_err(StoreError::from)?;
            }
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('last_snapshot_hash', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            rusqlite::params![hash],
        )
        .map_err(StoreError::from)?;
        // Bounded cache: keep newest MAX_OBSERVATIONS, drop the rest.
        tx.execute(
            "DELETE FROM sessions WHERE observation_id NOT IN
             (SELECT id FROM observations ORDER BY id DESC LIMIT ?1)",
            rusqlite::params![MAX_OBSERVATIONS],
        )
        .map_err(StoreError::from)?;
        tx.execute(
            "DELETE FROM observations WHERE id NOT IN
             (SELECT id FROM observations ORDER BY id DESC LIMIT ?1)",
            rusqlite::params![MAX_OBSERVATIONS],
        )
        .map_err(StoreError::from)?;
        tx.commit().map_err(StoreError::from)?;
        Ok(PersistOutcome::Written { observation_id })
    }
}

/// Hash over *meaningful* snapshot content: session identity, state,
/// project/git context, agent attribution, counts, activity epoch, window
/// facts. Excludes `collected_at` (timestamp-only changes must not write)
/// and all per-process detail (not persisted, not meaningful at this level).
pub fn meaningful_hash(snapshot: &WorkspaceSnapshot) -> String {
    let mut buf = String::new();
    buf.push_str(&format!("host={}\n", snapshot.hostname));
    for s in &snapshot.sessions {
        buf.push_str(&format!(
            "sess={}|state={}|procs={}|last={}|root={}|",
            s.id,
            s.state.as_str(),
            s.process_count,
            s.last_activity_epoch,
            s.root_pid
        ));
        match &s.project {
            Some(p) => buf.push_str(&format!(
                "proj={}|{}|{}|repo={}|br={:?}|clean={:?}|\n",
                p.id, p.dir, p.name, p.is_git_repo, p.branch, p.git_clean
            )),
            None => buf.push_str("proj=-\n"),
        }
        buf.push_str(&format!(
            "agent={}|{}|\n",
            s.agent.kind.as_str(),
            s.agent.confidence.as_str()
        ));
        match &s.window {
            Some(w) => buf.push_str(&format!(
                "win={}|{}|{}|{}|\n",
                w.address, w.class, w.title, w.workspace
            )),
            None => buf.push_str("win=-\n"),
        }
    }
    format!("fnv:{}", ids::fnv1a_hex(&buf))
}

/// Atomically write bytes to `path` (temp + rename), creating parents.
/// On failure the previous file is untouched — the last good artifact is
/// preserved by construction.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{
        AgentIdentity, AgentKind, Confidence, ProcessInfo, ProcessState, ProjectInfo, SessionState,
        TerminalSession, WorkspaceSnapshot, LAST_ACTIVITY_KIND,
    };
    use crate::platform::WindowInfo;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let n = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("pitwall-m2-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_session(id: &str, state: SessionState) -> TerminalSession {
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
            project: Some(ProjectInfo {
                id: "proj_abc".to_string(),
                dir: "/home/u/Work".to_string(),
                name: "Work".to_string(),
                is_git_repo: true,
                branch: Some("main".to_string()),
                git_clean: Some(true),
            }),
            agent: AgentIdentity {
                kind: AgentKind::Opencode,
                confidence: Confidence::High,
                evidence: vec!["cmd:opencode (pid 11)".to_string()],
            },
            state,
            process_count: 2,
            processes: vec![ProcessInfo {
                pid: 11,
                ppid: 10,
                name: "opencode".to_string(),
                // Deliberately secret-bearing: must NEVER reach SQLite.
                command: "opencode --token hunter2-supersecret --api-key ABC123".to_string(),
                cwd: "/home/u/Work".to_string(),
                state: ProcessState::Running,
                started_at_epoch: 1_700_000_001,
            }],
            last_activity_epoch: 1_700_000_001,
            last_activity_kind: LAST_ACTIVITY_KIND,
            summary: "opencode [high] on Work (main) · running · 2 procs".to_string(),
        }
    }

    fn sample_snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions: vec![sample_session("sess_1", SessionState::Running)],
        }
    }

    #[test]
    fn fresh_database_creates_schema_version_1() {
        let dir = test_dir("fresh");
        let store = Store::open(&dir.join("pitwall.db")).unwrap();
        assert!(!store.recovered_from_corrupt);
        assert_eq!(store.observation_count().unwrap(), 0);
        let v: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, STORE_SCHEMA_VERSION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_and_read_persisted_state() {
        let dir = test_dir("insert");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        let outcome = store.persist(&sample_snapshot()).unwrap();
        let id = match outcome {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("first persist must write"),
        };
        assert_eq!(store.observation_count().unwrap(), 1);
        let (sid, branch, kind, procs): (String, Option<String>, String, i64) = store
            .conn
            .query_row(
                "SELECT session_id, branch, agent_kind, process_count FROM sessions WHERE observation_id=?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(sid, "sess_1");
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(kind, "opencode");
        assert_eq!(procs, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unchanged_snapshot_writes_nothing() {
        let dir = test_dir("unchanged");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        let mut snap = sample_snapshot();
        assert!(matches!(
            store.persist(&snap),
            Ok(PersistOutcome::Written { .. })
        ));
        // Timestamp-only change must not write.
        snap.collected_at_epoch += 3600;
        assert!(matches!(
            store.persist(&snap),
            Ok(PersistOutcome::Unchanged)
        ));
        assert_eq!(store.observation_count().unwrap(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changed_snapshot_creates_new_observation() {
        let dir = test_dir("changed");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        store.persist(&sample_snapshot()).unwrap();
        let mut snap = sample_snapshot();
        snap.sessions[0].state = SessionState::Stopped;
        assert!(matches!(
            store.persist(&snap),
            Ok(PersistOutcome::Written { .. })
        ));
        assert_eq!(store.observation_count().unwrap(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn history_is_pruned_to_bound() {
        let dir = test_dir("prune");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        for i in 0..(MAX_OBSERVATIONS + 5) {
            let mut snap = sample_snapshot();
            // Force a difference each round.
            snap.sessions[0].process_count = 1000 + i as usize;
            snap.sessions[0].last_activity_epoch = 1_700_000_000 + i;
            store.persist(&snap).unwrap();
        }
        assert_eq!(store.observation_count().unwrap(), MAX_OBSERVATIONS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_database_is_quarantined_and_recreated() {
        let dir = test_dir("corrupt");
        let db = dir.join("pitwall.db");
        std::fs::write(&db, b"this is not a database at all").unwrap();
        let store = Store::open(&db).unwrap();
        assert!(store.recovered_from_corrupt);
        assert_eq!(store.observation_count().unwrap(), 0);
        // Aside file exists, original path is a fresh DB.
        let aside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains("corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "quarantined copy must be preserved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_schema_version_refuses_to_open() {
        let dir = test_dir("newer");
        let db = dir.join("pitwall.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(SCHEMA_SQL).unwrap();
            conn.execute_batch(&format!(
                "PRAGMA user_version = {};",
                STORE_SCHEMA_VERSION + 1
            ))
            .unwrap();
        }
        match Store::open(&db) {
            Err(StoreError::NewerVersion(v)) => assert_eq!(v, STORE_SCHEMA_VERSION + 1),
            Err(e) => panic!("expected NewerVersion, got backend error: {e}"),
            Ok(_) => panic!("expected NewerVersion refusal, store opened"),
        }
        // Refusal must not quarantine or modify the file.
        assert!(!db.with_extension("db.corrupt-0").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn permission_failure_degrades_without_quarantine() {
        let dir = test_dir("perms");
        let ro = dir.join("ro");
        std::fs::create_dir_all(&ro).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();
        }
        let result = Store::open(&ro.join("pitwall.db"));
        assert!(result.is_err(), "open in readonly dir must fail");
        assert!(
            matches!(result, Err(StoreError::Backend(_))),
            "must be Backend, never quarantine path"
        );
        let entries: Vec<_> = std::fs::read_dir(&ro).unwrap().collect();
        assert!(entries.is_empty(), "nothing may be created aside");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_cmdline_never_reaches_sqlite() {
        let dir = test_dir("privacy");
        let db = dir.join("pitwall.db");
        let mut store = Store::open(&db).unwrap();
        store.persist(&sample_snapshot()).unwrap();
        drop(store);
        let raw = std::fs::read(&db).unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains("hunter2-supersecret"),
            "argv secret in DB bytes"
        );
        assert!(!text.contains("ABC123"), "argv secret in DB bytes");
        assert!(!text.contains("--token"), "argv flag in DB bytes");
        // …while the benign project context IS stored.
        assert!(text.contains("/home/u/Work"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
