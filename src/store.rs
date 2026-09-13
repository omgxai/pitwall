//! Local continuity cache (M2).
//!
//! SQLite-backed bounded history of workspace observations plus the
//! `state.json` artifact writer. Local-only: one file under the data dir,
//! no network, no sync.
//!
//! Design (see pre-flight review):
//!
//! - tables (`meta`, `observations`, `sessions`, `checkpoints`,
//!   `summaries`), `PRAGMA user_version` schema gating, no migration framework;
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
///
/// v3 adds the `summaries` cache table (M5d) via additive
/// CREATE-IF-NOT-EXISTS: v1/v2 databases keep all existing rows.
/// (The v1→v2 recreate was a one-time pre-release exception; v2 holds
/// user checkpoints, so M5d migrates additively — still no framework.)
pub const STORE_SCHEMA_VERSION: i64 = 4;

/// Bounded-cache policy: keep the newest N observations (and their
/// sessions). One deterministic rule — no time-based second policy.
pub const MAX_OBSERVATIONS: i64 = 100;

/// Checkpoint retention: newest N per project…
pub const MAX_CHECKPOINTS_PER_PROJECT: i64 = 25;
/// …with a hard global cap.
pub const MAX_CHECKPOINTS_TOTAL: i64 = 500;

/// Human-supplied checkpoint notes are capped (a note is a label, not a log).
pub const MAX_NOTE_CHARS: usize = 280;

