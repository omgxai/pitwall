# Pitwall Next Sprint (2026-09-11+)

> Planning document for tomorrow's Master Agent. Nothing below is
> implemented except §1. Do not describe §2+ as built. Roadmap
> governance (`ROADMAP.md`) applies to every item: it must improve
> workspace awareness, preserve local-first/privacy, fit Omarchy, and
> justify its complexity — otherwise deferred.

## 1. Current frozen baseline (IMPLEMENTED + VERIFIED)

M0–M4, M5a, M5c, M5d Part 1, M5e, M5f: IMPLEMENTED (see CHANGELOG).
M5b folded into M5c. M5g workforce-control slices IMPLEMENTED and
frozen: semantic project tree (tiers pitwall-native > agents >
workspace > system, collapsed-by-default groups), `pitwall assign`
(validated foreground run, in-card form), and the notification inbox
slice (frozen `2d49bb4`):

- `notifications` table (schema v4, additive): closed-vocab kinds
  (appeared/vanished/stopped/assign-done/assign-failed/checkpoint…),
  closed-vocab severities (informational/attention/completion),
  session/project/branch/agent columns, dedup key, 100-row cap,
  column set pinned by test (no argv/env/transcript/evidence).
- Sync-derived events per snapshot: session appeared / vanished /
  stopped; assign completion (completion) / failure (attention).
  Bounded and non-spammy by construction (transitions only, never
  per-process, per-observation, per-refresh, or per-tick).
- `state.json`: `notifications[]` = unread rows only (cap 20) +
  `unread_count` = attention + completion badge count. Informational
  rows are listed, never badged.
- CLI: `pitwall notifications [--unread]` (list, `•` = unread),
  `pitwall notifications read <id>` (marks exactly one row read).
- Panel: per-entry unread dot; inbox rows in the pinned card (max 3
  + `+N more`); group-level unread counts; header badge
  (attention/completion only, hidden at zero). Listing/expanding
  never marks read — only an explicit row click does
  (`notifications read <id>` + refresh).
- Core product principle: **notification = "something happened"**;
  **AI summary = "what does the overall situation mean?"**.
  Example — notification: `✓ Agent completed grouping audit`;
  summary: `Both supporting agents completed their audits. The
  remaining work is integration and verification.`
- Gate at freeze: `cargo fmt --check`, `cargo clippy
  --all-targets --all-features -- -D warnings`, `cargo test --all`
  118 passed. Live-verified: panel open screenshot, read path
  (unread list shrinks, fresh events derive), `state.json` parses.

Branding: `assets/pitwallpixelart.jpeg` (frozen `31184e3`) is
discovery/documentation art. The 16px checkered flag
(`assets/flag.svg`) remains the compact UI identity — unchanged.

## 2. Priority 1 — Notification UX refinement (P1)

FUTURE. Direction only; design before implementation:

- Badge beside the Pitwall flag; group-level unread counts (exists —
  refine); session-level indicator where appropriate.
- Expand a notification → concise explanation; reading the actual
  notification reduces its unread count (exists for rows — extend
  carefully, keep the explicit-read contract).
- Possible future categories: completed / attention / failure /
  resumable / informational. Must remain local-first,
  privacy-preserving, bounded, non-spammy, evidence-based.
- Must NOT become a general event log.

## 3. Priority 2 — AI summary + contextual chat (P2)

FUTURE. Current baseline (frozen): concise cached summary, ~160-char
ticker with ping-pong scroll, hover-pause, click-expand, explicit
Generate, no auto-inference. Future evolution:

```
compact scrolling summary → click/touch → expanded summary
    → optional Chat icon → "Ask Pitwall" over selected context
```

Candidate questions: what is this agent doing / why is this session
idle / what remains unfinished / which agent should I check / what
changed since I left / which sessions need attention. Same
privacy-safe contextual architecture: NEVER raw transcripts,
credentials, env vars, arbitrary files, secrets. Mark:
FUTURE — Pitwall contextual chat.

