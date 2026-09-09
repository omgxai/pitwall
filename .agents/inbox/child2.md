# INBOX — Child 2 (Pitwall Systems Engineer)

## TASK-002 (assigned 2026-09-10, M5g hardening): grouping-correctness audit

- OBJECTIVE: Determine, with evidence: which processes are roots vs
  children; which sessions share a project (deterministic key);
  whether any duplicates are illegitimate; what derived presentation
  grouping field (if any) can safely support the UI tree. Prefer a
  derived field over identity changes; no new database unless you can
  prove it unavoidable (you likely cannot — argue it).
- READ: `src/collector.rs`, `src/ids.rs`, `src/store.rs` (read-only),
  live `./target/debug/pitwall status --json` (rebuild only if binary
  missing).
- FILE OWNERSHIP: read-only. No code changes, no commits, no redesign.
- DO NOT TOUCH: `plugin/` visuals, security posture, cloud anything.
- ACCEPTANCE: findings appended to `agent_child2.md` (roots/children
  table, project-sharing verdict, grouping-key proposal) +
  `.agents/status/child2.md` → COMPLETE with summary.
- PRIORITY: P1.