pub const DB_FILENAME: &str = "pitwall.db";
pub const STATE_FILENAME: &str = "state.json";

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS observations (
  id           INTEGER PRIMARY KEY,
  collected_at INTEGER NOT NULL,
  hostname     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
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
CREATE INDEX IF NOT EXISTS idx_sessions_observation ON sessions (observation_id);
CREATE INDEX IF NOT EXISTS idx_sessions_session ON sessions (session_id);
CREATE TABLE IF NOT EXISTS checkpoints (
  id                  INTEGER PRIMARY KEY,
  created_at          INTEGER NOT NULL,
  project_id          TEXT NOT NULL,
  session_id          TEXT NOT NULL,
  project_dir         TEXT NOT NULL,
  branch              TEXT,
  git_clean           INTEGER,
  agent_kind          TEXT NOT NULL,
  agent_confidence    TEXT NOT NULL,
  state               TEXT NOT NULL,
  last_activity_epoch INTEGER NOT NULL,
  window_address      TEXT,
  window_class        TEXT,
  note                TEXT,
  trigger             TEXT NOT NULL,
  observation_id      INTEGER
);
CREATE INDEX IF NOT EXISTS idx_checkpoints_project ON checkpoints (project_id);
CREATE INDEX IF NOT EXISTS idx_checkpoints_session ON checkpoints (session_id);
CREATE TABLE IF NOT EXISTS summaries (
  input_hash TEXT PRIMARY KEY,
  text       TEXT NOT NULL,
  model      TEXT,
  created_at INTEGER NOT NULL
);
-- v4: human-relevant event inbox. Allowlist columns only (see
-- Notification below); no argv/env/transcript/evidence/titles/notes.
CREATE TABLE IF NOT EXISTS notifications (
  id            INTEGER PRIMARY KEY,
  kind          TEXT NOT NULL,
  session_id    TEXT NOT NULL DEFAULT '',
  project_id    TEXT NOT NULL DEFAULT '',
  project_name  TEXT NOT NULL DEFAULT '',
  branch        TEXT,
  agent_kind    TEXT NOT NULL DEFAULT 'unknown',
  state         TEXT NOT NULL DEFAULT 'unknown',
  checkpoint_id INTEGER,
  created_at    INTEGER NOT NULL,
  read_at       INTEGER,
  severity      TEXT NOT NULL DEFAULT 'informational',
  detail        TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_notifications_unread
  ON notifications (read_at, created_at);
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
        if version < 2 {
            // Ancient pre-release cache (v1 held no user data): recreate.
            conn.execute_batch(
                "DROP TABLE IF EXISTS sessions;
                 DROP TABLE IF EXISTS observations;
                 DROP TABLE IF EXISTS checkpoints;
                 DROP TABLE IF EXISTS summaries;
                 DROP TABLE IF EXISTS notifications;
                 DROP TABLE IF EXISTS meta;",
            )
            .map_err(StoreError::from)?;
            create_schema(&conn).map_err(StoreError::from)?;
        } else if version < STORE_SCHEMA_VERSION {
            // Additive upgrade (v2 -> v3 -> v4): create any missing
            // tables, keep every existing row. No framework, one set.
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

/// Checkpoint trigger: how the checkpoint came to exist. Only these two
/// exist in M4 — no periodic, no transition, no git triggers.
pub mod trigger {
    pub const MANUAL: &str = "manual";
    pub const DISAPPEARANCE: &str = "disappearance";
}

/// One checkpoint row: "what was this workspace doing, and where do I
/// continue?" Never a transcript; never carries command lines (see the
/// privacy note on [`Store::insert_checkpoint`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: i64,
    pub created_at: i64,
    pub project_id: String,
    pub session_id: String,
    pub project_dir: String,
    pub branch: Option<String>,
    pub git_clean: Option<bool>,
    pub agent_kind: String,
    pub agent_confidence: String,
    pub state: String,
    pub last_activity_epoch: i64,
    pub window_address: Option<String>,
    pub window_class: Option<String>,
    pub note: Option<String>,
    pub trigger: String,
    pub observation_id: Option<i64>,
}

/// Last-known session facts for disappearance checkpoints (read back from
/// the sessions table — the live tree is gone by definition).
#[derive(Debug, Clone)]
struct LastKnownSession {
    project_id: Option<String>,
    project_dir: Option<String>,
    branch: Option<String>,
    git_clean: Option<i64>,
    agent_kind: String,
    agent_confidence: String,
    state: String,
    last_activity_epoch: i64,
    window_address: Option<String>,
    window_class: Option<String>,
}

fn truncate_note(note: &str) -> String {
    if note.chars().count() <= MAX_NOTE_CHARS {
        return note.to_string();
    }
    note.chars().take(MAX_NOTE_CHARS).collect()
}

impl Store {
    /// Number of stored checkpoints (bounded by [`MAX_CHECKPOINTS_TOTAL`]).
    pub fn checkpoint_count(&self) -> Result<i64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM checkpoints", [], |row| row.get(0))
            .map_err(StoreError::from)?;
        Ok(n)
    }

    /// Insert one checkpoint row and enforce retention. Returns the row id.
    ///
    /// PRIVACY: callers pass identity/context/state only. There is no
    /// parameter for command lines, environment, output, or transcripts —
    /// the schema has no such columns, so they cannot arrive here.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_checkpoint(
        &mut self,
        created_at: i64,
        project_id: &str,
        session_id: &str,
        project_dir: &str,
        branch: Option<&str>,
        git_clean: Option<bool>,
        agent_kind: &str,
        agent_confidence: &str,
        state: &str,
        last_activity_epoch: i64,
        window_address: Option<&str>,
        window_class: Option<&str>,
        note: Option<&str>,
        trigger: &str,
        observation_id: Option<i64>,
    ) -> Result<i64, StoreError> {
        let note_capped: Option<String> = note.map(truncate_note).filter(|n| !n.is_empty());
        let id: i64 = self
            .conn
            .query_row(
                "INSERT INTO checkpoints (
               created_at, project_id, session_id, project_dir, branch, git_clean,
               agent_kind, agent_confidence, state, last_activity_epoch,
               window_address, window_class, note, trigger, observation_id
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
             RETURNING id",
                rusqlite::params![
                    created_at,
                    project_id,
                    session_id,
                    project_dir,
                    branch,
                    git_clean.map(i64::from),
                    agent_kind,
                    agent_confidence,
                    state,
                    last_activity_epoch,
                    window_address,
                    window_class,
                    note_capped,
                    trigger,
                    observation_id,
                ],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        self.enforce_checkpoint_retention()?;
        Ok(id)
    }

    /// Checkpoint live sessions explicitly (`trigger::MANUAL`). When
    /// `only_session` is set, only that session id is recorded.
    pub fn checkpoint_live(
        &mut self,
        snapshot: &WorkspaceSnapshot,
        only_session: Option<&str>,
        note: Option<&str>,
        now: i64,
    ) -> Result<Vec<i64>, StoreError> {
        let mut ids = Vec::new();
        for s in &snapshot.sessions {
            if let Some(only) = only_session {
                if s.id != only {
                    continue;
                }
            }
            let Some(p) = &s.project else { continue };
            let w = s.window.as_ref();
            let cp_id = self.insert_checkpoint(
                now,
                &p.id,
                &s.id,
                &p.dir,
                p.branch.as_deref(),
                p.git_clean,
                s.agent.kind.as_str(),
                s.agent.confidence.as_str(),
                s.state.as_str(),
                s.last_activity_epoch,
                w.map(|w| w.address.as_str()),
                w.map(|w| w.class.as_str()),
                note,
                trigger::MANUAL,
                None,
            )?;
            ids.push(cp_id);
            // Manual checkpoints are user-recorded context: informational
            // record, never badged. Best-effort; checkpoint stands regardless.
            let _ = self.notify(
                notif_kind::CHECKPOINT,
                &s.id,
                &p.id,
                &p.name,
                p.branch.as_deref(),
                s.agent.kind.as_str(),
                s.state.as_str(),
                Some(cp_id),
                severity::INFORMATIONAL,
                &format!("manual checkpoint on {}", p.name),
                now,
            );
        }
        Ok(ids)
    }

    /// Session ids present in an observation (helper for disappearance math).
    fn observation_session_ids(&self, observation_id: i64) -> Result<Vec<String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id FROM sessions WHERE observation_id = ?1")
            .map_err(StoreError::from)?;
        let ids = stmt
            .query_map(rusqlite::params![observation_id], |row| row.get(0))
            .map_err(StoreError::from)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(StoreError::from)?;
        Ok(ids)
    }

    /// Newest observation id below `below` (i.e. "the previous snapshot").
    fn previous_observation(&self, below: i64) -> Result<Option<(i64, i64)>, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, collected_at FROM observations WHERE id < ?1 ORDER BY id DESC LIMIT 1",
                rusqlite::params![below],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        Ok(row)
    }

    /// Newest disappearance observation for a session, if any.
    fn last_disappearance_checkpoint(&self, session_id: &str) -> Result<Option<i64>, StoreError> {
        let at: Option<i64> = self
            .conn
            .query_row(
                "SELECT MAX(observation_id) FROM checkpoints
                 WHERE session_id = ?1 AND trigger = ?2",
                rusqlite::params![session_id, trigger::DISAPPEARANCE],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(at)
    }

    /// Last-known facts for a session (newest retained sessions row).
    fn last_known_session(&self, session_id: &str) -> Result<Option<LastKnownSession>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT project_id, project_dir,
                        branch, git_clean, agent_kind, agent_confidence, state,
                        last_activity_epoch, window_address, window_class
                 FROM sessions WHERE session_id = ?1 ORDER BY observation_id DESC LIMIT 1",
                rusqlite::params![session_id],
                |row| {
                    Ok(LastKnownSession {
                        project_id: row.get(0)?,
                        project_dir: row.get(1)?,
                        branch: row.get(2)?,
                        git_clean: row.get(3)?,
                        agent_kind: row.get(4)?,
                        agent_confidence: row.get(5)?,
                        state: row.get(6)?,
                        last_activity_epoch: row.get(7)?,
                        window_address: row.get(8)?,
                        window_class: row.get(9)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    /// Checkpoint sessions that vanished between the previous observation
    /// and `current_observation_id` (whose live ids are `live_ids`).
    ///
    /// Suppression: at most one disappearance checkpoint per continuous
    /// absence, keyed by observation order rather than wall-clock seconds.
    /// Reappearance resets this even within one second or after clock rollback.
    /// Returns new checkpoint ids.
    pub fn checkpoint_disappearances(
        &mut self,
        live_ids: &[String],
        current_observation_id: i64,
        now: i64,
    ) -> Result<Vec<i64>, StoreError> {
        let Some((prev_id, _)) = self.previous_observation(current_observation_id)? else {
            return Ok(Vec::new());
        };
        let prev_ids = self.observation_session_ids(prev_id)?;
        let mut created = Vec::new();
        for sid in prev_ids {
            if live_ids.iter().any(|live| live == &sid) {
                continue;
            }
            if let Some(already) = self.last_disappearance_checkpoint(&sid)? {
                if already >= current_observation_id {
                    continue; // same continuous absence — suppress.
                }
            }
            let Some(known) = self.last_known_session(&sid)? else {
                continue;
            };
            let (Some(pid), Some(pdir)) = (known.project_id, known.project_dir) else {
                continue; // projectless sessions carry no resumable context.
            };
            created.push(self.insert_checkpoint(
                now,
                &pid,
                &sid,
                &pdir,
                known.branch.as_deref(),
                known.git_clean.map(|v| v != 0),
                &known.agent_kind,
                &known.agent_confidence,
                &known.state,
                known.last_activity_epoch,
                known.window_address.as_deref(),
                known.window_class.as_deref(),
                None,
                trigger::DISAPPEARANCE,
                Some(current_observation_id),
            )?);
        }
        Ok(created)
    }

    /// Newest checkpoints first (for state.json `resumable` and inspection).
    pub fn latest_checkpoints(&self, limit: i64) -> Result<Vec<Checkpoint>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, created_at, project_id, session_id, project_dir, branch,
                        git_clean, agent_kind, agent_confidence, state,
                        last_activity_epoch, window_address, window_class,
                        note, trigger, observation_id
                 FROM checkpoints ORDER BY created_at DESC, id DESC LIMIT ?1",
            )
            .map_err(StoreError::from)?;
        let rows = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(Checkpoint {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    project_id: row.get(2)?,
                    session_id: row.get(3)?,
                    project_dir: row.get(4)?,
                    branch: row.get(5)?,
                    git_clean: row.get(6)?,
                    agent_kind: row.get(7)?,
                    agent_confidence: row.get(8)?,
                    state: row.get(9)?,
                    last_activity_epoch: row.get(10)?,
                    window_address: row.get(11)?,
                    window_class: row.get(12)?,
                    note: row.get(13)?,
                    trigger: row.get(14)?,
                    observation_id: row.get(15)?,
                })
            })
            .map_err(StoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        Ok(rows)
    }

    /// Newest checkpoint for one session, if any (Resume lookup).
    pub fn latest_checkpoint_for_session(
        &self,
        session_id: &str,
    ) -> Result<Option<Checkpoint>, StoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT id, created_at, project_id, session_id, project_dir, branch,
                        git_clean, agent_kind, agent_confidence, state,
                        last_activity_epoch, window_address, window_class,
                        note, trigger, observation_id
                 FROM checkpoints WHERE session_id = ?1
                 ORDER BY created_at DESC, id DESC LIMIT 1",
                rusqlite::params![session_id],
                |row| {
                    Ok(Checkpoint {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        project_id: row.get(2)?,
                        session_id: row.get(3)?,
                        project_dir: row.get(4)?,
                        branch: row.get(5)?,
                        git_clean: row.get(6)?,
                        agent_kind: row.get(7)?,
                        agent_confidence: row.get(8)?,
                        state: row.get(9)?,
                        last_activity_epoch: row.get(10)?,
                        window_address: row.get(11)?,
                        window_class: row.get(12)?,
                        note: row.get(13)?,
                        trigger: row.get(14)?,
                        observation_id: row.get(15)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }
}

/// One cached AI summary: the final interpretation text plus minimal
/// metadata. The summaries table NEVER carries terminal text, transcripts,
/// argv, or context — only what the agent returned, already scrubbed
/// upstream by construction (context was the only carrier, and it is
/// ephemeral). Enforced by the column set itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryRow {
    pub input_hash: String,
    pub text: String,
    pub model: Option<String>,
    pub created_at: i64,
}

impl Store {
    /// Look up a cached summary by input hash. `None` means generate.
    pub fn lookup_summary(&self, input_hash: &str) -> Result<Option<SummaryRow>, StoreError> {
        let row: Option<SummaryRow> = self
            .conn
            .query_row(
                "SELECT input_hash, text, model, created_at FROM summaries WHERE input_hash = ?1",
                rusqlite::params![input_hash],
                |row| {
                    Ok(SummaryRow {
                        input_hash: row.get(0)?,
                        text: row.get(1)?,
                        model: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    /// Store (or replace) the summary for an input hash. Only final text +
    /// minimal metadata — callers cannot pass anything else (no params).
    pub fn store_summary(
        &self,
        input_hash: &str,
        text: &str,
        model: Option<&str>,
        created_at: i64,
    ) -> Result<(), StoreError> {
        self.conn
            .execute(
                "INSERT INTO summaries (input_hash, text, model, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(input_hash) DO UPDATE SET
                   text = excluded.text, model = excluded.model,
                   created_at = excluded.created_at",
                rusqlite::params![input_hash, text, model, created_at],
            )
            .map_err(StoreError::from)?;
        Ok(())
    }

    /// Newest cached summary overall (state.json display contract).
    pub fn latest_summary(&self) -> Result<Option<SummaryRow>, StoreError> {
        let row: Option<SummaryRow> = self
            .conn
            .query_row(
                "SELECT input_hash, text, model, created_at FROM summaries
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok(SummaryRow {
                        input_hash: row.get(0)?,
                        text: row.get(1)?,
                        model: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .ok();
        Ok(row)
    }

    /// Delete all cached summaries (user-initiated clear). Checkpoints,
    /// observations, and sessions are untouched — only the interpretation
    /// cache is dropped. Returns rows removed.
    pub fn clear_summaries(&self) -> Result<i64, StoreError> {
        let n = self
            .conn
            .execute("DELETE FROM summaries", [])
            .map_err(StoreError::from)?;
        Ok(n as i64)
    }

    /// Number of cached summaries (bounded in practice by hash cardinality;
    /// one row per distinct workspace context).
    pub fn summary_count(&self) -> Result<i64, StoreError> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM summaries", [], |row| row.get(0))
            .map_err(StoreError::from)?;
        Ok(n)
    }

    /// Insert or refresh an unread notification (dedup: one unread row per
    /// kind+session; a repeat refreshes created_at/detail in place instead
    /// of stacking duplicates). Prunes to the newest MAX_NOTIFICATIONS rows
    /// on every insert. Returns the row id.
    ///
    /// PRIVACY: allowlist columns only (see schema). Callers pass short
    /// scrubbed detail text; there is no parameter for argv, env,
    /// transcripts, evidence, titles, or notes.
    #[allow(clippy::too_many_arguments)]
    pub fn notify(
        &mut self,
        kind: &str,
        session_id: &str,
        project_id: &str,
        project_name: &str,
        branch: Option<&str>,
        agent_kind: &str,
        state: &str,
        checkpoint_id: Option<i64>,
        severity: &str,
        detail: &str,
        created_at: i64,
    ) -> Result<i64, StoreError> {
        if !notif_kind::valid(kind) {
            return Err(StoreError::Backend(format!(
                "refusing unknown notification kind {kind:?}"
            )));
        }
        if !severity::valid(severity) {
            return Err(StoreError::Backend(format!(
                "refusing unknown severity {severity:?}"
            )));
        }
        let detail = truncate_detail(detail);
        if let Some(existing) = self.unread_notification(kind, session_id)? {
            self.conn
                .execute(
                    "UPDATE notifications SET created_at = ?1, detail = ?2,
                     severity = ?3, project_name = ?4, project_id = ?6,
                     branch = ?7, agent_kind = ?8, state = ?9, checkpoint_id = ?10
                     WHERE id = ?5",
                    rusqlite::params![
                        created_at,
                        detail,
                        severity,
                        project_name,
                        existing,
                        project_id,
                        branch,
                        agent_kind,
                        state,
                        checkpoint_id
                    ],
                )
                .map_err(StoreError::from)?;
            return Ok(existing);
        }
        self.insert_notification_row(
            kind,
            session_id,
            project_id,
            project_name,
            branch,
            agent_kind,
            state,
            checkpoint_id,
            severity,
            &detail,
            created_at,
        )
    }

    /// Raw insert without dedup (callers that already checked, or bulk
    /// paths). Prefer [`Store::notify`] unless suppression is handled
    /// by the caller.
    #[allow(clippy::too_many_arguments)]
    fn insert_notification_row(
        &self,
        kind: &str,
        session_id: &str,
        project_id: &str,
        project_name: &str,
        branch: Option<&str>,
        agent_kind: &str,
        state: &str,
        checkpoint_id: Option<i64>,
        severity: &str,
        detail: &str,
        created_at: i64,
    ) -> Result<i64, StoreError> {
        if !notif_kind::valid(kind) {
            return Err(StoreError::Backend(format!(
                "refusing unknown notification kind {kind:?}"
            )));
        }
        if !severity::valid(severity) {
            return Err(StoreError::Backend(format!(
                "refusing unknown severity {severity:?}"
            )));
        }
        let id: i64 = self
            .conn
            .query_row(
                "INSERT INTO notifications (kind, session_id, project_id, project_name,
               branch, agent_kind, state, checkpoint_id, created_at, read_at,
               severity, detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?10,?11)
             RETURNING id",
                rusqlite::params![
                    kind,
                    session_id,
                    project_id,
                    project_name,
                    branch,
                    agent_kind,
                    state,
                    checkpoint_id,
                    created_at,
                    severity,
                    detail,
                ],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        self.enforce_notification_retention()?;
        Ok(id)
    }

    /// Existing unread row id for a kind+session pair, if any.
    pub fn unread_notification(
        &self,
        kind: &str,
        session_id: &str,
    ) -> Result<Option<i64>, StoreError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM notifications
                 WHERE kind = ?1 AND session_id = ?2 AND read_at IS NULL
                 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![kind, session_id],
                |row| row.get(0),
            )
            .ok();
        Ok(id)
    }

    /// Recent notifications regardless of read state (CLI history view).
    pub fn recent_notifications(&self, limit: i64) -> Result<Vec<Notification>, StoreError> {
        self.list_notifications(false, limit)
    }

    /// Unread notifications, newest first (panel inbox + badge source).
    pub fn unread_notifications(&self, limit: i64) -> Result<Vec<Notification>, StoreError> {
        self.list_notifications(true, limit)
    }

    /// Badge count: unread attention + completion rows only. Informational
    /// rows are listed, never badged.
    pub fn unread_badge_count(&self) -> Result<i64, StoreError> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM notifications
                 WHERE read_at IS NULL AND severity IN ('attention','completion')",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(n)
    }

    fn list_notifications(
        &self,
        unread_only: bool,
        limit: i64,
    ) -> Result<Vec<Notification>, StoreError> {
        let sql = if unread_only {
            "SELECT id, kind, session_id, project_id, project_name, branch,
                    agent_kind, state, checkpoint_id, created_at, read_at,
                    severity, detail FROM notifications
             WHERE read_at IS NULL ORDER BY created_at DESC, id DESC LIMIT ?1"
        } else {
            "SELECT id, kind, session_id, project_id, project_name, branch,
                    agent_kind, state, checkpoint_id, created_at, read_at,
                    severity, detail FROM notifications
             ORDER BY created_at DESC, id DESC LIMIT ?1"
        };
        let mut stmt = self.conn.prepare(sql).map_err(StoreError::from)?;
        let rows = stmt
            .query_map(rusqlite::params![limit], |row| {
                Ok(Notification {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    session_id: row.get(2)?,
                    project_id: row.get(3)?,
                    project_name: row.get(4)?,
                    branch: row.get(5)?,
                    agent_kind: row.get(6)?,
                    state: row.get(7)?,
                    checkpoint_id: row.get(8)?,
                    created_at: row.get(9)?,
                    read_at: row.get(10)?,
                    severity: row.get(11)?,
                    detail: row.get(12)?,
                })
            })
            .map_err(StoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        Ok(rows)
    }

    /// Mark one notification read. Returns true when a row flipped.
    /// Reading is explicit (detail click) — rendering a list never marks.
    pub fn mark_notification_read(&self, id: i64, now: i64) -> Result<bool, StoreError> {
        let n = self
            .conn
            .execute(
                "UPDATE notifications SET read_at = ?1
                 WHERE id = ?2 AND read_at IS NULL",
                rusqlite::params![now, id],
            )
            .map_err(StoreError::from)?;
        Ok(n > 0)
    }

    /// Derive inbox rows for one snapshot transition (called after persist
    /// writes the current observation). Pure diff, bounded output:
    /// appeared/vanished informational, newly-stopped attention. Dedup
    /// inside notify() keeps one unread per kind+session. The previous
    /// snapshot must be the immediately preceding observation: the diff
    /// suppresses steady states, while new transitions refresh unread rows.
    /// Returns new-or-refreshed row count. Never fails the snapshot.
    pub fn sync_snapshot_notifications(
        &mut self,
        prev: &[PrevSession],
        curr: &crate::collector::WorkspaceSnapshot,
        now: i64,
    ) -> Result<usize, StoreError> {
        let prev_by_id: std::collections::HashMap<&str, &PrevSession> =
            prev.iter().map(|p| (p.session_id.as_str(), p)).collect();
        let curr_ids: std::collections::HashSet<&str> =
            curr.sessions.iter().map(|s| s.id.as_str()).collect();
        let mut fired = 0;
        let fire_if_new = |store: &mut Store,
                           kind: &str,
                           session_id: &str,
                           project_id: &str,
                           project_name: &str,
                           branch: Option<&str>,
                           agent_kind: &str,
                           state: &str,
                           severity: &str,
                           detail: &str,
                           fired: &mut usize|
         -> Result<(), StoreError> {
            store.notify(
                kind,
                session_id,
                project_id,
                project_name,
                branch,
                agent_kind,
                state,
                None,
                severity,
                detail,
                now,
            )?;
            *fired += 1;
            Ok(())
        };
        for s in &curr.sessions {
            let project_name = s
                .project
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "session".to_string());
            let project_id = s.project.as_ref().map(|p| p.id.clone()).unwrap_or_default();
            let branch = s.project.as_ref().and_then(|p| p.branch.clone());
            match prev_by_id.get(s.id.as_str()) {
                None => {
                    fire_if_new(
                        self,
                        notif_kind::APPEARED,
                        &s.id,
                        &project_id,
                        &project_name,
                        branch.as_deref(),
                        s.agent.kind.as_str(),
                        s.state.as_str(),
                        severity::INFORMATIONAL,
                        &format!(
                            "{} on {} \u{00b7} {}",
                            s.agent.kind.as_str(),
                            project_name,
                            s.state.as_str()
                        ),
                        &mut fired,
                    )?;
                }
                Some(p) => {
                    if s.state.as_str() == "stopped" && p.state != "stopped" {
                        fire_if_new(
                            self,
                            notif_kind::STOPPED,
                            &s.id,
                            &project_id,
                            &project_name,
                            branch.as_deref(),
                            s.agent.kind.as_str(),
                            s.state.as_str(),
                            severity::ATTENTION,
                            &format!("{} {} \u{2192} stopped", s.agent.kind.as_str(), p.state),
                            &mut fired,
                        )?;
                    }
                }
            }
        }
        for p in prev {
            if curr_ids.contains(p.session_id.as_str()) {
                continue;
            }
            let project_name = p
                .project_dir
                .as_ref()
                .and_then(|d| d.rsplit('/').next())
                .filter(|s| !s.is_empty())
                .unwrap_or("session")
                .to_string();
            fire_if_new(
                self,
                notif_kind::VANISHED,
                &p.session_id,
                p.project_id.as_deref().unwrap_or_default(),
                &project_name,
                p.branch.as_deref(),
                &p.agent_kind,
                &p.state,
                severity::INFORMATIONAL,
                &format!("was {} {}", p.agent_kind, p.state),
                &mut fired,
            )?;
        }
        Ok(fired)
    }

    /// Bounded inbox: newest MAX_NOTIFICATIONS rows survive, oldest pruned.
    fn enforce_notification_retention(&self) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM notifications WHERE id NOT IN (
                   SELECT id FROM notifications ORDER BY created_at DESC, id DESC LIMIT ?1
                 )",
                rusqlite::params![MAX_NOTIFICATIONS],
            )
            .map_err(StoreError::from)?;
        Ok(())
    }
}

/// Closed notification vocabulary. Anything else is rejected at insert
/// (see [`Store::notify`]) — the inbox cannot become a generic log.
pub mod notif_kind {
    pub const APPEARED: &str = "appeared";
    pub const VANISHED: &str = "vanished";
    pub const STOPPED: &str = "stopped";
    pub const CHECKPOINT: &str = "checkpoint";
    pub const ASSIGN_DONE: &str = "assign-done";
    pub const ASSIGN_FAILED: &str = "assign-failed";

    pub fn valid(kind: &str) -> bool {
        matches!(
            kind,
            APPEARED | VANISHED | STOPPED | CHECKPOINT | ASSIGN_DONE | ASSIGN_FAILED
        )
    }
}

/// Severity: only attention/completion feed the badge. Informational rows
/// are listed, never badged.
pub mod severity {
    pub const ATTENTION: &str = "attention";
    pub const COMPLETION: &str = "completion";
    pub const INFORMATIONAL: &str = "informational";

    pub fn valid(severity: &str) -> bool {
        matches!(severity, ATTENTION | COMPLETION | INFORMATIONAL)
    }
}

/// Maximum inbox rows (newest win; enforced on every insert).
pub const MAX_NOTIFICATIONS: i64 = 100;

/// Cap on persisted notification detail text (short human sentence).
pub const MAX_NOTIFICATION_DETAIL_CHARS: usize = 140;

fn truncate_detail(detail: &str) -> String {
    let clean: String = detail.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= MAX_NOTIFICATION_DETAIL_CHARS {
        return clean;
    }
    let mut out: String = clean.chars().take(MAX_NOTIFICATION_DETAIL_CHARS).collect();
    out.push('\u{2026}');
    out
}

/// One inbox row. Allowlist shape: identity refs + short scrubbed detail.
/// No argv/env/transcript/evidence/titles/notes/pids by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub id: i64,
    pub kind: String,
    pub session_id: String,
    pub project_id: String,
    pub project_name: String,
    pub branch: Option<String>,
    pub agent_kind: String,
    pub state: String,
    pub checkpoint_id: Option<i64>,
    pub created_at: i64,
    pub read_at: Option<i64>,
    pub severity: String,
    pub detail: String,
}

/// Previous-observation session facts for read-time event derivation.
/// Shape mirrors the sessions table; mapping to display events lives in
/// the context module (no new tables by design).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrevSession {
    pub session_id: String,
    pub project_id: Option<String>,
    pub project_dir: Option<String>,
    pub agent_kind: String,
    pub branch: Option<String>,
    pub git_clean: Option<bool>,
    pub state: String,
}

impl Store {
    /// First-seen epoch for a session across retained observations
    /// (observed duration basis for timeline bars). `None` when the session
    /// appears only in no retained observation (e.g. brand-new live data).
    pub fn session_first_seen(&self, session_id: &str) -> Result<Option<i64>, StoreError> {
        let at: Option<i64> = self
            .conn
            .query_row(
                "SELECT MIN(o.collected_at) FROM observations o
                 JOIN sessions s ON s.observation_id = o.id
                 WHERE s.session_id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            )
            .map_err(StoreError::from)?;
        Ok(at)
    }

    /// Recent observed states for a session, oldest→newest, one char per
    /// retained observation (cap `limit`): R running, S sleeping/stopped
    /// (non-running but present), U unknown, . absent-from-sample is NOT
    /// emitted (only observations containing the session are sampled, so
    /// every char is real evidence, never filler).
    pub fn session_history(&self, session_id: &str, limit: i64) -> Result<String, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.state FROM sessions s
                 JOIN observations o ON o.id = s.observation_id
                 WHERE s.session_id = ?1
                 ORDER BY o.id DESC LIMIT ?2",
            )
            .map_err(StoreError::from)?;
        let mut states: Vec<String> = stmt
            .query_map(rusqlite::params![session_id, limit], |row| row.get(0))
            .map_err(StoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        states.reverse();
        let out: String = states
            .iter()
            .map(|s| match s.as_str() {
                "running" => 'R',
                "sleeping" | "stopped" => 'S',
                _ => 'U',
            })
            .collect();
        Ok(out)
    }

    /// Newest retained observation `(id, collected_at)`, if any.
    pub fn latest_observation(&self) -> Result<Option<(i64, i64)>, StoreError> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, collected_at FROM observations ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        Ok(row)
    }

    /// Session facts for one observation (event derivation input).
    pub fn observation_sessions(
        &self,
        observation_id: i64,
    ) -> Result<Vec<PrevSession>, StoreError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT session_id, project_id, project_dir, agent_kind, branch,
                        git_clean, state FROM sessions WHERE observation_id = ?1",
            )
            .map_err(StoreError::from)?;
        let rows = stmt
            .query_map(rusqlite::params![observation_id], |row| {
                Ok(PrevSession {
                    session_id: row.get(0)?,
                    project_id: row.get(1)?,
                    project_dir: row.get(2)?,
                    agent_kind: row.get(3)?,
                    branch: row.get(4)?,
                    git_clean: row.get::<_, Option<i64>>(5)?.map(|v| v != 0),
                    state: row.get(6)?,
                })
            })
            .map_err(StoreError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        Ok(rows)
    }

    /// Checkpoints created after `since_epoch` (event derivation input).
    pub fn checkpoints_since(
        &self,
        since_epoch: i64,
        limit: i64,
    ) -> Result<Vec<Checkpoint>, StoreError> {
        let mut all = self.latest_checkpoints(limit)?;
        all.retain(|cp| cp.created_at > since_epoch);
        Ok(all)
    }

    /// Bounded retention: newest N per project + hard global cap.
    fn enforce_checkpoint_retention(&self) -> Result<(), StoreError> {
        self.conn
            .execute(
                "DELETE FROM checkpoints WHERE id NOT IN (
                   SELECT id FROM (
                     SELECT id, ROW_NUMBER() OVER (
                       PARTITION BY project_id ORDER BY created_at DESC, id DESC
                     ) AS rn FROM checkpoints
                   ) WHERE rn <= ?1
                 )",
                rusqlite::params![MAX_CHECKPOINTS_PER_PROJECT],
            )
            .map_err(StoreError::from)?;
        self.conn
            .execute(
                "DELETE FROM checkpoints WHERE id NOT IN (
                   SELECT id FROM checkpoints ORDER BY created_at DESC, id DESC LIMIT ?1
                 )",
                rusqlite::params![MAX_CHECKPOINTS_TOTAL],
            )
            .map_err(StoreError::from)?;
        Ok(())
    }
}

