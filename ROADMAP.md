# Pitwall Roadmap

Milestones are sequential and independently testable. Do not start M(n+1)
until M(n) is stable and committed.

## Built and verified

- [x] **M0 — Repository foundation**: Cargo scaffold, MIT license,
  docs, ADRs, CI skeleton, `.pitwall/STATE.md`, packaging stub.
- [x] **M1 — Workspace/process discovery**: `/proc` + `hyprctl clients -j`
  collector; `pitwall status --json` lists real terminals/sessions/processes.
- [x] **M2 — Local state**: SQLite continuity cache (`meta`, `observations`,
  `sessions`; hash-gated writes; newest-100 pruning), `state.json` artifact,
  `pitwall snapshot`, systemd unit + timer (shipped, not enabled by default).
- [x] **M3 — Omarchy panel prototype**: `dev.pitwall` bar-widget (indicator +
  KeyboardPanel popup) reading `state.json`; Focus via native activate;
  dev-installed, screenshot-verified.
- [x] **M4 — Checkpoints + Resume**: severity-ordered session state,
  `checkpoints` table (manual + disappearance triggers, bounded retention),
  `pitwall checkpoint` / `pitwall resume` (focus-or-terminal, never agents),
  state.json v2 `resumable`, panel RESUME section. Safety Levels 1–2 only.
- [x] **M5 — Agent-aware workspace UI (FROZEN)**: evidence/role
  classification, agent discovery (`pitwall agents`/`models`), ephemeral
  AI context, agent-delegated summaries with local cache, state.json v3,
  pixel-flag identity, session-timeline rail with pin-stable interaction,
  settings (agent/model/toggle) via `pitwall config`. No provider keys,
  no cloud, no autonomous behavior. See `CHANGELOG.md` (M5a–M5f).

## M5 is feature-frozen

Remaining M5 work is hardening only: correctness, reliability, privacy,
performance, Omarchy compatibility, installation, edge cases,
documentation. No visual redesigns, no new product surfaces.

## Next: M5 hardening (M5g)

Correctness of detection/identity/grouping, Resume and process-control
safety, summary quality, panel UX validation, performance, docs,
clean-room install. No new features unless a genuine defect requires one.

## Direction: Pitwall AI Workforce Design

Pitwall can help developers structure projects where humans work
alongside multiple AI agents: a human coordinator, specialized AI
workers, Git/project artifacts as shared truth, and Pitwall workspace
awareness for human visibility and control. Agents keep their own
inference, provider, auth, model, and execution — Pitwall observes and
coordinates context. See `docs/AI_WORKFORCE.md`.

## Direction: Pitwall Recipes

Reusable open-source development workflows (roles, handoffs, memory,
coordination rules, approval boundaries) as small Git-friendly documents
the community can fork and share. No marketplace, no accounts, no cloud.
See `docs/RECIPES.md` and `CONTRIBUTING.md`.

## Contribution areas

Pitwall Recipes, AI workforce patterns, Omarchy integration,
terminal/session detection, agent detection, summary quality,
accessibility, performance, privacy/security, documentation, testing.
Bring your own workflow: build it, test it, share the recipe.

## Roadmap governance

**No new feature is added merely because an agent proposes it.**
A proposal must answer: does it improve workspace awareness? Does it
help the human understand AI-assisted work? Does it preserve
local-first/privacy? Does it fit Omarchy? Does it justify its
complexity? Otherwise it is deferred. This rule binds human and AI
contributors alike — especially important with multiple AI agents
working on Pitwall.

## Explicitly out of scope for v0.x

macOS/Windows ports, cloud backend, Supabase, mobile apps, full MCP server,
secret vault/rotation, billing, plugin marketplace, autonomous actions.
These are architected-for (see ADRs), not built.

## Later

- **M6 — Packaging + first public release**: `install.sh`, docs pass,
  clean-room install verified, v0.1.0 tag.
