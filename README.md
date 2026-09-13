# Pitwall

<div align="center">
  <br />
  <img width="512" height="512" alt="Pitwall pixel art" src="assets/pitwallpixelart.jpeg" />
  <br />
  <br />
</div>

**Omarchy's workspace awareness layer for AI-assisted development.**

Pitwall is the human's window into the AI workspace. It answers one
question from the Omarchy desktop:

> What is happening across my AI workspace, what changed while I was
> away, what needs my attention, and where should I continue?

```
SEE → UNDERSTAND → REMEMBER → RESUME → CONTROL → DELEGATE → NOTIFY
```

> **Status:** v0.1.0. M5 + M5g (semantic tree, assignment,
> notification inbox) and M6 user-local packaging are implemented; the UI
> remains deliberately compact and Omarchy-native.
> `pitwall status` observes · `snapshot` persists · `checkpoint`
> records · `resume` focuses or reopens · `summarize` asks your
> configured agent (cached locally) · `chat` opens a native terminal
> conversation about the workspace · `assign` runs a validated task
> on a live session · `notifications` lists human-relevant events.
> M8 (Pitwall Chat + the corrected AI brief ticker) is implemented but
> **not yet verified on an Omarchy runtime** — see
> [Pitwall Chat](#pitwall-chat).
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

## How it works

Pitwall keeps the observation path local and explicit:

```text
Hyprland + /proc
      -> bounded collector
      -> hash-gated SQLite continuity + state.json
      -> Omarchy Quickshell panel
      -> optional, user-triggered agent summary
```

The panel reads the generated artifact; it does not inspect processes or
infer meaning itself. The optional systemd timer runs the same oneshot
snapshot every 30 seconds. AI summaries use a temporary sanitized context,
are cached by deterministic input hash, and never become a background agent.

## Who is it for?

Developers on [Omarchy](https://omarchy.org) Linux running multiple
terminals and AI coding agents in parallel — including anyone running
a Master/Child agent team against one repo.

## What it does today

Omarchy-native workspace panel (`plugin/dev.pitwall`, Quickshell bar
widget): collapsed flag-only button; header with refresh + settings;
AI brief ticker; session rail newest-first with log-scaled duration
bars and R/S/U history segments; hover-highlight + click-pin detail
cards; RESUME history rows; Open Chat control.

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
  focuses-or-reopens. Strict validation, no fallbacks. Failed targets and
  command results remain visible as compact, non-blocking panel feedback.
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
- **AI brief ticker (complete text, one direction).** The brief area
  renders the *whole* summary with eliding disabled — no truncation
  marker, no filler, nothing clipped away. Motion is continuous
  right-to-left only: each pass starts with the first character at the
  right edge and runs until the last character is fully past the left
  edge, then a fresh pass enters from the right again. Pass duration is
  `(content width + viewport width) / 180 px per second`, taken from
  measured geometry and never from the summary's character count, so a
  long brief and a short one scroll at the same speed. Hovering the
  brief surface or expanding it pauses the pass in place; it resumes
  from the offset it was holding, and the offset survives the panel
  tearing its content down and being reopened. A new summary abandons
  the pass in flight and restarts from the right.
- **Pitwall Chat (native terminal).** `pitwall chat` opens a real
  terminal window and talks to you in it, about the workspace Pitwall
  already observes. The panel's `Open Chat` control launches the same
  command. Asking a question observes only; exactly one in-chat command
  changes workspace state. See [Pitwall Chat](#pitwall-chat).
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
- **Live observation.** The optional user systemd timer runs a bounded
  read-only snapshot every 30 seconds. Hash-gated persistence avoids writes
  when nothing changed, while the panel watches `state.json` and updates as
  soon as a new artifact is published. Manual refresh remains available.
- **Settings + refresh.** `~/.config/pitwall/config` (0600:
  agent/model/summary_enabled) via `pitwall config get|set`, echoed
  into `state.json` for QML. Refresh = snapshot. Generate = explicit
  AI. Always.

CLI surface (all explicit, no daemons): `status [--json]`,
`snapshot`, `checkpoint`, `resume`, `chat`, `agents`, `models`,
`summarize`, `assign`, `notifications`, `config`, `doctor`. Exit codes:
0 ok, 1 operational failure, 2 usage error. `pitwall doctor` is a read-only
installation report: it checks the binary, data/config artifacts, plugin
path, and optional user timer without creating state or enabling services.

## Pitwall Chat

```bash
pitwall chat                        # about the whole workspace
pitwall chat --session sess_<hex>   # scoped to one live session
```

A chat is a foreground conversation in a real terminal window. It has no
daemon and no background process: the command *is* the chat, and the chat
ends when you type `/exit` or close the window. Exit codes follow the rest
of the CLI — **0** for a normal end, **1** for an operational failure,
**2** for a usage error.

Refusals are decided in a fixed order, and the usage group is decided
first, so a malformed `--session` exits 2 even when the configured harness
is also missing:

1. the option parse — `chat` accepts only `--session`, and `--session`
   with nothing after it is refused rather than treated as a
   whole-workspace chat (exit 2);
2. the `--session` shape, `sess_[0-9a-f]{16}` (exit 2);
3. the configured model — empty means "the harness default", anything else
   must pass the existing model validator (exit 2);
4. the configured harness must be a known agent and installed here
   (exit 1);
5. a `--session` target must be currently live, and the chat's directory
   (the scoped session's project → the most recently active observed
   project → `$HOME`) must be absolute and must exist. A directory that
   fails validation is named, never quietly swapped for the next candidate
   (exit 1);
6. a free chat number in `001..999` (exit 1).

Nothing environmental is touched until the usage group has passed, and on
every refusal path no terminal is opened, no harness is invoked, no context
file is written, and no chat number is claimed.

### Identity

Each chat takes the lowest free number and sets its own window title with
OSC 2:

```text
Pitwall Chat 001 · opencode · provider/model · pitwall
```

That title is what you see in the Omarchy window switcher. It is also half
of how Pitwall recognises its own chat window: discovery needs **both** the
reserved title grammar *and* a chat-number lease whose owner pid is a live
`pitwall chat` process inside that window's process tree. Printing the
title into some other terminal is not enough — a spoofed title alone never
becomes a chat entry. Details and consequences:
[ADR-009](docs/adr/ADR-009-chat-identity.md).

Discovered chats appear in the panel as `pitwall-native` entries and
disappear when their terminal closes. Nothing about a chat is persisted.

### Configuration is captured once

The harness and model are read from `~/.config/pitwall/config` exactly once,
at startup, and are then immutable for the life of that chat — a later
`pitwall config set` cannot reach a running chat. The next chat picks up the
changed values. The header and the window title show what that chat actually
holds.

### Context

Chat sees the workspace only through the same bounded, scrubbed
`SummaryContext` that `pitwall summarize` uses — observed sessions, derived
events, checkpoints, scrubbed notification facts. It gets no raw snapshot.

Every turn stages that document as one pid-tagged `0600` file under
`/run/user/$UID/pitwall/` (0700, tmpfs). It then reaches the harness either
by path (opencode, via `-f`) or on the child's stdin (claude, codex) —
**never in argv**, which is world-readable through `/proc`. The file is
removed on success, refusal, spawn failure, non-zero exit, empty answer and
timeout, and by a `Drop` guard on any path that does not reach the explicit
cleanup.

No conversation is stored. Turns live in the terminal and die with it;
`/clear` clears the screen, not a record, because there is no record.

### Asking is not acting

Anything that does not start with `/` is a question. A question observes
only: it is answered from the bounded context, no workspace action is
derived from its wording, and phrasing it as an instruction does not make it
one. The in-chat vocabulary is closed — these six entries and nothing else:

| Command | Effect | Does |
|---|---|---|
| `/help` | read-only | list these commands |
| `/context` | read-only | show the bounded context this chat can see |
| `/sessions` | read-only | show the observed sessions in that context |
| `/clear` | read-only | clear the conversation shown here (nothing was stored) |
| `/resume [session-id]` | **CHANGES WORKSPACE STATE** | resume a session: focus its window, or open one terminal |
| `/exit` | read-only | end this chat |

`/resume` is the single command that changes workspace state, and the
in-chat listing marks it that way. It runs the existing `resume` path with a
fixed argument vector and no shell, and refuses malformed, non-live and
non-resumable targets. `/exit` ends the chat but changes nothing in the
workspace, so it is marked read-only alongside the informational entries.
A `/`-prefixed word outside this list runs nothing and prints the list.

### The panel control

The expanded panel carries an `Open Chat` control. It runs the CLI with
fixed argv and nothing else: `[pitwall, chat]`, or
`[pitwall, chat, --session, <id>]` when a **live** session is pinned. A
pinned resumable names a vanished session, so it cannot be a chat context —
the control says so before you click, and an invalid session id refuses
rather than quietly falling back to workspace context. Rendering, hovering
or activating the control invokes no inference.

### Terminal capabilities and limits

- **Inline branding is decoration, never a prerequisite.** The chat header
  probes the terminal's graphics protocol at runtime and shows the Pitwall
  flag inline where it can. Where it cannot — no graphics protocol, a
  missing asset, or a Sixel-only terminal, since Pitwall carries no Sixel
  encoder — the header is textual and the chat is fully functional. Absence
  is normal and is not reported as a problem.
- **A chat started by hand inside a tmux pane is not discovered as a
  chat.** Its process hangs off the tmux server rather than the window
  client, and tmux owns the title, so neither identity signal holds. It
  shows up as the ordinary terminal session it is. Chats that Pitwall
  launches are direct children of the terminal, so this affects manual tmux
  use only.
- **Terminals are opened through Omarchy's terminal path, not a named
  emulator.** One launch path (`xdg-terminal-exec`, fixed argv, no shell)
  serves both `resume` and chat. Pitwall names no emulator, requires none,
  and reads no emulator identity from the environment. One known argv
  discrepancy in that path is documented in
  [ADR-009](docs/adr/ADR-009-chat-identity.md) and deliberately preserved
  so `resume`'s argv stays byte-identical.
- **Abnormal termination can leave one temporary file.** A `SIGINT`
  delivered while the harness is running may leave a single pid-tagged
  `0600` context file in the runtime directory (tmpfs). The next chat
  startup sweeps orphans whose owner is gone; logout clears the rest.
  `/exit` and closing the window are clean paths.

### Verification status

Chat and the corrected ticker are implemented, with unit and property tests
covering the deterministic parts (title grammar, number allocation,
descriptor immutability, argv construction, input classification, privacy
scrubbing, state round trip). **Nothing here has been observed on an
Omarchy runtime.** The work was written on macOS with no Rust toolchain and
no QML tooling, so it has not been compiled and no test has been executed.
Still to confirm on target: the terminal abstraction's command
pass-through, the window title in the switcher, chat discovery in the panel,
the header's inline and textual forms, concurrent chats `001`/`002`/`003`,
and codex's stdin delivery. Treat every runtime claim in this section as
designed behaviour pending that check.

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
fixtures never reach DB bytes or `state.json`). Chat holds the same
line: it reads the workspace only through the bounded `SummaryContext`,
its document never travels in argv, and no conversation is stored
anywhere — turns die with the terminal. See [SECURITY.md](SECURITY.md).

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

Future directions (planned, not built): context-window monitoring and
handover, project/recipe bootstrap, agent communication channels,
multi-node Pitwall, temporary remote summary sharing.

M7 hardened summary freshness and passed summary generation through one
bounded `SummaryContext` containing observed sessions, derived events,
checkpoints, and scrubbed notification facts; terminal text remains confined
to the existing ephemeral document path. The panel presents that
interpretation as a compact `AI BRIEF`, kept visually subordinate to Pitwall
identity and workspace groups.

M8 builds on that context in two places: the AI brief ticker now scrolls the
complete brief in one direction from measured geometry, and `pitwall chat`
turns the same bounded context into an interactive, observe-only
conversation in a native terminal — with a panel `Open Chat` control and one
explicitly entered command that may change workspace state. External
communication channels are still not implemented. M8 has not been verified
on an Omarchy runtime; see [Pitwall Chat](#pitwall-chat).

## Branding

- `assets/flag.svg` (+ `flag-64.png`): the 16px checkered flag — the
  compact panel identity.
- `assets/pitwallpixelart.jpeg`: pixel-art pit-wall scene for project
  and discovery use (releases, docs). Not a UI asset; the README does
  not depend on it.

## Install

The supported install is user-local and does not require root. From a Pitwall
checkout:

```bash
git clone <repo-url> ~/Projects/pitwall
cd ~/Projects/pitwall
./packaging/install.sh
```

The installer builds a release binary at `~/.local/bin/pitwall`, installs the
Omarchy plugin at `~/.config/omarchy/plugins/dev.pitwall`, and installs the
optional user units at `~/.config/systemd/user/`. It is safe to run again.
The timer is installed but disabled by default. To enable live 30-second
snapshots:

```bash
systemctl --user enable --now pitwall-snapshot.timer
omarchy plugin enable dev.pitwall --after omarchy.agents
```

Run `pitwall snapshot` once to initialize the first state artifact, or let the
timer do it. Runtime data is stored under
`${XDG_DATA_HOME:-~/.local/share}/pitwall/` and includes the SQLite continuity
database and `state.json`; the installer never deletes that data. To remove
application files while preserving history:

```bash
./packaging/install.sh --uninstall
```

Uninstall disables the Pitwall user timer, removes the binary, plugin, and
units, and leaves user data intact. If the plugin directory was not installed
by Pitwall, it is left untouched. The clean-room smoke test is
`packaging/test-install.sh` after `cargo build --release`.

For a quick installation check after setup:

```bash
pitwall doctor
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