/// Atomically write bytes to `path` (temp + rename), creating parents.
/// On failure the previous file is untouched — the last good artifact is
/// preserved by construction.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    // Exclusive, per-writer siblings prevent truncating another writer's
    // pending file (or following a pre-existing temporary symlink).
    let (tmp, mut file) = loop {
        let tmp = path.with_extension(format!(
            "tmp-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        file.write_all(bytes)?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::{
        AgentIdentity, AgentKind, Confidence, ProcessInfo, ProcessState, ProjectInfo, SessionState,
        TerminalSession, WindowRole, WorkspaceSnapshot, LAST_ACTIVITY_KIND,
    };
    use crate::platform::WindowInfo;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_dir(name: &str) -> PathBuf {
        let n = TEST_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("pitwall-m2-{name}-{}-{n}", std::process::id()));
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
            role: WindowRole::Terminal,
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
                exe_name: "opencode".to_string(),
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
    fn atomic_writers_publish_complete_independent_files() {
        let dir = test_dir("atomic-concurrent");
        let path = dir.join("state.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..10 {
                        atomic_write(&path, &vec![b'a' + i; 65536]).unwrap();
                        let bytes = std::fs::read(&path).unwrap();
                        assert_eq!(bytes.len(), 65536);
                        assert!(bytes.iter().all(|b| *b == bytes[0]));
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn atomic_failed_rename_cleans_temporary_file() {
        let dir = test_dir("atomic-failure");
        let path = dir.join("state.json");
        std::fs::create_dir(&path).unwrap();
        assert!(atomic_write(&path, b"new").is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn new_stop_transition_refreshes_unread_context() {
        let dir = test_dir("notif-refire");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        let mut snap = sample_snapshot();
        store.persist(&snap).unwrap();
        let prev = store
            .observation_sessions(store.latest_observation().unwrap().unwrap().0)
            .unwrap();
        snap.sessions[0].state = SessionState::Stopped;
        assert_eq!(
            store
                .sync_snapshot_notifications(&prev, &snap, 100)
                .unwrap(),
            1
        );
        let id = store.unread_notifications(10).unwrap()[0].id;
        // A later running -> stopped transition with changed project context.
        snap.sessions[0].project.as_mut().unwrap().branch = Some("topic".into());
        assert_eq!(
            store
                .sync_snapshot_notifications(&prev, &snap, 200)
                .unwrap(),
            1
        );
        let rows = store.unread_notifications(10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].created_at, 200);
        assert_eq!(rows[0].branch.as_deref(), Some("topic"));
        std::fs::remove_dir_all(dir).unwrap();
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

    fn snap_with_sessions(ids: &[&str]) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 1_700_000_100,
            hostname: "testbox".to_string(),
            sessions: ids
                .iter()
                .map(|id| {
                    let mut s = sample_session(id, SessionState::Running);
                    s.id = id.to_string();
                    s
                })
                .collect(),
        }
    }

    fn live_ids(snapshot: &WorkspaceSnapshot) -> Vec<String> {
        snapshot.sessions.iter().map(|s| s.id.clone()).collect()
    }

    #[test]
    fn manual_checkpoint_records_live_session() {
        let dir = test_dir("cp-manual");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        let snap = sample_snapshot();
        let ids = store
            .checkpoint_live(&snap, None, Some("before lunch"), 1_700_000_200)
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(store.checkpoint_count().unwrap(), 1);
        let cps = store.latest_checkpoints(10).unwrap();
        assert_eq!(cps.len(), 1);
        let cp = &cps[0];
        assert_eq!(cp.session_id, "sess_1");
        assert_eq!(cp.project_dir, "/home/u/Work");
        assert_eq!(cp.branch.as_deref(), Some("main"));
        assert_eq!(cp.trigger, trigger::MANUAL);
        assert_eq!(cp.note.as_deref(), Some("before lunch"));
        assert_eq!(cp.observation_id, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manual_checkpoint_scopes_to_session_and_caps_note() {
        let dir = test_dir("cp-scope");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        let snap = snap_with_sessions(&["sess_a", "sess_b"]);
        let long_note = "n".repeat(500);
        let ids = store
            .checkpoint_live(&snap, Some("sess_b"), Some(&long_note), 1_700_000_200)
            .unwrap();
        assert_eq!(ids.len(), 1);
        let cps = store.latest_checkpoints(10).unwrap();
        assert_eq!(cps[0].session_id, "sess_b");
        assert_eq!(
            cps[0].note.as_ref().unwrap().chars().count(),
            MAX_NOTE_CHARS
        );
        // Unknown session id records nothing.
        let none = store
            .checkpoint_live(&snap, Some("sess_zzz"), None, 1_700_000_200)
            .unwrap();
        assert!(none.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disappearance_fires_once_then_suppresses_then_refires() {
        let dir = test_dir("cp-disappear");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        // Obs 1: sess_1 live.
        let snap_a = snap_with_sessions(&["sess_1"]);
        let obs1 = match store.persist(&snap_a).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        // Obs 2: sess_1 gone, sess_2 live → one disappearance checkpoint.
        let mut snap_b = snap_with_sessions(&["sess_2"]);
        snap_b.sessions[0].process_count = 7; // force difference
        let obs2 = match store.persist(&snap_b).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        assert!(obs2 > obs1);
        let fired = store
            .checkpoint_disappearances(&live_ids(&snap_b), obs2, 1_700_000_300)
            .unwrap();
        assert_eq!(fired.len(), 1);
        let cps = store.latest_checkpoints(10).unwrap();
        assert_eq!(cps[0].session_id, "sess_1");
        assert_eq!(cps[0].trigger, trigger::DISAPPEARANCE);
        assert_eq!(cps[0].observation_id, Some(obs2));
        // Same absence again → suppressed.
        let again = store
            .checkpoint_disappearances(&live_ids(&snap_b), obs2, 1_700_000_400)
            .unwrap();
        assert!(again.is_empty());
        assert_eq!(store.checkpoint_count().unwrap(), 1);
        // Reappearance then re-vanishing fires even with clock rollback.
        let mut snap_c = snap_with_sessions(&["sess_1", "sess_2"]);
        snap_c.collected_at_epoch = 1_700_000_000;
        snap_c.sessions[0].process_count = 9;
        let obs3 = match store.persist(&snap_c).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        let none = store
            .checkpoint_disappearances(&live_ids(&snap_c), obs3, 1_700_000_500)
            .unwrap();
        assert!(none.is_empty(), "nothing vanished");
        let mut snap_d = snap_with_sessions(&["sess_2"]);
        snap_d.sessions[0].process_count = 11;
        let obs4 = match store.persist(&snap_d).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        let refired = store
            .checkpoint_disappearances(&live_ids(&snap_d), obs4, 1_700_000_600)
            .unwrap();
        assert_eq!(refired.len(), 1);
        assert_eq!(store.checkpoint_count().unwrap(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_retention_per_project_and_global() {
        let dir = test_dir("cp-retain");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        for i in 0..30 {
            let mut snap = snap_with_sessions(&["sess_p"]);
            snap.sessions[0].project.as_mut().unwrap().id = "proj_same".to_string();
            snap.sessions[0].id = format!("sess_p{i}");
            store
                .checkpoint_live(&snap, None, None, 1_700_000_000 + i64::from(i))
                .unwrap();
        }
        assert_eq!(
            store.checkpoint_count().unwrap(),
            MAX_CHECKPOINTS_PER_PROJECT
        );
        // Flood 30 distinct projects x 20 (all under the per-project cap):
        // the global cap must still hold at 500.
        for i in 0..600 {
            let mut snap = snap_with_sessions(&["sess_g"]);
            let pid = format!("proj_flood{}", i % 30);
            snap.sessions[0].project.as_mut().unwrap().id = pid;
            snap.sessions[0].id = format!("sess_g{i}");
            store
                .checkpoint_live(&snap, None, None, 1_800_000_000 + i64::from(i))
                .unwrap();
        }
        assert_eq!(store.checkpoint_count().unwrap(), MAX_CHECKPOINTS_TOTAL);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_rows_carry_no_cmdline() {
        let dir = test_dir("cp-privacy");
        let db = dir.join("pitwall.db");
        let mut store = Store::open(&db).unwrap();
        store
            .checkpoint_live(
                &sample_snapshot(),
                None,
                Some("note hunter2? no—notes are labels"),
                1_700_000_200,
            )
            .unwrap();
        drop(store);
        let raw = std::fs::read(&db).unwrap();
        let text = String::from_utf8_lossy(&raw);
        // The evil fixture command must not be stored…
        assert!(!text.contains("hunter2-supersecret"));
        assert!(!text.contains("ABC123"));
        // …and even a secret typed into a *note* is capped, though notes are
        // human-supplied labels (documented boundary, not silently executed).
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_checkpoint_row_does_not_break_reads() {
        let dir = test_dir("cp-corrupt-row");
        let db = dir.join("pitwall.db");
        let mut store = Store::open(&db).unwrap();
        store
            .checkpoint_live(&sample_snapshot(), None, None, 1_700_000_200)
            .unwrap();
        // Inject garbage directly: impossible state string, NULL project.
        store
            .conn
            .execute(
                "INSERT INTO checkpoints (created_at, project_id, session_id, project_dir,
                  agent_kind, agent_confidence, state, last_activity_epoch, trigger)
                 VALUES (1700000300, 'proj_x', 'sess_x', '/tmp/x', 'weird-agent', 'maybe',
                         'flying', 1, 'manual')",
                [],
            )
            .unwrap();
        let cps = store.latest_checkpoints(10).unwrap();
        assert_eq!(cps.len(), 2, "reader tolerates unexpected row content");
        assert_eq!(cps[0].session_id, "sess_x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summaries_round_trip_and_latest_wins() {
        let dir = test_dir("sum-roundtrip");
        let store = Store::open(&dir.join("pitwall.db")).unwrap();
        assert_eq!(store.summary_count().unwrap(), 0);
        assert_eq!(store.latest_summary().unwrap(), None);
        assert_eq!(store.lookup_summary("fnv:aaa").unwrap(), None);
        store
            .store_summary("fnv:aaa", "First.", Some("prov/m"), 100)
            .unwrap();
        store
            .store_summary("fnv:bbb", "Second.", None, 200)
            .unwrap();
        assert_eq!(store.summary_count().unwrap(), 2);
        let got = store.lookup_summary("fnv:aaa").unwrap().unwrap();
        assert_eq!(got.text, "First.");
        assert_eq!(got.model.as_deref(), Some("prov/m"));
        let latest = store.latest_summary().unwrap().unwrap();
        assert_eq!(latest.input_hash, "fnv:bbb");
        // Replace semantics: same hash overwrites, count unchanged.
        store
            .store_summary("fnv:aaa", "First v2.", Some("prov/m"), 300)
            .unwrap();
        assert_eq!(store.summary_count().unwrap(), 2);
        assert_eq!(
            store.lookup_summary("fnv:aaa").unwrap().unwrap().text,
            "First v2."
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn v2_database_upgrades_additively_preserving_rows() {
        use rusqlite::Connection;
        let dir = test_dir("sum-migrate");
        let db = dir.join("pitwall.db");
        // Hand-build a v2-shaped database (no summaries table).
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                 CREATE TABLE observations (id INTEGER PRIMARY KEY, collected_at INTEGER, hostname TEXT);
                 CREATE TABLE sessions (observation_id INTEGER, session_id TEXT);
                 INSERT INTO meta (key, value) VALUES ('k', 'v');
                 INSERT INTO observations (id, collected_at, hostname) VALUES (1, 100, 'h');
                 INSERT INTO sessions (observation_id, session_id) VALUES (1, 'sess_1');
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let store = Store::open(&db).unwrap();
        // Old rows survive; new table exists and works.
        assert_eq!(store.observation_count().unwrap(), 1);
        let v: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, STORE_SCHEMA_VERSION);
        // Write + read a summary through the upgraded handle.
        store.store_summary("fnv:x", "Kept.", None, 400).unwrap();
        assert_eq!(store.latest_summary().unwrap().unwrap().text, "Kept.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summaries_table_carries_no_evidence_columns() {
        let dir = test_dir("sum-privacy");
        let db = dir.join("pitwall.db");
        let store = Store::open(&db).unwrap();
        store
            .store_summary("fnv:x", "Note hunter2 was here.", None, 1)
            .unwrap();
        drop(store);
        // Column set is exactly the approved four.
        let db2 = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = db2
            .prepare("SELECT name FROM pragma_table_info('summaries') ORDER BY cid")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(cols, vec!["input_hash", "text", "model", "created_at"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_first_seen_and_history() {
        let dir = test_dir("hist");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        assert_eq!(store.session_first_seen("sess_nope").unwrap(), None);
        assert_eq!(store.session_history("sess_nope", 16).unwrap(), "");
        // Two observations with known states (bypass collect via persist).
        let mut snap1 = sample_snapshot();
        snap1.collected_at_epoch = 1000;
        snap1.sessions[0].id = "sess_h".to_string();
        snap1.sessions[0].state = SessionState::Sleeping;
        let obs1 = match store.persist(&snap1).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        let mut snap2 = sample_snapshot();
        snap2.collected_at_epoch = 2000;
        snap2.sessions[0].id = "sess_h".to_string();
        snap2.sessions[0].state = SessionState::Running;
        snap2.sessions[0].process_count = 5;
        let _ = match store.persist(&snap2).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        assert_eq!(store.session_first_seen("sess_h").unwrap(), Some(1000));
        assert_eq!(store.session_history("sess_h", 16).unwrap(), "SR");
        assert_eq!(store.session_history("sess_h", 1).unwrap(), "R");
        let _ = obs1;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summaries_clear_removes_only_summaries() {
        let dir = test_dir("sum-clear");
        let store = Store::open(&dir.join("pitwall.db")).unwrap();
        store.store_summary("fnv:a", "A.", None, 1).unwrap();
        store.store_summary("fnv:b", "B.", None, 2).unwrap();
        assert_eq!(store.summary_count().unwrap(), 2);
        assert_eq!(store.clear_summaries().unwrap(), 2);
        assert_eq!(store.summary_count().unwrap(), 0);
        assert_eq!(store.latest_summary().unwrap(), None);
        // Other tables untouched (meta probe).
        let tables: Vec<String> = store
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.contains(&"checkpoints".to_string()));
        assert!(tables.contains(&"observations".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn notif(store: &mut Store, kind: &str, session: &str, severity: &str, at: i64) -> i64 {
        store
            .notify(
                kind,
                session,
                "proj_x",
                "Work",
                Some("main"),
                "opencode",
                "running",
                None,
                severity,
                "detail text",
                at,
            )
            .unwrap()
    }

    #[test]
    fn notifications_insert_dedup_read_and_prune() {
        let dir = test_dir("notif-crud");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        assert_eq!(store.unread_badge_count().unwrap(), 0);
        let id1 = notif(
            &mut store,
            notif_kind::STOPPED,
            "sess_a",
            severity::ATTENTION,
            100,
        );
        // Repeat kind+session refreshes in place instead of duplicating.
        let id2 = notif(
            &mut store,
            notif_kind::STOPPED,
            "sess_a",
            severity::ATTENTION,
            200,
        );
        assert_eq!(id1, id2);
        assert_eq!(store.unread_notifications(10).unwrap().len(), 1);
        // Attention feeds the badge; informational does not.
        notif(
            &mut store,
            notif_kind::APPEARED,
            "sess_b",
            severity::INFORMATIONAL,
            300,
        );
        assert_eq!(store.unread_badge_count().unwrap(), 1);
        notif(
            &mut store,
            notif_kind::ASSIGN_DONE,
            "sess_c",
            severity::COMPLETION,
            400,
        );
        assert_eq!(store.unread_badge_count().unwrap(), 2);
        // Explicit read flips one row; listing never marks.
        assert!(store.mark_notification_read(id1, 500).unwrap());
        assert!(!store.mark_notification_read(id1, 500).unwrap());
        assert_eq!(store.unread_badge_count().unwrap(), 1);
        assert_eq!(store.unread_notifications(10).unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notifications_reject_closed_vocab_and_prune_to_cap() {
        let dir = test_dir("notif-guard");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        assert!(store
            .notify(
                "process-spawned",
                "s",
                "p",
                "w",
                None,
                "x",
                "y",
                None,
                severity::ATTENTION,
                "d",
                1
            )
            .is_err());
        assert!(store
            .notify(
                notif_kind::APPEARED,
                "s",
                "p",
                "w",
                None,
                "x",
                "y",
                None,
                "urgent!!!",
                "d",
                1
            )
            .is_err());
        for i in 0..(MAX_NOTIFICATIONS + 10) {
            notif(
                &mut store,
                notif_kind::APPEARED,
                &format!("sess_{i}"),
                severity::INFORMATIONAL,
                1000 + i,
            );
        }
        let rows = store.unread_notifications(MAX_NOTIFICATIONS + 50).unwrap();
        assert_eq!(rows.len() as i64, MAX_NOTIFICATIONS);
        // Newest survive (highest created_at).
        assert!(rows
            .iter()
            .any(|n| n.session_id == format!("sess_{}", MAX_NOTIFICATIONS + 9)));
        assert!(!rows.iter().any(|n| n.session_id == "sess_0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notifications_table_carries_no_banned_columns() {
        let dir = test_dir("notif-privacy");
        let db = dir.join("pitwall.db");
        let mut store = Store::open(&db).unwrap();
        notif(
            &mut store,
            notif_kind::STOPPED,
            "sess_a",
            severity::ATTENTION,
            1,
        );
        drop(store);
        let conn = rusqlite::Connection::open(&db).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('notifications') ORDER BY cid")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for banned in [
            "argv",
            "env",
            "transcript",
            "evidence",
            "command",
            "title",
            "note",
            "prompt",
            "text",
        ] {
            assert!(
                !cols.iter().any(|c| c == banned),
                "banned column {banned}: {cols:?}"
            );
        }
        for required in [
            "id",
            "kind",
            "session_id",
            "severity",
            "detail",
            "created_at",
            "read_at",
        ] {
            assert!(
                cols.iter().any(|c| c == required),
                "missing {required}: {cols:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sync_derives_appeared_vanished_stopped() {
        use crate::collector::{SessionState, TerminalSession, WindowRole, WorkspaceSnapshot};
        use crate::platform::WindowInfo;
        fn sess(id: &str, state: SessionState) -> TerminalSession {
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
                role: WindowRole::Terminal,
                project: None,
                agent: crate::collector::AgentIdentity {
                    kind: crate::collector::AgentKind::Unknown,
                    confidence: crate::collector::Confidence::Low,
                    evidence: Vec::new(),
                },
                state,
                process_count: 1,
                processes: Vec::new(),
                last_activity_epoch: 1,
                last_activity_kind: crate::collector::LAST_ACTIVITY_KIND,
                summary: String::new(),
            }
        }
        let dir = test_dir("notif-sync");
        let mut store = Store::open(&dir.join("pitwall.db")).unwrap();
        // Seed previous observation: sess_old (running) + sess_gone.
        let mut snap0 = WorkspaceSnapshot {
            schema_version: 1,
            collected_at_epoch: 100,
            hostname: "h".to_string(),
            sessions: vec![
                sess("sess_old", SessionState::Running),
                sess("sess_gone", SessionState::Sleeping),
            ],
        };
        let obs0 = match store.persist(&snap0).unwrap() {
            PersistOutcome::Written { observation_id } => observation_id,
            PersistOutcome::Unchanged => panic!("must write"),
        };
        assert!(obs0 > 0);
        // Current: sess_old stopped, sess_gone absent, sess_new present.
        snap0.sessions[0].state = SessionState::Stopped;
        snap0.sessions.retain(|s| s.id != "sess_gone");
        snap0.sessions.push(sess("sess_new", SessionState::Running));
        let prev = store.observation_sessions(obs0).unwrap();
        let n = store
            .sync_snapshot_notifications(&prev, &snap0, 200)
            .unwrap();
        assert_eq!(n, 3, "appeared + vanished + stopped");
        assert_eq!(
            store.unread_badge_count().unwrap(),
            1,
            "only stopped badges"
        );
        // A steady observation fires nothing, even after reading its inbox.
        store.persist(&snap0).unwrap();
        let latest = store.latest_observation().unwrap().unwrap().0;
        let prev = store.observation_sessions(latest).unwrap();
        for n in store.unread_notifications(10).unwrap() {
            store.mark_notification_read(n.id, 250).unwrap();
        }
        let n2 = store
            .sync_snapshot_notifications(&prev, &snap0, 300)
            .unwrap();
        assert_eq!(n2, 0, "dedup must suppress repeats: got {n2}");
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
