# ADR-002: Why SQLite

- **Status:** Accepted (M0).
- **Context:** Pitwall needs lightweight, crash-safe local persistence for
  workspace snapshots and checkpoints, with no server and no accounts.
- **Decision:** SQLite (`rusqlite`) as the single state store, plus a small
  derived `state.json` snapshot file that the QML panel reads without
  touching the DB.
- **Consequences:** Zero infrastructure, trivial backup (one file), and a
  queryable history the future MCP server can expose read-only. Must keep
  write cadence low (event-driven + debounced) to avoid disk churn.