## 4. Priority 3 — Agent communication/channel integration (P3)

FUTURE. No new provider, no gateway, no OpenRouter, no Pitwall-held
credentials — ever. If the user already has a configured agent/channel
(e.g. existing Hermes/OpenClaw channel config), future Settings may
offer: communication channel / agent / model / notification mode
(visual only · visual + agent communication · delegated
notification). Integration must be designed and live-verified first.
Pitwall uses the user's channel; it never becomes a
provider-management product. Mark: FUTURE — agent communication
channel integration.

## 5. Future / deferred (P4–P7, all FUTURE)

- **P4 — Pitwall project bootstrap / recipes.** "Make Pitwall Native"
  workflow: new project → recipe/template → project-specific
  Master + child structure → Pitwall-aware project. Recipes stay
  forkable, inspectable, GitHub-native, community-contributed. Never
  a marketplace. No commercial strategy in this repo.
- **P5 — Context monitoring + handover assistance.** Opt-in Context
  Monitor (Master/Child usage, warning threshold, fresh-session
  recommendation, handover readiness; possible `[Prepare Handover]` /
  `[Start Fresh Master]`). Automation OFF by default. Never invent
  usage numbers the agent runtime does not provide. Today's
  Master-agent handover practice is the design experiment for this.
- **P6 — Multi-node Pitwall (possible Tailscale foundation).**
  Laptop/Desktop/Server trees unified in one view. Local mode must
  remain fully useful without it; cloud is an optional extension,
  never a dependency. No implementation now.
- **P7 — Temporary remote sharing.** Share icon/QR → 1h/4h/12h/24h
  expiry → remote sees summary/selected state only. Never
  transcripts, credentials, source, or machine control.
  FUTURE only.
- **Parent/child delegation from Pitwall.** `Assign` exists (frozen);
  future: select session → Assign/Delegate → prompt → Master
  delegates to a named visible child → Pitwall observes → completion
  becomes a notification → Master receives the result. The user
  never leaves Pitwall to start delegated work. Beyond current
  `assign`: FUTURE.

## 6. Explicit non-goals (this sprint and v0.x)

No new product features, no QML redesign, no Rust refactor, no
databases beyond SQLite continuity, no cloud services, no networking
(tailnets, daemons, remote control), no APIs, no chat systems, no
agent orchestration, no provider credentials, no billing/marketplace
mechanisms, no transcript storage, no autonomous actions. See
`ROADMAP.md` (out of scope for v0.x).

## 7. Acceptance criteria (for any sprint item)

1. Designed (fits governance rule) → implemented → verified →
   documented. No feature exists until all four hold.
2. `cargo fmt --check`, `cargo clippy --all-targets --all-features
   -- -D warnings`, `cargo test --all` green; new behavior has tests.
3. Live-verified on a real shell: zero QML errors across restart,
   screenshot-inspected, CLI equivalents proven where headless
   clicks are impossible.
4. Privacy sweep: no argv/env/transcript/secret in DB, state.json,
   logs, or repo (canary tests where applicable).
5. Docs updated in the same push (context + master log + changelog
   as appropriate); child-owned files never touched.
6. Future work stays marked FUTURE/PLANNED/DEFERRED until merged.

## 8. Open architectural questions

1. Notification category taxonomy: which kinds earn badge vs list?
   (Current: attention/completion badge, informational lists.)
2. Where does "Ask Pitwall" context come from without widening the
   privacy boundary? (No answer yet — design required.)
3. Channel integration: what is the narrowest verified contract with
   the user's existing agent tooling? (Unknown — verify first.)
4. Context usage: which runtimes expose reliable numbers, if any?
   (Never estimate; OFF by default.)
5. Multi-node identity: how do `sess_*`/`proj_*` stay unique across
   nodes without a central registry? (Unresearched.)
6. Sharing redaction: what is the minimal safe shared view?
   (Summary + selection only, per §5 — needs a design.)
