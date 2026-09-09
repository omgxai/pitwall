# Pitwall

**AI workspace awareness for developers.**

Modern AI development spreads across terminals, shell sessions, projects, and
CLI agents (Hermes, Claude Code, OpenCode, …). Pitwall is a lightweight,
local-first awareness and continuity layer above them: one glance tells you
what is happening across your workspace, and when you return to your machine
you can resume without reconstructing context.

> **Status:** v0.1.0 scaffold (M0). The collector, panel plugin, and summaries
> are not implemented yet. See [ROADMAP.md](ROADMAP.md) and
> [.pitwall/STATE.md](.pitwall/STATE.md).

## What is Pitwall?

The human's window into the AI workspace: **visibility, context, memory,
handover, control.**

## Why does it exist?

Developers lose time re-discovering state: which terminal holds the auth work,
which agent is waiting, what failed while away. Pitwall records lightweight
checkpoints and shows live workspace state in the Omarchy top bar.

## Who is it for?

Developers on [Omarchy](https://omarchy.org) Linux running multiple
terminals and AI coding agents in parallel.

## What problem does it solve?

"I left my machine — what was I doing and where do I continue?" Pitwall
answers with project, agent, terminal, branch, last activity, a one-line
summary, and a Resume action.

## Why Omarchy?

Omarchy's Quickshell bar-widget plugin system is the ideal native home for a
calm, always-visible indicator. Pitwall v0.x is Omarchy-first; the core is
modular so other OS adapters can come later.

## What Pitwall is NOT

- Not an AI coding agent, not an orchestrator, not a terminal replacement.
- No autonomous actions, no cloud backend, no telemetry, no mandatory accounts.

## Install

M0 has no installer yet. Target flow (M6):

```bash
git clone <repo-url> ~/Projects/pitwall
cd ~/Projects/pitwall
./packaging/install.sh
omarchy plugin enable dev.pitwall --after omarchy.agents
```

## Develop

Requirements: Rust stable (1.80+), standard Linux tools.

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

Omarchy panel development (from M3): copy `plugin/dev.pitwall` to
`~/.config/omarchy/plugins/` and enable with `omarchy plugin enable`.

## Privacy & security

Local-first. Workspace data stays on your machine (SQLite). The optional
OpenRouter summarizer (deferred to M5) sends only minimal, user-approved
context and never secrets. See [SECURITY.md](SECURITY.md).

## Contribute

See [CONTRIBUTING.md](CONTRIBUTING.md). Please read the
[Code of Conduct](CODE_OF_CONDUCT.md) first.

## License

MIT — see [LICENSE](LICENSE).
