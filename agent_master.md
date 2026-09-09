# Pitwall Master Development Log

> Authoritative consolidated project story. Date-first. Child files
> (`agent_child1.md`, `agent_child2.md`) keep detail; this file keeps
> verified truth. If child reports disagree with Git/source, Git wins.

## 2026-09-10 — PRE established (Pitwall Race Engineer)

### Objective
First mission (§32): establish the master, inspect workspace/Git/docs,
determine child status, set up coordination, dogfood.

### Findings (verified, not assumed)
- Tree clean at `c57e6f9` == `origin/main`. Full milestone chain
  M0→M5f+polish pushed. No uncommitted work from anyone.
- No `agent_master.md` / child files / `.agents/` existed — created now.
- Child agents: NOT running. One opencode process (PID 328900) exists
  and it is this PRE session itself (cwd `~/Projects/pitwall`).
  No Child 1 / Child 2 processes anywhere.
- Dogfood: `pitwall status` sees PRE as `opencode [high] on
  pitwall (main)` + one plain foot session. Distinguishable, correct.
- STATE.md top block was stale ("M5f polish pending commit/push" after
  it was pushed as `c57e6f9`; "Remaining: M2" leftover). Fixed.

### Decisions
- D1: M5 UI stays FROZEN. No cosmetic redesign without user order.
- D2: No unilateral child-agent launches (token spend + terminal use
  need user approval). Inbox tasks prepared; children start on user go.
- D3: Mailbox is files only until a verified IPC exists. A file in an
  inbox is NOT proof of receipt.
- D4: Next engineering phase is M5g hardening (correctness, safety,
  docs, install) — not features.

### Verification
- `git status` clean; `git log` matches GitHub chain.
- `pitwall status` + `pitwall snapshot` run green from PRE session.

### Status
Single-agent operation (PRE only). Awaiting user direction.

---

## 2026-09-10 — Docs & roadmap governance (PRE)

### Objective
Public roadmap/AI-workforce documentation freeze. Docs only; no code,
no milestones, no M5g.

### Implementation
- Rewrote `ROADMAP.md`: built milestones marked complete, M5 frozen,
  M5-hardening + AI Workforce Design + Recipes directions, governance
  rule, contribution areas, M6 intact.
- Refreshed `README.md` status (M5f), removed obsolete OpenRouter
  wording, added workforce/recipes pointers.
- Added `docs/AI_WORKFORCE.md`, `docs/RECIPES.md`.
- Fixed `SECURITY.md` principle 3 (agent-delegated, was OpenRouter).
- Added recipe note to `CONTRIBUTING.md`.
- Commercial-term scan: clean (below).

### Verification
- `git status` clean before/after; docs-only diff; markdown reviewed.
- No Rust/QML behavior changed (`cargo` untouched by this task).

### Status
Complete. Awaiting user direction.

---

## CURRENT STATE

- Latest verified commit: `124be9a` (== origin/main, tree clean).
- Active child agents: NONE (inbox tasks queued, unstaffed).
- Next action: user decides — M5g hardening kickoff or child staffing.
