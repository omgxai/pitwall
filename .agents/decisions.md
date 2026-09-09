# Pitwall Coordination Decisions

- D1 (2026-09-10): M5 UI FROZEN. No cosmetic redesign without user order.
- D2 (2026-09-10): No unilateral child-agent launches. Staffing child
  sessions spends tokens and uses terminals; requires user approval.
- D3 (2026-09-10): File mailbox does not notify. Inbox write != receipt.
  Only `.agents/status/*` updates by the child itself count as signal.
- D4 (2026-09-10): Next phase is M5g hardening, not features.
- D5 (2026-09-10): Coordination files (`agent_*.md`, `.agents/`) are
  committed to the repo like `.pitwall/STATE.md` — shared memory.
