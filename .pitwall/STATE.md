# Pitwall Workspace State (durable checkpoint)

> The repository is the source of truth. This file is the resume point for
> interrupted sessions (sleep, restart, dropped agent sessions).

## Last verified milestone

- **M0 — Repository foundation**: COMPLETE, pushed (`62fdfdb`).
- **M1 — Workspace/process discovery**: COMPLETE, pushed (`7f5ec34`).
- **M2 — Local state**: COMPLETE, pushed (`5511eaa`).
- **M3 — Omarchy panel prototype**: COMPLETE, pushed (`5a591d5` + polish `751c7d6`).
- **M4 — Checkpoints + Resume**: COMPLETE, pushed (`c76dea5`).
- **M5a — Evidence/roles + agent discovery**: COMPLETE, pushed (`3f04f45`).
- **M5c — Ephemeral AI workspace context**: COMPLETE, pushed (`99c6fae`).
- **M5d Part 1 — Summary persistence + state v3**: COMPLETE, pushed (`41aaa03`).
- **M5e — Pixel flag identity**: COMPLETE, pushed (`63a85e8`).
- **M5f — Session timeline rail**: COMPLETE (pending commit/push below).

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

## Completed work (M5c)

- [x] Platform: `process_io` (/proc/PID/io) + `terminal_text` (kitty
  socket best-effort w/ 2s timeout + thread-drained pipe; foot/exotic →
  explicit Unavailable; pty reads banned).
- [x] Context builder: allowlisted fields only, 10+10 lines, 4KB/window,
  6 sessions (terminal-first), 16KB cap + honest truncation count.
- [x] Scrub: secret-shape matrix (incl. PEM/Bearer/bare forms w/ shape
  heuristic preserving plain words), control stripping, canary tests.
- [x] Ephemeral lifecycle: /run/user/$UID/pitwall 0700, O_EXCL 0600
  unpredictable names (/dev/urandom), RAII Drop + explicit close;
  cleanup verified on success/failure/timeout/drop paths.
- [x] Derived events (M5b gap-fill): read-time diff over retained
  observation + checkpoints_since; no new tables.
- [x] `pitwall summarize [--agent --model --dir --timeout --dry-run]`:
  fixed argv (`opencode run --format json -m -f`), model validation,
  timeout+kill, JSON-event text extraction (tool payloads ignored),
  exit 0/1/2 contract. Live run on free-tier model returned an honest,
  evidence-graded summary; agent modified nothing; no residue/strays.
- [x] Kitty live validation: PENDING (no kitty session available;
  fixture-tested degradation only).

## Remaining work (next)

- M5d+ (summary cache/state.json/panel/identity) or release packaging.
  Awaiting instruction. Do NOT start unapproved scope.

## Known problems / limits (M5c)

- Foot terminal text unobservable by design (honest marker, not error).
- Kitty path implemented but not live-validated (no kitty running).
- Live agent summary spends user agent budget (used free-tier model
  for the single smoke test); auto-generation stays banned.
- Full `cargo test` takes ~30s (retention/prune volume + timeout test).

## Tests performed (M5c, 2026-09-09)

- `cargo fmt --check` clean; `cargo test` 88 passed; clippy
  (--all-targets --all-features) clean.
- Live: dry-run context (roles/io/events/unavailable markers),
  agent summary rc=0 with graded uncertainty, ctx cleanup, schema
  unchanged (4 tables), secret scan clean.

## Exact next step

## Completed work (M5d Part 1)

- [x] `summaries` table (input_hash PK, text, model, created_at) + v3
  additive migration (CREATE-IF-NOT-EXISTS; v2 rows preserved; v1
  still recreates). Column set pinned by test (no evidence columns).
- [x] `input_hash`: FNV over structured context only (terminal text
  excluded by construction); same workspace → same hash.
- [x] Cache-first `summarize`: lookup before staging/spawning; hit
  prints cached text (stderr notes generated/cached); miss stores.
- [x] state.json v3 `summary{text,model,created_at,input_hash,status}`;
  `snapshot` carries latest cached summary; failures write short
  fixed error states, never internals.
- [x] StateReader accepts v1|v2|v3 + strict `summary` property (no
  visuals — M5f). Panel verified visually unchanged on v3.
- [x] Fixed a real bug found live: doubled top-level brace in v3
  writer (caught by new whole-document well-formedness test, now
  covering all artifacts).

## Remaining work (next)

- M5d Part 2 / M5e / M5f per approved plan. Awaiting instruction.
  Do NOT start unapproved scope.

## Tests performed (M5d Part 1, 2026-09-10)

- `cargo fmt --check` clean; `cargo test --all` 96 passed; clippy
  (--all-targets --all-features) clean.
