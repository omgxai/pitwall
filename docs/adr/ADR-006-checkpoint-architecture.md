# ADR-006: Local state / checkpoint architecture

- **Status:** Accepted (M0; schema lands in M2/M4).
- **Context:** "Resume" needs durable, human-readable continuity records.
- **Decision:** Two tiers: (1) `snapshots` — frequent, overwritten live
  workspace state (also rendered to `state.json` for QML); (2) `checkpoints`
  — infrequent, append-only records (project, cwd, branch, status, terminal,
  agent, summary, next-step, timestamp) created on meaningful transitions
  (project switch, agent exit, manual save). `.pitwall/STATE.md` tracks
  *development* progress, not workspace state — the two must not be confused.
- **Consequences:** Simple to query, cheap to store, and directly reusable by
  the future MCP `get_project_checkpoint` capability.
