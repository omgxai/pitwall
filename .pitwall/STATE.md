# Pitwall Workspace State (durable checkpoint)

> The repository is the source of truth. This file is the resume point for
> interrupted sessions (sleep, restart, dropped agent sessions).

## Last verified milestone

- **M0 — Repository foundation**: COMPLETE (pending verification below).

## Completed work

- [x] Environment inspected (Omarchy 4.0.0.alpha, Quickshell bar-widget
  mechanism, Rust 1.98 toolchain, no Go).
- [x] Architecture agreed: Rust daemon + SQLite + QML bar-widget plugin.
- [x] `~/Projects/pitwall` created, `git init -b main`.
- [x] Cargo scaffold, MIT docs set, ADRs 001–006, CI skeleton, packaging stub.

## Remaining work (next)

- M1: workspace/process discovery (`/proc` + `hyprctl clients -j`).

## Known problems

- None (M0).

## Tests performed (M0, 2026-09-09)

- `cargo fmt --check` — clean.
- `cargo test` — 1 passed, 0 failed.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo build` + `./target/debug/pitwall --version` → `pitwall 0.1.0`.
- Secret scan — clean (only match is the scanner's own regex in ci.yml).
- `git status` — only intentional M0 files; `target/` ignored; `kerdos` untouched.

## Exact next step

- Begin M1: implement `collector` module with process + Hyprland client
  discovery and `pitwall status --json`.
