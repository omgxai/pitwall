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

## 2026-09-10 — M5g tree/grouping: delegated, reviewed, implemented (PRE)

### Objective
First M5g hardening slice from dogfood wave 1: semantic display
hierarchy + project grouping, via real child agents.

### Child work (reviewed, accepted)
- Child 1: UI tree proposal (project-grouped, tier-ordered, collapse,
  StateReader untouched). Sound; adopted with adjustments.
- Child 2: grouping-correctness proof (roots=window PIDs, project
  sharing legitimate, /proc artifact identified) + derived
  `group_key=project.id else session.id`, no new DB. Adopted verbatim.

### PRE decision & implementation
Smallest coherent slice: (1) exclude `/proc|/sys|/dev` cwds at source
(spurious identity fix, tested); (2) pure `tier_for`/`group_for` in
output.rs with additive state.json fields (sessions + resumable);
(3) QML grouped rail reusing SessionBar/toast/actions (tiers,
  collapsible project headers, live-then-history per group).
No identity, schema, or DB changes. Resumable confidence added on
review (high-confidence history rendered `?` before).

### Verified result
- `cargo fmt/clippy/test` green (107 passed, incl. 4 new).
- Live: 6 sessions → grouped tree (pitwall-native ×3+2, agents,
  workspace); /proc project gone; screenshots inspected; zero QML
  errors across restart; resumable confidence correct.
- Children logs untouched by PRE (4 files remain theirs to commit).

### Status
Implemented by PRE (cross-cutting integration per §12). Children idle
awaiting follow-up. Next: user decides (more M5g slices or stand down).

---

## 2026-09-10 — M5g tree/grouping + Assign backend (PRE)

### Objective
First M5g hardening slice from dogfood wave 1: semantic display
hierarchy + project grouping (Child 1 UI proposal + Child 2 systems
proof, both reviewed), plus the Assign-task backend both reports
converged on needing.

### Child work (reviewed)
- Child 1: collapsed-by-default tree, icon-first tiers (19 glyphs
  fc-query-verified — spot-verified all 10 used), native scrollbar,
  R3/R4 trims. PARTIALLY adopted: rejected R1/R2 (brief §17 requires
  state+agent lines in detail), accepted rest.
- Child 2: Assign = foreground sync spawn reusing summary/runner
  shapes, strict validation chain, no persistence, MAX_PROMPT 4000,
  scrub second layer. Adopted with role-as-display-label.

### PRE decision & implementation
Smallest coherent slice: (1) /proc|sys|dev cwd exclusion at source;
(2) pure tier_for/group_for + additive state fields (sessions AND
resumable); (3) QML grouped rail reusing SessionBar/cards/actions;
(4) `pitwall assign` (validate→spawn→filter, no storage) + QML
mini-form in pinned card; (5) collapsed default, tier icons, native
scrollbar, R3/R4 trims, crash-artifact dismissal.
No identity/schema/DB changes (additive fields only).

### Verified result
- Gate green (fmt/clippy/107 tests incl. 9 new).
- Live: grouped tree screenshot (tiers, counts, muted history);
  expanded render with segments; collapsed default after restart;
  assign refusals live (malformed/vanished/long); SIGTERM argv
  proven; zero QML errors across restarts.
- Children logs untouched by PRE (theirs to commit).

### Status
Implemented by PRE (cross-cutting integration). Children idle.
Next: user decides (more M5g slices or stand down).

---

## 2026-09-10 — M5 freeze + next-sprint plan + brand asset (PRE, travel handoff)

### Objective
Documentation + future-readiness pass only. No features, no QML
redesign, no Rust refactor. Freeze the verified M5/M5g state, record
the notification baseline, add supplied brand art, plan the next
sprint for tomorrow. User travelling.

### Implementation
- Code freeze `2d49bb4`: notification inbox slice (table schema v4,
  sync events, CLI list/read, state.json inbox + badge, panel dots/
  rows/group counts, header-out-of-Flickable + brace fix). Gate green
  (fmt/clippy/118 tests); live-verified (open-panel screenshot, read
  path shrinks unread list, fresh events derive).
- Asset `31184e3`: `assets/pitwallpixelart.jpeg` (byte-identical copy,
  discovery art; flag stays UI identity).
- Docs (this commit): `docs/NEXT_SPRINT.md` (P0 hardening → P7
  sharing, acceptance criteria, open questions); CURRENT_CONTEXT
  freeze update; ROADMAP pointer; CHANGELOG + README notes.
- Principle locked: notification = "something happened"; AI summary
  = "what does the situation mean". Future chat/channel/bootstrap/
  context/cloud/sharing all marked FUTURE, none started.

### Verification
- `qmllint` clean (import warnings only); shell reload zero errors.
- Commercial-term scan on docs (below); links checked.
- Child-owned logs untouched (still modified, theirs).

### Status
Frozen for today. Next session starts at `docs/NEXT_SPRINT.md`.

---

## CURRENT STATE (2026-09-10 freeze)

- Frozen on `main`: notification code (`2d49bb4`) + brand asset
  (`31184e3`) + docs (this push). Working tree holds only
  child-owned logs (theirs, uncommitted).
- M5 + M5g slices (tree, assign, inbox) IMPLEMENTED and VERIFIED;
  project FROZEN. Next: `docs/NEXT_SPRINT.md`.
- Active child agents: Child 1 + Child 2 COMPLETE but sessions alive
  (awaiting follow-up or dismissal) — unchanged, not PRE's to close.
- Open hardening items (next sprint P0): twin disambiguation,
  summary-freshness signal, unknown label/color, transient-count
  semantics. Future (P1+): see `docs/NEXT_SPRINT.md`.
