# Pitwall Workspace State (durable checkpoint)

> The repository is the source of truth. This file is the resume point for
> interrupted sessions (sleep, restart, dropped agent sessions).

## Last verified milestone

- **M0 — Repository foundation**: COMPLETE, pushed (`62fdfdb`).
- **M1 — Workspace/process discovery**: COMPLETE, pushed (`7f5ec34`).
- **M2 — Local state**: COMPLETE, pushed (`5511eaa`).
- **M3 — Omarchy panel prototype**: COMPLETE, pushed (`5a591d5` + polish `751c7d6`).
- **M4 — Checkpoints + Resume**: COMPLETE (pending commit/push below).

## Completed work

- [x] Environment inspected (Omarchy 4.0.0.alpha, Quickshell bar-widget
  mechanism, Rust 1.98 toolchain, no Go).
- [x] Architecture agreed: Rust daemon + SQLite + QML bar-widget plugin.
- [x] `~/Projects/pitwall` created, connected to `omgxai/pitwall`, M0 pushed.
- [x] M1 read-only discovery: Hyprland 0.56.1 clients, `/proc` truth, agent
  conventions (`org.omarchy.agent` + `OC |` title), foot-cwd-is-`/` finding.
- [x] M1 implemented: `ids` (FNV-1a `proj_`/`sess_` IDs), `platform` trait +
  `platform::linux` (`/proc` walk, minimal `hyprctl` parser, `.git/HEAD`
  branch, git cleanliness), `collector` (join + confidence states +
  `latest_child_start` heuristic), `output` (stable JSON schema v1 + text),
  `pitwall status [--json]`. Zero dependencies.

## Remaining work (next)

- M2: Local state (SQLite + snapshot JSON + systemd user unit).

## Known problems

- None (M1). Noted limits: agent taxonomy is heuristic (opencode verified
  High live; claude/codex/gemini/hermes/aider rules untested live — no such
  processes running); exit-code/failure detection needs M2 event listener;
  `last_activity` is latest-child-start, not true idle.

## Tests performed (M1, 2026-09-09)

- `cargo fmt --check` — clean.
- `cargo test` — 23 passed, 0 failed (fixtures: hypr clients, stat lines,
  mock platform end-to-end, JSON escaping, stable IDs).
- `cargo clippy --all-targets -- -D warnings` — clean.
- Live: `pitwall status --json` exit 0, valid JSON (`python3 -m json.tool`);
  detected opencode/High on pitwall:main, foot→/home/guru, screensaver as
  unknown; IDs stable across runs; ~11ms runtime.
- Secret scan — clean.

## Completed work (M2)

- [x] `rusqlite` 0.32 (bundled) added as sole dependency; `Cargo.lock` now
  tracked (binary crate).
- [x] P1 normalization (`normalize_project_dir`): trailing-slash strip,
  canonicalize, raw fallback. Live IDs unchanged for canonical paths.
- [x] Observer self-exclusion: collector drops its own PID (concrete
  correctness fix — observer no longer forces `last_activity` to "now").
- [x] `store` module: `meta`/`observations`/`sessions` schema v1,
  `user_version` gating, hash-gated writes, newest-100 pruning, corrupt
  quarantine (`pitwall.db.corrupt-<ts>`), newer-version refusal, permission
  degradation without quarantine.
- [x] `state.json` artifact: separate state schema v1 (no cmdlines, no
  evidence, no per-process detail), atomic temp+rename writes.
- [x] `pitwall snapshot [--db/--state/--data-dir]`; defaults to
  `~/.local/share/pitwall/`.
- [x] systemd `pitwall.service` (oneshot) + `pitwall-snapshot.timer` (5min)
  shipped, NOT enabled.
- [x] SECURITY.md persistence boundary; ADR-007; ROADMAP/CHANGELOG updated.

## Remaining work (next)

- M3: Omarchy panel prototype (`dev.pitwall` bar-widget reading state.json).

## Known problems / limits (M2)

- None blocking. Observer effect: separate one-shot invocations always see
  a fresh parent shell, so back-to-back manual snapshots each write; the
  hash gate bites for daemon-style steady state (proven: same-shell repeat
  → "unchanged, no write"). Documented, accepted.
- Non-opencode agent rules still untested live; checkpoints/resume are M4.

## Tests performed (M2, 2026-09-09)

