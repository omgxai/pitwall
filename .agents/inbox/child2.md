# INBOX — Child 2 (Pitwall Systems Engineer)

## TASK-001 (standing, on staffing): Settings/sharing boundary audit

- OBJECTIVE: Verify configuration, ephemeral sharing concerns, and any
  future share interfaces are not contaminating the M5 core
  (no cloud, no keys, no transcript persistence).
- FILE OWNERSHIP: read `src/config.rs`, `src/store.rs`, `src/summary.rs`,
  `src/context.rs`, `SECURITY.md`; write nothing yet.
- DO NOT TOUCH: `plugin/dev.pitwall/*`, visual behavior, cloud features.
- ACCEPTANCE: report posted to `.agents/status/child2.md` with findings
  or all-clear.
- PRIORITY: P2 (after staffing).

## Rules
- No cloud functionality unless the master explicitly assigns it.
- Never claim receipt: update `.agents/status/child2.md` yourself.
