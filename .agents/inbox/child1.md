# INBOX — Child 1 (Pitwall UI Engineer)

## TASK-002 (assigned 2026-09-10, M5g hardening): information-architecture audit

- OBJECTIVE: Design (do NOT implement) the smallest native QML change
  representing: project → sessions tree, semantic priority
  (Pitwall-native → agents → workspace/apps → system, recency within),
  Unknown as evidence-state (never a category), icon-first density.
- READ: `plugin/dev.pitwall/Widget.qml`, `SessionBar.qml`,
  `StateReader.qml`; live `state.json`; one panel screenshot if you can
  take one (`omarchy capture screenshot` may need an unlocked desktop —
  do NOT attempt unlock/lock actions; skip screenshots if locked).
- FILE OWNERSHIP: read-only. No code changes, no commits.
- DO NOT TOUCH: `src/`, schema, security posture.
- ACCEPTANCE: proposal appended to `agent_child1.md` (structure, files,
  approach, accessibility, expected visual) + `.agents/status/child1.md`
  → COMPLETE with summary.
- PRIORITY: P1.
