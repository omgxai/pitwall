# Pitwall Workspace State (durable checkpoint)

> The repository is the source of truth. This file is the resume point for
> interrupted sessions (sleep, restart, dropped agent sessions).

## Last verified milestone

- **M0 — Repository foundation**: COMPLETE, pushed (`62fdfdb`).
- **M1 — Workspace/process discovery**: COMPLETE (pending commit/push below).

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

## Exact next step

- Commit + push M1, then await instruction to begin M2.

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
