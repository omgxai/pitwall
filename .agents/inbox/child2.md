# INBOX — Child 2 (Pitwall Systems Engineer)

## TASK-003 (assigned 2026-09-10, M5g hardening): assignment backend audit

- OBJECTIVE: Determine the safest architecture for assigning a prompt
  to a selected Pitwall session (future `+ Assign` flow): available
  agent invocation mechanism, fixed-argv construction, project/session
  targeting, role/name handling (display label only vs identity),
  lifecycle tracking, failure handling, privacy implications
  (no secrets/env/transcript in prompts; prompt length caps).
- READ: `src/agents.rs`, `src/summary.rs` (fixed-argv precedent),
  `src/resume.rs` (validation precedent), `src/store.rs`,
  `src/collector.rs` identity.
- CONSTRAINTS: no shell interpolation, no arbitrary commands, explicit
  human click required, no background execution, no session merging,
  no new database unless proven unavoidable, no orchestration framework.
- FILE OWNERSHIP: read-only. No code changes, no commits.
- DO NOT TOUCH: `plugin/` visuals.
- ACCEPTANCE: findings appended to `agent_child2.md` (mechanism table,
  argv shape, validation rules, lifecycle/failure/privacy analysis) +
  `.agents/status/child2.md` → COMPLETE with summary.
- PRIORITY: P1.