- Live: dry-run; free-tier generate (~16s) + cache behavior; v3 state
  valid with ready summary; panel loads v3 with zero warnings;
  screenshots inspected; schema verified (5 tables, no text columns).
- Secret scan — clean (results at commit time).

## Completed work (M5e)

- [x] Original 16x16 pixel checkered flag (`assets/flag.svg` + 64px
  PNG export; Omarchy default fg/muted; transparent bg; crispEdges).
  Pixel-verified at 16px (pure 2px cells, no blending).
- [x] Panel header mark: 14px unsmoothed Image + PITWALL caption;
  wordmark-only fallback (quadrant glyphs absent from font).
  Runtime copy derived from `assets/` at install; documented.
- [x] Screenshot-verified on live desktop: recognizable checker,
  native proportions, no layout regression, zero shell warnings.

## Remaining work (next)

- M5f (scrollable panel + summary/gear UI) per approved plan.
  Awaiting instruction. Do NOT start unapproved scope.

## Tests performed (M5e, 2026-09-10)

- `cargo fmt --check` clean; 96 tests passed; clippy clean.
- Asset pixel dump + live screenshots inspected; secret scan clean.

## Completed work (M5f)

- [x] Rail UI: collapsed flag-only button; header (flag + PIT/WALL +
  gear); AI summary ticker (ping-pong, hover-pause, Generate/error
  states); session bars (log-scaled duration, R/S/U segments, recency
  order); hover-preview + pinned toast with deterministic detail;
  actions (Focus/Stop-SIGTERM/Close/Resume) on explicit clicks only.
- [x] Settings view: Agent/Model Dropdowns + Summary Toggle, persisted
  via `pitwall config` + state.json echo; models listed per agent.
- [x] Backend: session age/history/root_pid in state.json; minimal
  `config.rs` + `pitwall config get/set`; resumable short-hash display.
- [x] Removed replaced components (ActivityStrip/Hero/rows/StateDot).
- [x] Crash notification during testing investigated: restart-teardown
  artifact (SI_TKILL on old shell), not a QML bug; current shell healthy.

## Remaining work (next)

- M5g hardening/release per approved plan. Awaiting instruction.
  Do NOT start unapproved scope.

## Known problems / limits (M5f)

- Physical click/tap not performed headlessly (no input tool); all
  click paths reviewed + CLI equivalents proven live (focus, resume,
  SIGTERM argv, config set). Recommend manual click-through.
- 6+ sessions, vertical bar, ticker pause-on-hover: code-reviewed,
  untested live (max 3 sessions observed).
- No OS reduced-motion signal on this stack; mitigation is slow
  speeds + hover pause + toggle-gated summary.

## Tests performed (M5f, 2026-09-10)

- `cargo fmt --check` clean; 103 tests passed; clippy clean.
- Live: collapsed/expanded/ticker/bars/toast/RESUME screenshots;
  zero QML errors; resume/focus/stop-argv/config proven via CLI;
  schema verified (5 tables + summaries, no text columns).

## Completed work (M5f polish / interaction fix)

- [x] Pin-stable interaction: hover highlights only; click pins; toast
  card lives inside its own bar delegate (stable geometry, no dead
  zone, no overlap); Esc clears selection first, then closes.
- [x] Header: single-word PITWALL (split-color) + refresh (md-refresh,
  spins while running) + gear. Refresh spawns `pitwall snapshot`
  (state only, never AI) on expand + manual click.
- [x] Density: AI SUMMARY caption removed; RESUME heading removed
  (history rows flow muted); toast is the only detail surface.
- [x] Actions per selection: Focus/Stop-SIGTERM/Close (live),
  Resume (history); explicit clicks; validated inputs; no delete.
- [x] Summary trash: `summarize --clear` (cache-only) + panel button.
- [x] Found + fixed live: Repeater `onHovered` vs `hoverChanged`
  signal mismatch (widget failed to instantiate; caught via journal).
- [x] Screenshots: rail + ticker + RESUME rows + collapsed flag, all
  native-proportioned, zero QML errors.

## Known problems / limits (polish)

- Physical click/tap/hover/scroll untestable headlessly (no input
  tool): click/hover/scroll paths code-reviewed against kit patterns;
  CLI equivalents proven (focus, resume, SIGTERM argv, config).
- Settings view + selected card visuals not screenshotted (need a
  click); components use verified kit APIs; panel opens error-free.
- 6+ sessions and vertical bar untested live.

## Tests performed (polish, 2026-09-10)

- `cargo fmt --check` clean; 104 tests passed; clippy clean.
- No Rust behavior change except `summarize --clear` (+1 test).

## Exact next step

- Commit + push polish, then STOP (no M5g without instruction).

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
