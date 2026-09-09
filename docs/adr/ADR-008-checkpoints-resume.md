# ADR-008: Checkpoints and Resume (M4)

- **Status:** Accepted (M4).
- **Context:** Continuity needs named, bounded "what was this doing"
  records plus a deterministic way to continue — without becoming an
  autonomous agent.
- **Decision:**
  - `checkpoints` table (schema v2): identity/context/state/activity +
    human `note` (280 chars) + `trigger` (`manual`|`disappearance`).
    No transcripts, cmdlines, env, or output — enforced by schema.
  - Triggers: explicit `pitwall checkpoint` + disappearance during
    `snapshot` (one per continuous absence via last-sighting comparison).
  - Retention: newest 25 per project + global 500, enforced on write.
  - Resume Levels 1–2 only: focus live session (native activate / Lua
    dispatch CLI equivalent) or open one terminal at the validated
    project dir (`xdg-terminal-exec --dir`, fixed argv, no shell).
    Strict validation, no fallbacks, never starts agents.
  - state.json v2 adds scrubbed `resumable[]` (cap 10); all v1 fields
    unchanged. Panel Resume buttons state their concrete action.
  - `sess_*` = terminal-context identity (not agent-run or human-task).
  - Session state by severity (Stopped > Running > Sleeping > Unknown),
    order-independent.
- **Consequences:** Resume is explicit clicks on labeled actions. MCP can
  later read `checkpoints` directly. Agent start and richer triggers are
  deferred by design, not by accident.
