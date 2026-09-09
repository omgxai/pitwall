# Pitwall Roadmap

Milestones are sequential and independently testable. Do not start M(n+1)
until M(n) is stable and committed.

- [x] **M0 — Repository foundation** (current): Cargo scaffold, MIT license,
  docs, ADRs, CI skeleton, `.pitwall/STATE.md`, packaging stub.
- [x] **M1 — Workspace/process discovery**: `/proc` + `hyprctl clients -j`
  collector; `pitwall status --json` lists real terminals/sessions/processes.
- [x] **M2 — Local state**: SQLite continuity cache (`meta`, `observations`,
  `sessions`; hash-gated writes; newest-100 pruning), `state.json` artifact
  (state schema v1), `pitwall snapshot`, systemd unit + timer (shipped,
  not enabled by default).
- [x] **M3 — Omarchy panel prototype**: `dev.pitwall` bar-widget (indicator +
  KeyboardPanel popup) reading `state.json`; Focus via native activate;
  dev-installed, screenshot-verified.
- [ ] **M4 — Project/terminal mapping + checkpoints + Resume**: git-aware
  project detection, checkpoint records, focus/open actions.
- [ ] **M5 — OpenRouter BYO integration**: optional cached one-line summaries
  with deterministic fallback; key via env/keyring, never logged.
- [ ] **M6 — Packaging + first public release**: `install.sh`, docs pass,
  clean-room install verified, v0.1.0 tag.

## Explicitly out of scope for v0.x

macOS/Windows ports, cloud backend, Supabase, mobile apps, full MCP server,
secret vault/rotation, billing, plugin marketplace, autonomous actions.
These are architected-for (see ADRs), not built.
