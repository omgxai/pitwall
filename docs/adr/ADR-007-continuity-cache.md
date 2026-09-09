# ADR-007: SQLite continuity cache (M2)

- **Status:** Accepted (M2).
- **Context:** M1 observes; continuity needs bounded, queryable local
  history plus a file the future panel can read without spawning processes.
- **Decision:** `rusqlite` (bundled; sole dependency) with three tables
  (`meta`, `observations`, `sessions`), `PRAGMA user_version = 1`, no
  migration framework. Writes are hash-gated (meaningful content only,
  timestamps excluded); newest 100 observations kept, older pruned on
  write. `state.json` (state schema v1) is generated from the same snapshot
  model as `status --json` but is a separate, scrubbed interface: no
  command lines, no evidence strings, no PIDs-per-process.
- **Consequences:** Idle refreshes cost zero writes; corrupt DBs quarantine
  aside and recreate; newer schemas refuse writes but stay live-only. Full
  checkpoints/resume stay M4 — this is a cache, not an event log.
