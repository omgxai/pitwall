# Pitwall
https://github.com/omgxai/pitwall/blob/main/assets/pitwallpixelart.jpeg

**Omarchy's workspace awareness layer for AI-assisted development.**

Pitwall is the human's window into the AI workspace. It answers one
question from the Omarchy desktop:

> What is happening across my AI workspace, what changed while I was
> away, what needs my attention, and where should I continue?

```
SEE → UNDERSTAND → REMEMBER → RESUME → CONTROL → DELEGATE → NOTIFY
```

> **Status:** v0.1.0 in development. M5 + M5g slices (semantic tree,
> assign, notification inbox) implemented and frozen; UI frozen.
> `pitwall status` observes · `snapshot` persists · `checkpoint`
> records · `resume` focuses or reopens · `summarize` asks your
> configured agent (cached locally) · `assign` runs a validated task
> on a live session · `notifications` lists human-relevant events.
> Plan: [docs/NEXT_SPRINT.md](docs/NEXT_SPRINT.md) · [ROADMAP.md](ROADMAP.md)
> · [CHANGELOG.md](CHANGELOG.md).

## What is Pitwall?

Pitwall sits above your existing terminals and AI agents and gives you
**awareness, continuity, context, control, delegation, and
notifications** — from a calm, always-visible Omarchy bar widget.

Pitwall is **not** itself the coding agent. Your agent (OpenCode,
Claude Code, Codex, Hermes, …) remains responsible for inference,
provider, authentication, model, and actual task execution. Pitwall
provides the layer around it: what is running, what it means, what
persisted, and what needs you.

## Why does it exist?

Modern AI development means multiple terminals, multiple projects,
multiple agents, Master/Child agent teams, long-running sessions, and
work that continues while you are away.

The problem is not "where is my terminal?" The problem is "what is
happening across all of it, what changed, what needs me, and where do
I continue?" Pitwall answers that without opening twelve windows.

## Who is it for?

