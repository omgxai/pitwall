# INBOX — Child 1 (Pitwall UI Engineer)

## TASK-003 (assigned 2026-09-10, M5g hardening): compact tree + iconography audit

- OBJECTIVE: Design (do NOT implement) the smallest QML change for:
  (a) collapsed-by-default project groups (header shows project +
  count, expands on click); (b) icon-first tier/category headers
  using verified Nerd glyphs only (verify every codepoint with
  `fc-query` before proposing); (c) native subdued ScrollBar treatment
  following Omarchy/Quickshell theme primitives (no hard-coded white);
  (d) removal of any redundant running/status caption lines.
- READ: `plugin/dev.pitwall/Widget.qml`, `SessionBar.qml`,
  `StateReader.qml`; relevant `Ui/` kit components.
- CONSTRAINTS: icon-first, compact, stable in-flow expansion (no
  floating toast regressions), touch-friendly, no new timers, no
  polling, no AI inference from UI.
- FILE OWNERSHIP: read-only. No code changes, no commits.
- DO NOT TOUCH: `src/`, schema, security posture.
- ACCEPTANCE: proposal appended to `agent_child1.md` (structure, files,
  glyph table with verification, approach) + `.agents/status/child1.md`
  → COMPLETE with summary.
- PRIORITY: P1.