- `cargo fmt --check` — clean.
- `cargo test` — 38 passed, 0 failed (store: fresh/version/insert/
  unchanged/changed/prune/corrupt/newer/perms/privacy; ids: normalization,
  trailing-slash, symlink; output: state-contract scrub).
- `cargo clippy --all-targets -- -D warnings` — clean.
- Live: `status` + `status --json` valid; `snapshot` wrote obs 1;
  same-shell repeat → unchanged; new child process → new observation;
  session/project IDs identical to M1; `state.json` valid state_version 1;
  real-DB strings sweep clean; ~10ms per snapshot.
- Secret scan — clean (results at commit time).

## Completed work (M3)

- [x] M3 design study (installed Omarchy sources only): Style/Color tokens,
  Panel/BarIconButton/WidgetButton/KeyboardPanel/PanelHero/rows/buttons,
  FileView pattern, Nerd glyph coverage verified via `fc-query`.
- [x] `plugin/dev.pitwall/`: manifest + Widget/StateReader/SessionHero/
  SessionRow/ActivityStrip/StateDot + README. Kit-only, zero hardcodes.
- [x] Focus via native `Toplevel.activate()` (app-id + title-tiebreak,
  refuse-on-ambiguous). `hyprctl dispatch` proven unusable here (its Lua
  shorthand rejects all multi-token calls) — documented in plugin README.
- [x] Functional pass live: registered third-party, hot-reload clean, IPC
  open/close, malformed→warn+null, missing→silent null, FileView live
  refresh on snapshot, multi-session rows, stale→urgent tint.
- [x] Visual pass via screenshots: popup (hero/sections/strip/rows) and
  bar indicator (`● pitwall:main +1`) both native-proportioned.
- [x] Dev install at `~/.config/omarchy/plugins/dev.pitwall` (copy, not
  in repo); enabled in right section via `omarchy plugin enable`.

## Remaining work (next)

- M4: project/terminal mapping + checkpoints + Resume.

## Known problems / limits (M3)

- Focus button click path itself not clicked headlessly; matching logic
  follows the first-party `activate()` mechanism and degrades to warn +
  no-op. Recommend a manual click check when convenient.
- Vertical-bar mode falls back to dot-only (untested live, mirrors
  ActiveWindow precedent).
- 6+ session cap (`+N more`, no scroll) untested live (max 3 observed).

## Tests performed (M3, 2026-09-09)

- Rust: `cargo fmt --check` clean, `cargo test` 38 passed, clippy clean
  (no Rust changes in M3; QML is the deliverable).
- Live shell: zero QML errors/warnings for dev.pitwall across reloads;
  `omarchy-shell shell listPlugins` shows dev.pitwall enabled third-party.
- Screenshots inspected: popup open state, bar indicator fresh + stale.
- Secret scan — clean (results at commit time).

## Completed work (M3 polish)

- [x] Pulse softened: 1400ms period, opacity 1.0↔0.65, running-only,
  dead otherwise (unchanged budget).
- [x] State transitions: 160ms color ease on StateDot, 180ms height/color
  ease on ActivityStrip segments (data-change only). Worst case 2
  concurrent animations — within budget.
- [x] Gauge deliberately NOT added: process-count implies false semantics,
  freshness duplicates the strip. Documented in plugin README.
- [x] Focus hover/press: already owned by kit PanelActionButton — no change.
- [x] Live screenshots: working (`●` accent), waiting (`●` urgent + tint,
  genuine STOPped-session capture), idle, stale-tinted, multi-row.
- [x] No Rust / state.json / dependency / font / M4 changes.

## Remaining work (next)

- M4: project/terminal mapping + checkpoints + Resume.

## Known problems / limits (M3 polish)

- Session-state priority is pid-ordered, not severity-ordered: a Running
  process earlier in pid order masks a Stopped one (observed live). No
  Rust changes allowed in this pass — noted for M4 review.
- 6+ cap, vertical-bar dot fallback, physical Focus click: as M3.

## Tests performed (M3 polish, 2026-09-09)

- Rust gate unchanged-green: fmt clean, 38 passed, clippy clean.
- Shell reload clean; QML error sweep clean; secret scan clean.

## Completed work (M4)

- [x] M4a: severity-ordered `derive_session_state` (Stopped > Running >
  Sleeping > Unknown, order-independent) + order/permutation tests.
- [x] `checkpoints` table (schema v2, no FKs, indexes on project/session);
  v1 cache recreated on open (pre-release, documented).