Developers on [Omarchy](https://omarchy.org) Linux running multiple
terminals and AI coding agents in parallel — including anyone running
a Master/Child agent team against one repo.

## What it does today

Omarchy-native workspace panel (`plugin/dev.pitwall`, Quickshell bar
widget): collapsed flag-only button; header with refresh + settings;
AI summary ticker; session rail newest-first with log-scaled duration
bars and R/S/U history segments; hover-highlight + click-pin detail
cards; RESUME history rows.

- **Project grouping, Pitwall-native.** Sessions are grouped by project
  (collapsed by default), ordered by semantic priority + recency.
  AI-agent sessions, workspace/application windows, and Omarchy/system
  entries each render in their own tier (see mental model below).
- **Session timeline bars + evidence.** Duration bars with running /
  sleeping / unknown history. Detail cards show deterministic state,
  agent evidence, window, and project — never AI-guessed identity.
  Unknown renders as unknown with a reason, never a guessed agent.
- **Checkpoints + Resume.** `pitwall checkpoint` records a bounded,
  length-capped note (manual + disappearance triggers, 25/project +
  500 global). `pitwall resume` focuses a live session natively or
  opens a terminal at the validated project dir. It never starts
  agents.
- **Actions: Focus / Stop / Close / Resume.** Explicit clicks only.
  Stop is SIGTERM-only; Close closes the window; Resume
  focuses-or-reopens. Strict validation, no fallbacks.
- **AI summaries (explicit, cached).** `pitwall summarize` or the
  Generate button builds an ephemeral bounded context — structured
  state + derived events + checkpoints + up to 10+10 terminal lines
  where observable, sanitized, 6 sessions / 16KB cap — and hands it to
  your configured local agent (`opencode run`, fixed argv,
  timeout+kill). The agent does the inference; Pitwall caches the
  text result locally by input hash (`state.json` v3 `summary{}`).
  Refresh runs `pitwall snapshot` (state only) and **never** invokes
  AI. Opening, hovering, or selecting never invokes AI. The context
  file (`/run/user/$UID/pitwall/`, 0700/0600, Drop-guard cleanup) is
  temporary — Pitwall does not record terminal conversations.
- **Notifications (bounded inbox).** Sync-derived transitions only:
  session appeared / vanished / stopped, assign completion
  (completion) / failure (attention). Never per-process, per-tick, or
  per-refresh. `pitwall notifications [--unread]` lists;
  `notifications read <id>` marks exactly one row read. The panel
  shows per-entry unread dots, inbox rows in the pinned card (max 3 +
  `+N more`), group unread counts, and a header badge for
  attention/completion only (hidden at zero). Listing or expanding
  never marks read — only an explicit row click does.
- **Validated assignment.** The pinned card's Assign form runs
  `pitwall assign --session-id --role --prompt` (validated foreground
  agent run, no persistence; completion/failure raises a
  notification). Pitwall observes the workforce; it is not the
  workforce.
- **Master/Child workforce awareness.** Pitwall detects and displays
  Master + child agent sessions with titles, evidence, and confidence
  (see [docs/AI_WORKFORCE.md](docs/AI_WORKFORCE.md)).
- **Settings + refresh.** `~/.config/pitwall/config` (0600:
  agent/model/summary_enabled) via `pitwall config get|set`, echoed
  into `state.json` for QML. Refresh = snapshot. Generate = explicit
  AI. Always.

CLI surface (all explicit, no daemons): `status [--json]`,
`snapshot`, `checkpoint`, `resume`, `agents`, `models`, `summarize`,
`assign`, `notifications`, `config`. Exit codes: 0 ok, 1 operational
failure, 2 usage error.

## UI mental model

One terminal/window root = one session. Multiple sessions may share a
project. Children contribute to their parent session — never a flat
PID list.

```
🏁 pitwall · 5
   ├─ Master
   ├─ UI
   └─ Systems

🤖 Work · 3
   ├─ OpenCode
   └─ OpenCode

📁 guru · 3
   ├─ Chromium
   └─ terminal
```

Hierarchy (top to bottom):

1. **PITWALL-NATIVE** — your own Pitwall/Master sessions
2. **AI AGENTS** — detected agent sessions (OpenCode, Claude, …)
3. **WORKSPACE / APPLICATIONS** — editors, browsers, terminals
4. **OMARCHY / SYSTEM** — everything else

Within each tier: newest / most recently active first.

## AI summary vs notification

- **Notification = something happened.** Agent completed grouping
  audit. A session vanished. An assign failed.
- **AI summary = what it means.** Both supporting agents finished
  their audits; remaining work is integration and verification.

Notifications are meaningful, bounded events — not a general event
log.

## Workforce & recipes

Pitwall observes Master/Child agent work and offers a controlled way
to assign validated work to a selected live session. It is the
human's control and awareness surface around the workforce — not the
workforce itself.

Two open, community-oriented directions build on this:

- [AI Workforce Design](docs/AI_WORKFORCE.md) — how humans + agent
  teams coordinate through Git with Pitwall as shared awareness.
- [Pitwall Recipes](docs/RECIPES.md) — reusable, Git-friendly
  human+AI workflows. Document practiced runs, not theory. No
  marketplace, no accounts.

## Privacy

Local-first. No mandatory account, no provider credentials held by
Pitwall, no OpenRouter gateway, no cloud dependency, no telemetry by
default. Workspace data stays on-device in SQLite.

AI context is sanitized, bounded, and ephemeral. Pitwall does not
persist terminal transcripts, full argv, environment variables,
credentials, or secrets — enforced by unit tests (secret-bearing
fixtures never reach DB bytes or `state.json`). See
[SECURITY.md](SECURITY.md).

## What Pitwall is not

- an AI coding agent
- a replacement for your terminal
- a provider/authentication gateway
- a generic process monitor
- a SaaS dashboard
- a mandatory cloud service

Pitwall is the awareness/control layer around your existing AI
workspace.

## Roadmap

Implemented (frozen): M0–M5f, M5g tree + assign + notification inbox.
See [CHANGELOG.md](CHANGELOG.md).

Next vs future are tracked outside this file:

- [ROADMAP.md](ROADMAP.md) — milestones, governance, non-goals
- [docs/NEXT_SPRINT.md](docs/NEXT_SPRINT.md) — P0 hardening → P7 sharing
- [docs/AI_WORKFORCE.md](docs/AI_WORKFORCE.md) — workforce direction
- [docs/RECIPES.md](docs/RECIPES.md) — recipe direction

Future directions (planned, not built): contextual "Ask Pitwall"
interaction, context-window monitoring and handover, project/recipe
bootstrap, agent communication channels, multi-node Pitwall,
temporary remote summary sharing.

## Branding

- `assets/flag.svg` (+ `flag-64.png`): the 16px checkered flag — the
  compact panel identity.
- `assets/pitwallpixelart.jpeg`: pixel-art pit-wall scene for project
  and discovery use (releases, docs). Not a UI asset; the README does
  not depend on it.

## Install

No installer yet — it arrives with M6 packaging. `packaging/install.sh`
is a stub that says so (verified). Target flow:

```bash
git clone <repo-url> ~/Projects/pitwall
cd ~/Projects/pitwall
./packaging/install.sh
omarchy plugin enable dev.pitwall --after omarchy.agents
```

## Develop

Requirements: Rust stable (1.80+), standard Linux tools. Sole
dependency: `rusqlite` (bundled SQLite).

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

Panel development: copy `plugin/dev.pitwall` to
`~/.config/omarchy/plugins/` and enable with `omarchy plugin enable`.
Current dev state: M5 UI frozen; see `docs/NEXT_SPRINT.md` before
changing anything.

## Contribute

See [CONTRIBUTING.md](CONTRIBUTING.md). Please read the
[Code of Conduct](CODE_OF_CONDUCT.md) first.

## License

MIT — see [LICENSE](LICENSE).
