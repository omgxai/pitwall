# Pitwall

**AI workspace awareness for developers.**

Modern AI development spreads across terminals, shell sessions, projects, and
CLI agents (Hermes, Claude Code, OpenCode, …). Pitwall is a lightweight,
local-first awareness and continuity layer above them: one glance tells you
what is happening across your workspace, and when you return to your machine
you can resume without reconstructing context.

> **Status:** v0.1.0 in development, M5 UI frozen. `pitwall status`
> observes; `pitwall snapshot` persists; `pitwall checkpoint` records;
> `pitwall resume` focuses or reopens; `pitwall summarize` asks your
> configured agent for a workspace summary (cached locally);
> `pitwall assign` runs a validated task on a live session;
> `pitwall notifications` lists marks for human-relevant events;
> the Omarchy bar widget shows sessions, checkpoints, Resume actions,
> and the notification inbox.
> Next sprint plan: [docs/NEXT_SPRINT.md](docs/NEXT_SPRINT.md).
> See [ROADMAP.md](ROADMAP.md).

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
AI summary delegates to your own configured agent (OpenCode, …) with a
small sanitized context — no provider keys in Pitwall, no cloud, no
telemetry, never secrets. See [SECURITY.md](SECURITY.md).

## AI workforce & recipes

Pitwall pairs with an open pattern for humans working alongside AI
agents ([AI Workforce Design](docs/AI_WORKFORCE.md)) and a community
format for reusable workflows ([Pitwall Recipes](docs/RECIPES.md)).
Bring your own workflow: build it, test it, share the recipe.

## Branding

- `assets/flag.svg` (+ `flag-64.png`): the 16px checkered flag, the
  compact panel identity.
- `assets/pitwallpixelart.jpeg`: pixel-art pit-wall scene for docs
  and discovery (README, release notes). Not a UI asset; the panel
  is not designed around it.

## Contribute

See [CONTRIBUTING.md](CONTRIBUTING.md). Please read the
[Code of Conduct](CODE_OF_CONDUCT.md) first.

## License

MIT — see [LICENSE](LICENSE).
