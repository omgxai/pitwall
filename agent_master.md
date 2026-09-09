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

## 2026-09-10 — Dogfood wave 1: real child agents observed (PRE)

### Objective
Make the Master/Child model real: launch 2 OpenCode children in
separate Omarchy terminals, verify Pitwall detection, assign audits,
review reports, prioritize.

### Implementation
- Launched via `setsid foot --working-directory ~/Projects/pitwall
  --app-id org.omarchy.agent --title … opencode --auto --prompt …`
  (native mechanism, mirrors user agent terminal). First attempt used
  wrong cwd (`/home/guru/Work`); killed both, relaunched rooted —
  no stray files, verified reaped.
- Child 1 (UI audit) + Child 2 (systems audit), read-only scope,
  single-file write allowance each, no commits.
- `pitwall snapshot` + panel screenshot verification.

### Verified result
- Detection: 5 windows → 5 sessions (PRE + 2 children opencode/high
  with distinct titles, foot unknown/low, chromium app/unknown).
  Natural discovery, no synthetic state.
- Grouping: window-rooted confirmed; ? shell rows are distinct
  terminal roots (keep separate — §19 answered, no fix).
- Privacy: DB 18 cols + state keys clean (no argv/env/evidence).
- Hierarchy gap CONFIRMED: recency-only ordering, no semantic
  categories (Pitwall-native/agent/app/system) — M5g design item.
- UI findings accepted to M5g backlog: twin disambiguation, summary-
  freshness signal, `? terminal` label, live-unknown vs resumable
  color, transient/zombie count nuance (design TBD, not a hack).
- Crash notification during wave investigated: restart-teardown
  SI_TKILL artifact, shell healthy, dismissed.

### Tests
- `pitwall status --json` live (5 sessions); panel screenshots;
  no Rust/QML changes this wave (audit-only).

### Status
Wave 1 complete. Children COMPLETE (reports in), still running
(PIDs available on request). Their 4 log files left UNCOMMITTED
per no-commit-others'-work rule — need child commit or wave 2.

---

## 2026-09-10 — Context compaction handover (PRE)

### Objective
Canonical `docs/CURRENT_CONTEXT.md` (~2k words) so a fresh session can
start without prior conversation. No code, no milestones, no M5g.

### Implementation
- Wrote `docs/CURRENT_CONTEXT.md` (21 sections: identity → next task).
- Recorded wave-1 lessons, locked decisions, rejected/deferred list,
  boundary statement, ranked next step (semantic hierarchy first).
- Verified links, milestone/commit accuracy, commercial-term clean.

### Status
Complete. Handover ready.

---

## CURRENT STATE

- Latest verified commit: evolving this wave (verify at push).
- Active child agents: Child 1 + Child 2 COMPLETE but sessions alive
  (awaiting follow-up or dismissal).
- M5g backlog (prioritized): semantic display hierarchy; twin
  disambiguation; summary-freshness signal; unknown label/color;
  transient/zombie count semantics.
- Next action: user decides — child log commit, M5g kickoff, or stand down.