- [x] Triggers: `pitwall checkpoint [--note --session-id]` (manual) +
  disappearance during `snapshot` (one per continuous absence, re-fires
  after reappearance). Retention 25/project + 500 global. Note cap 280.
- [x] Identity docs: sess_* = terminal context (B), not agent run (C) or
  human task (D).
- [x] M4b: `pitwall resume --session-id` (L1 focus via Lua dispatch, L2
  terminal via `xdg-terminal-exec --dir`; strict validation, no fallback,
  never starts agents). `Platform::{launch_terminal, focus_window_address}`.
- [x] state.json v2 (additive `resumable`, cap 10, scrubbed, incl.
  session_id correction). Reader accepts v1|v2.
- [x] M4c: RESUME section + ResumableRow + resume-via-`pitwall` Process;
  hero tooltip "Resume: focus terminal". M3 README dispatch wording fixed.
- [x] Dev finding: shell hot-reload reuses same-URL components — QML
  content edits need `omarchy-restart-shell` (lock-guarded). Documented
  in plugin README.

## Remaining work (next)

- M5 (not yet defined — OpenRouter summaries per roadmap) or release
  packaging. Awaiting instruction.

## Known problems / limits (M4)

- Physical panel-button click not performed headlessly (no input tool);
  click wiring reviewed, CLI focus path proven live. Recommend one manual
  click when convenient.
- 6+ sessions and vertical-bar mode untested live (max 3 sessions seen).
- Session-state pid-ordering fixed; severity model documented.

## Tests performed (M4, 2026-09-09)

- `cargo fmt --check` clean; `cargo test` 62 passed; clippy clean.
- Live: manual checkpoint; disappearance + suppression + refire;
  L1 focus (activewindow verified); L2 terminal-at-dir (plain foot, no
  agent); malformed/unknown/missing-dir refusals (rc 2/1/1, no fallback);
  state v2 + resumable rendered in panel; screenshots inspected.
- Secret scan — clean (results at commit time).

## Completed work (M5a)

- [x] `/proc/PID/exe` basename signal (`RawProcess.exe_name`,
  `ProcessInfo.exe_name`); independent `exe:` evidence in classifier.
- [x] Window roles: `role_for_class` (verified terminal classes;
  else app; empty unknown). Agent inference skipped for App.
- [x] Honest Unknown: terminal + no signals → Unknown/Low with
  `terminal context only`; App → Unknown/Unknown/empty.
- [x] `role` added additively to status JSON + state.json sessions.
- [x] `pitwall agents` (PATH scan over fixed table) + `pitwall models`
  (opencode list; agent-default fallback; refusals). Validated: claude
  `--model`, codex `exec -m`; hermes/aider/gemini excluded (documented).
- [x] Live: chromium/screensaver → app; foot → low honest unknown;
  opencode high unchanged; agents/models commands work on real tools.

## Remaining work (next)

- M5b: derived activity events (read-time diff, no new tables).

## Known problems / limits (M5a)

- Role allowlist is static; exotic terminals classify Unknown (safe:
  inference still runs, panel shows distinctly).
- Claude/codex model LIST unvalidated → Agent default (documented).

## Tests performed (M5a, 2026-09-09)

- `cargo fmt --check` clean; `cargo test` 70 passed; clippy clean.
- Hermetic discovery tests (temp dirs, no PATH mutation).
- Live: agents/models/status verified against real tools (above).

## Exact next step

- Commit + push M5a, then begin M5b (derived events).

## Completed work (M0 archive)

- [x] Environment inspected (Omarchy 4.0.0.alpha, Quickshell bar-widget
  mechanism, Rust 1.98 toolchain, no Go).
- [x] Architecture agreed: Rust daemon + SQLite + QML bar-widget plugin.
- [x] `~/Projects/pitwall` created, `git init -b main`.
- [x] Cargo scaffold, MIT docs set, ADRs 001–006, CI skeleton, packaging stub.

## Tests performed (M0, 2026-09-09)

- `cargo fmt --check` — clean.
- `cargo test` — 1 passed, 0 failed.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo build` + `./target/debug/pitwall --version` → `pitwall 0.1.0`.
- Secret scan — clean (only match is the scanner's own regex in ci.yml).
- `git status` — only intentional M0 files; `target/` ignored; `kerdos` untouched.
