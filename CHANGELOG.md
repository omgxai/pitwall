# Changelog

All notable changes to Pitwall are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning follows
[SemVer](https://semver.org/) once the first release is tagged.

## [Unreleased]

### Added
- Docs governance: disciplined public roadmap (M5 frozen, hardening +
  AI Workforce/Recipes directions), refreshed README, new
  `docs/AI_WORKFORCE.md` + `docs/RECIPES.md`, corrected security
  wording to agent-delegated summaries.
- M5f final polish (UI freeze): glyph-only confidence, process counts,
  expander affordances, clamped ticker with attention line, state-word
  detail cards, trash removal; grouping verified window-rooted.
- M5f interaction polish: pin-stable hover/click model, in-delegate
  detail cards, single-word header + refresh + gear, caption-free
  ticker, heading-free rail, validated action row, summary trash via
  `summarize --clear`.
- M5f session timeline rail: collapsed flag button; header + gear;
  AI summary ticker; duration/history bars newest-first; hover/pin
  toast with deterministic detail; Focus/Stop/Resume/Close actions;
  settings (agent/model/toggle) via `pitwall config`; session age,
  history, root_pid in state.json.
- M5e pixel flag identity: original 16x16 checkered `assets/flag.svg`
  (+ 64px PNG), 14px panel header mark with wordmark fallback,
  screenshot-verified, no layout/semantics changes.
- M5d Part 1 summary persistence + state v3: `summaries` cache table
  (input_hash PK; additive v3 migration preserving v2 rows), hash-gated
  cache-first `summarize`, state.json v3 `summary{}` (ready/error/null),
  strict StateReader `summary` property (no visuals yet).
- M5c ephemeral AI context: `/proc` IO signals, best-effort kitty text
  with honest foot Unavailable, bounded context builder (10+10 lines,
  4KB/window, 6 sessions, 16KB cap), secret scrubbing, RAII ephemeral
  files under /run/user, read-time derived events (no new tables),
  `pitwall summarize` via configured agent (fixed argv, timeout, text
  extraction). No OpenRouter/keys/cloud; no schema changes.
- M5a evidence + discovery: `/proc` exe basename signal, window roles
  (terminal/app/unknown; no agent inference for apps), honest Unknown
  (`terminal context only`), additive `role` in JSON outputs,
  `pitwall agents` / `pitwall models` over a fixed validated table.
- M4 checkpoints + Resume: severity-ordered session state; `checkpoints`
  table (schema v2, manual/disappearance triggers, 25/project + 500
  retention, 280-char notes); `pitwall checkpoint` / `pitwall resume`
  (Levels 1–2, strict validation, no fallbacks, never agents);
  `Platform::{launch_terminal, focus_window_address}`; state.json v2
  `resumable`; panel RESUME section; ADR-008.
- M3 panel instrumentation polish: subconscious 1400ms working pulse,
  160–180ms state/data transitions (budget-held), ActivityStrip honesty
  kept, gauge deliberately omitted (documented). No Rust/state.json
  changes.
- M3 Omarchy panel: `plugin/dev.pitwall/` (manifest, Widget, StateReader,
  SessionHero, SessionRow, ActivityStrip, StateDot, README). Kit-only QML,
  FileView-driven state.json, native `Toplevel.activate()` Focus,
  animation budget enforced, screenshot-verified on live desktop.
- M2 local continuity cache: `store` module on `rusqlite` (bundled, sole
  dependency), `meta`/`observations`/`sessions` schema v1, hash-gated
  writes, newest-100 pruning, corrupt-quarantine + newer-version refusal,
  `state.json` artifact (state schema v1, scrubbed), `pitwall snapshot`,
  P1 project-dir normalization, observer self-exclusion, systemd unit +
  timer (shipped disabled), ADR-007, SECURITY persistence boundary.
- M1 workspace discovery: `platform` trait + Linux impl, `collector` with
  confidence states and stable `proj_`/`sess_` IDs, `pitwall status [--json]`
  (JSON schema v1), 23 unit tests with fixtures/mocks.
- M0 project foundation: Rust crate scaffold (`pitwall` CLI + `pitwall_lib`),
  MIT license, README, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, ROADMAP,
  CHANGELOG, `.gitignore`, GitHub templates, CI skeleton, ADRs 001–006,
  `.pitwall/STATE.md`, systemd unit + install script stubs.
