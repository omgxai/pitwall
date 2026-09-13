# Pitwall Current Context

> Compact handover for the next AI session. Read this + `agent_master.md`;
> Git remains source of truth for code. Written 2026-09-10 at freeze
> series `2d49bb4` (code) + `31184e3` (asset); this file lands in the
> docs-freeze commit on top — verify with `git log --oneline -3`.
> M5/M5g is FROZEN. No new features until the next sprint starts.

## 1. Product Identity

Pitwall is **Omarchy's workspace awareness layer for AI-assisted development**.
Core principle: **Pitwall is the human's window into the AI workspace.**
Core loop: **SEE → UNDERSTAND → REMEMBER → RESUME → CONTROL.**

Pitwall is: lightweight, local-first, Omarchy-native (Quickshell bar widget),
open-source (MIT), developer-focused, privacy-conscious, agent-aware.
Pitwall is NOT: an AI coding agent, provider gateway, model marketplace,
cloud IDE, terminal replacement, billing platform, or autonomous system.

## 2. Current Product State

All IMPLEMENTED and live-verified unless noted:

- **Discovery** (`src/collector.rs`, `src/platform/linux.rs`): `/proc`
  (stat/comm/cmdline/cwd/**exe basename**) + `hyprctl clients -j`
  joined per window. Zero dependencies for collection.
- **Sessions**: one terminal/window root = one session; descendants grouped
  by process tree; project = mode-cwd of descendants (excl. `/`), with
  trailing-slash/symlink normalization; Git branch + clean/dirty per dir.
- **Agent detection**: cmdline + exe basename + `org.omarchy.agent` class +
  title prefix (`OC |`…). High ≥2 signals / Medium lone cmd-exe / Low lone
  title / honest Unknown (`terminal context only`). Never invented.
- **Roles**: terminal / app / unknown from verified classes. Agent
  inference skipped for apps (Chromium renders as Chromium, not
  "? unknown agent").
- **Continuity** (`src/store.rs`, SQLite, schema v4): `meta`,
  `observations` (newest-100), `sessions`, `checkpoints` (manual +
  disappearance, 25/project + 500 global, 280-char notes),
  `summaries` (input_hash PK), `notifications` (closed-vocab
  kind/severity, session/project/agent columns, dedup key, 100-row
  cap, column set pinned by test — no argv/env/transcript).
  Hash-gated writes; systemd unit ships disabled.
- **Notifications** (M5g slice, frozen): sync-derived per snapshot —
  session appeared/vanished/stopped, assign completion (completion) /
  failure (attention). Bounded transitions only, never per-process /
  per-observation / per-tick. `state.json` carries unread rows (cap
  20) + `unread_count` badge (attention/completion only;
  informational lists, never badges). `pitwall notifications
  [--unread]` lists; `notifications read <id>` marks exactly one row.
  Core principle: **notification = "something happened"; AI summary
  = "what does the situation mean?"** (see `docs/NEXT_SPRINT.md` §1).
- **Resume** (`pitwall resume`): Level 1 focus live session (native
  `Toplevel.activate()`; CLI Lua-dispatch equivalent), Level 2 open
  terminal at validated dir (`xdg-terminal-exec --dir`). Strict validation,
  no fallbacks, **never starts agents**.
- **Panel** (`plugin/dev.pitwall/`, 6 files): collapsed flag-only button;
  header (flag + PITWALL + refresh + gear); AI summary ticker (ping-pong,
  hover-pause, click-expand); session rail newest-first (log-scaled
  duration bars, R/S/U history segments); hover-highlight + click-pin
  detail cards (deterministic detail, never AI-inferred per-bar);
   actions Focus/Stop(SIGTERM-only)/Close/Resume on explicit clicks;
   settings (Agent/Model/Toggle via `pitwall config`); RESUME history rows.
   Notification inbox: per-entry unread dot, inbox rows in the pinned
   card (max 3 + `+N more`), group unread counts, header badge
   (attention/completion only, hidden at zero). Listing/expanding
   never marks read — only an explicit row click does. Header lives
   outside the scroll Flickable (stable hitboxes).
- **AI summary**: ephemeral bounded context (structured state + 10+10
  terminal lines where observable + derived events + checkpoints; 6
  sessions, 16KB cap) → user-configured agent (`opencode run --format
  json -f`, fixed argv, timeout+kill) → text-only extraction → cached by
  input hash → `state.json` v3 `summary{}` → ticker. Explicit Generate
  button or `pitwall summarize` only. No auto-inference, no polling.
- **Settings**: `~/.config/pitwall/config` (0600, agent/model/
  summary_enabled) via `pitwall config get/set`; echoed into state.json
  for QML. No provider keys anywhere in Pitwall.
- **Refresh**: expand/manual runs `pitwall snapshot` (state only).
  Refresh ≠ AI, ever.

CLI surface (all explicit, no daemons): `status [--json]`, `snapshot
[--db/--state/--data-dir]`, `checkpoint [--note --session-id]`,
`resume --session-id` (focus live or open validated terminal; never
agents), `agents`, `models [--agent]`, `summarize [--agent --model
--dir --timeout --dry-run --clear]`, `assign --session-id --role
--prompt [--model --timeout]` (validated foreground agent run, no
persistence; completion/failure raises a notification),
`notifications [--unread]` / `notifications read <id>`,
`config get|set` (agent, model, summary_enabled only). Exit codes: 0 ok, 1 operational failure,
2 usage error.

## 3. Architecture

```
Omarchy / Hyprland (windows, PIDs)
        ↓ hyprctl + /proc (read-only)
Pitwall discovery (collector, platform trait)
        ↓ normalized snapshot
Workspace state (in-memory)
        ↓ hash-gated persist
SQLite continuity (observations/sessions/checkpoints/summaries)
        ↓ read-time derivation (events, resumable, timeline meta)
Ephemeral AI context (/run/user tmpfs, scrubbed, Drop-guard cleanup)
        ↓ -f attachment, fixed argv
User's configured agent (own auth/billing/model)
        ↓ text-only extraction
Cached summary → state.json v3 → Quickshell/QML panel
```

Privacy boundaries: no argv/env/transcripts/file-contents in SQLite,
state.json, logs, or repo (unit-tested with canary secrets); terminal
text lives only in the ephemeral file; full cmdlines exist solely in
ephemeral `status --json` output.

## 4. Milestone History

| Milestone | Status | Purpose | Commit |
|---|---|---|---|
| M0 | IMPLEMENTED | Repo foundation | `62fdfdb` |
| M1 | IMPLEMENTED | Discovery | `7f5ec34` |
| M2 | IMPLEMENTED | Continuity cache | `5511eaa` |
| M3 | IMPLEMENTED | Panel prototype | `5a591d5`+`751c7d6` |
| M4 | IMPLEMENTED | Checkpoints + Resume | `c76dea5` |
| M5a | IMPLEMENTED | Evidence/roles/discovery CLI | `3f04f45` |
| M5b | FOLDED into M5c | Derived events (read-time, no tables) | — |
| M5c | IMPLEMENTED | Ephemeral context + invocation | `99c6fae` |
| M5d | IMPLEMENTED | Summary cache + state v3 | `41aaa03` |
| M5e | IMPLEMENTED | Pixel flag identity | `63a85e8` |
| M5f | IMPLEMENTED | Timeline rail + interactions + polish | `09106f9`+`b193021`+`c57e6f9` |
| M5g tree | IMPLEMENTED | Semantic tree + project grouping + assign backend | `f92037c`+`7aff577` |
| M5g inbox | IMPLEMENTED (FROZEN) | Notification slice: table + sync events + CLI + state fields + panel inbox | `2d49bb4` |
| Brand asset | IMPLEMENTED | `assets/pitwallpixelart.jpeg` (discovery art; flag stays UI identity) | `31184e3` |

M5 UI is feature-frozen. 118 unit tests; `cargo fmt/clippy/test` gate.
Next sprint plan: `docs/NEXT_SPRINT.md` (P0 hardening → P7 sharing).
Everything beyond §2's list is PLANNED / FUTURE / DEFERRED.

## 5. Current Git State

- Freeze series on `main`: `2d49bb4` (notification code freeze) →
  `31184e3` (brand asset) → docs-freeze commit (this file; verify
  with `git log --oneline -3`, then `== origin/main` after push).
- Tree: only child-agent log files remain modified (theirs — do not
  touch). PRE-owned code/docs/asset all committed.
- Rust: gate green at freeze (`fmt`, `clippy --all-targets
  --all-features -D warnings`, 118 tests). QML validated via live
  shell + open-panel screenshot (headless ceiling: no physical
  click/tap; CLI equivalents proven).

## 6. Current UI Model

Collapsed: flag icon only. Expanded: `[flag] PITWALL [↻] [⚙]` (+ unread
badge when attention/completion pending); summary
ticker; rail (label `●/○/? kind · project[:branch] · N ▾/▴` + duration
bar); pinned card (state/agent/window/project lines + actions +
inbox rows for that entry); resumable history rows (muted, short-hash distinguishable); settings
view swaps the body. Unread dots mark entries with pending
notifications; group headers carry unread counts.

Display hierarchy (IMPLEMENTED wave M5g-1): tier captions
(PITWALL-NATIVE → AGENTS → WORKSPACE → SYSTEM) with collapsible
project groups (default collapsed) and recency within; resumable
history merged muted into groups. Pitwall must never become a flat
process monitor.
Assign flow: pinned card `+ Assign` → inline prompt/role form →
`pitwall assign` (validated, foreground, result shown in card).

## 7. Session Identity / Grouping

VERIFIED live (5 windows → 5 sessions, dogfood wave 1): one terminal
root = one session; descendants grouped; separate windows stay separate
even sharing shell/user/project. `sess_*` = terminal-context identity
(not agent-run, not human-task); `proj_*` = canonical dir. "Shell
duplicates" to date are legitimate distinct roots — keep separate.
Residual nuance: transient children can nudge counts/activity at sample
instants (design TBD in M5g, not a hotfix).

## 8. Agent Detection

Supported table (`pitwall agents`): opencode HIGH (binary + `models` +
`run --format json` + default-agent status), claude/codex MEDIUM
(binary + non-interactive flags; model list unvalidated → agent
default); hermes not an executable, aider absent, gemini/crush+ CLI
unvalidated — all excluded. Evidence/confidence is a pure local
function; the LLM never classifies. Unknown renders as `?`/hollow glyph
with the reason in the detail card, never as a guessed agent.

## 9. AI Summary

Context (A) structured state, (B) 10+10 terminal lines where safely
observable, (C) derived events (appeared/vanished/agent/branch/git/
checkpoint), (D) latest checkpoints. Foot: `unavailable (no scrollback
API)` marker — honest, not an error; kitty socket path implemented but
**not live-validated** (no kitty session observed). Scrub matrix
(sk/gh/AKIA/xox/Bearer/password/PEM/bare-forms) + structural bans.
Ephemeral file: `/run/user/$UID/pitwall/ctx-<urandom>.json`, 0700/0600,
O_EXCL, Drop-guard cleanup on every path (tested incl. timeout/drop).
Delivery: fixed argv, content only via `-f`, static interpret-only
instruction with evidence hierarchy (IO ≠ task meaning). Cache by
structured-only input hash; state.json v3 `summary{text,model,
created_at,input_hash,status}`; ticker + Generate button + error/empty
states. **Refresh = snapshot. Generate = explicit AI. Opening/hovering/
selecting never invokes AI.**

## 10. Terminal Context Privacy

- No transcript storage, no argv/env storage, no terminal-content tables
  (schema-shape test pins column sets).
- Temp location `/run/user/$UID/pitwall/` (tmpfs), 0700 dir, 0600 files,
  unpredictable names, unlinked on success/error/timeout/drop (tested).
- Scrub before writing; canary-secret unit tests.
- Foot gap + kitty-pending status as above. Verified: no residue after
  live runs; agent modified nothing (read-only contract held).

## 11. Master / Child Agent Model

PRE (Race Engineer) coordinates; Child 1 (UI) owns `plugin/`+`assets`;
Child 2 (systems) owns Rust core/config/security proposals. Mailbox =
files (`.agents/inbox|status/`); a file is NOT receipt (no IPC exists).
Shared worktree rules: never reset/clean/stash, never blind `add -A`,
stage own files only, leave others' work alone. Logs:
`agent_master.md` (consolidated truth) + `agent_child{1,2}.md`
(detail). Demonstrated: dogfood wave 1 ran two REAL `foot →
opencode --auto --prompt` children (verified PIDs, windows, Pitwall
detection as opencode/high). Intended future: same shape on demand.

## 12. Dogfooding Results (wave 1, 2026-09-10)

- PRE observed: PRE + 2 children + foot + Chromium, all correctly
  classified with distinct titles; panel rail screenshot-verified.
- Child 1 (UI): sessions understandable; agents identifiable; terminals
  weakly distinguishable; twins rail-identical (no title shown);
  summary-freshness unsignaled; `? shell` overstates (prefer `? terminal`);
  live-unknown color collides with resumable-muted.
- Child 2 (systems): grouping window-rooted ✓; detection correct with
  3-way evidence ✓; privacy intact (18 session cols, scrubbed keys) ✓.
- Coordination: mailbox works as files; launch-cwd mistake caught early
  (first launch inherited `/home/guru/Work` — children would have
  written reports to the wrong directory; both killed cleanly with zero
  stray files and relaunched with `--working-directory` + prompt-side
  `pwd` check; lesson recorded: always pin child cwd explicitly).
- Fixes made: none (audit scope; freeze holds). Crash notification seen
  = restart-teardown SI_TKILL artifact, shell healthy, dismissed.
- Residual unknowns: physical click/tap feel, 6+ session scroll, vertical
  bar, kitty text path, real cost/latency of repeated summaries — all
  require a staffed live session, none block M5g planning.

## 13. Current Known Issues

| Sev | Issue | Evidence | Status |
|---|---|---|---|
| P1 | ~~Display hierarchy is recency-only, not semantic~~ | wave-1 panel | IMPLEMENTED (M5g tree, frozen) |
| P2 | Same-project twins rail-identical | screenshot | OPEN (title disambiguator, next sprint) |
| P2 | Summary freshness unsignaled (`isStale` watches snapshot) | code review | M5g |
| P3 | `? shell` overstates; unknown-muted == resumable-muted | screenshot | M5g micro-fix |
| P3 | Transient children nudge counts/activity | sampling | design TBD, not hotfix |
| P4 | Click/tap, 6+ sessions, vertical bar, kitty path untested live | headless ceiling | validate when possible |

## 14. Decisions Already Made (do NOT reverse without user order)

Local-first; no OpenRouter/direct-provider integration; no Pitwall-held
API keys (agent owns auth/billing); no transcript persistence; no fake
identity; one terminal root = one session; M5 UI frozen; semantic
hierarchy direction; no process-monitor dashboard; no uncontrolled
expansion; refusals never fall back; checkpoints/retention bounds;
fixed-argv subprocesses only; mailbox ≠ receipt; Git/source wins over
reports.

## 15. Explicitly Rejected / Deferred

Cloud/Supabase/sync, mobile, full MCP, secret vault/rotation, billing,
marketplace, autonomous agents/deployments, QR sharing, RAG, transcripts,
macOS/Windows, per-session summaries, streaming, agent auto-run,
transcript database, background AI polling. (Deferred, not denied
forever — except autonomy/safety bans, which stand.)

## 16. Public Roadmap

M0–M5f built; M5 + M5g slices (tree, assign, inbox) built and frozen;
brand asset added. M6 user-local packaging is implemented: `install.sh`
installs the release binary, plugin, and optional user units; the clean-room
smoke test passes; uninstall preserves user data. The v0.1.0 release gate
passes; fresh-machine Omarchy verification remains a follow-up limitation.
M7 summary freshness now rejects mismatched cached text, and the ticker
rotates bounded summary sentences. Chat and external channels remain
deferred because the current runner is synchronous summary-only execution.
The shared `SummaryContext` is now the deterministic privacy boundary for
sessions, events, checkpoints, and scrubbed notification facts; terminal
text is still ephemeral and excluded from its hash.
The visual pass labels the summary surface `AI BRIEF` and uses a subdued
palette-bound surface; the installed panel was updated, but a live
Quickshell restart/click-through remains unverified because of the existing
duplicate IPC handler.
Directions: AI Workforce Design + community Recipes (see
`docs/AI_WORKFORCE.md`, `docs/RECIPES.md`). Contribute: recipes,
patterns, Omarchy integration, detection, docs, tests. MIT.

## 17. Workforce Design (one paragraph)

Human owns decisions; master coordinates; scoped child workers produce
into Git (the only shared truth); local-file memory; explicit approval
boundaries; cheap resumption via state files; Pitwall observes so the
master coordinates from evidence. Verified in miniature by wave 1.

## 18. Recipes (one paragraph)

Reusable human+AI workflows as small Git-friendly docs (roles,
handoffs, memory, coordination, approvals, Git/test workflow, Pitwall
use). Document practiced runs, not theory. No marketplace, no accounts.

## 19. Public / Private Boundary

The public repo is developer-focused open source. No commercial
strategy lives here or in any public artifact. (Boundary verified by
term scan at each docs commit.)

## 20. Development Rules for Future Agents

Git/source is truth; inspect before modifying; never blind `add -A`;
never reset others' work; one terminal root = one session; never invent
identity; no transcript/credential persistence; no silent AI inference;
no feature creep; M5 UI frozen; correctness before features; semantic
hierarchy direction; privacy-first; Omarchy-native; verify real behavior
before claiming success; mailbox files are not receipt.

## 21. Next Step

FROZEN for 2026-09-10 (user travelling). Next session starts at
`docs/NEXT_SPRINT.md`: P0 = M5/M5g hardening and correctness on the
frozen baseline (twins disambiguation, summary-freshness signal,
unknown label/color are the known OPEN items in §13). Do NOT start
P1+ without design review per roadmap governance. Runway is clean:
code frozen (`2d49bb4`), asset in (`31184e3`), docs in this commit,
only child-owned logs uncommitted (theirs).
